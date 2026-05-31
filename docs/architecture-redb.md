# Architecture du backend de métriques — `metrics-store` (redb)

> Document de référence d'architecture du backend TSDB de Daly-BMS-Rust.
> Source de vérité : le code réel des crates `metrics-store` et
> `daly-bms-server`. Tout écart entre ce document et le code est un bug du
> document — corriger ici et committer (CLAUDE.md §9.9).
>
> Contexte de conception : `docs/plan_migration_vm_redb.md`.

---

## Table des matières

1. [Vue d'ensemble + flux complet](#1-vue-densemble--flux-complet)
2. [Schéma redb détaillé](#2-schéma-redb-détaillé)
3. [Chemin d'écriture (write path)](#3-chemin-décriture-write-path)
4. [Chemin de lecture (read path)](#4-chemin-de-lecture-read-path)
5. [PromQL supporté / non supporté](#5-promql-supporté--non-supporté)
6. [Tiering & rétention](#6-tiering--rétention)
7. [Endpoints HTTP](#7-endpoints-http)
8. [Référence de configuration](#8-référence-de-configuration)
9. [Commandes d'exploitation (ops)](#9-commandes-dexploitation-ops)
10. [Note historique : remplacement de VictoriaMetrics](#10-note-historique--remplacement-de-victoriametrics)

---

## 1. Vue d'ensemble + flux complet

`metrics-store` est une base de séries temporelles (TSDB) **pure-Rust**,
construite sur [`redb`](https://crates.io/crates/redb) (B-tree embarqué,
mono-fichier, MVCC). Elle est l'**unique** backend de métriques du système
depuis le retrait de VictoriaMetrics (cf. §10). Toutes les valeurs mesurées
(RS485, D-Bus/MQTT Victron, energy-manager) y sont écrites, et toutes les
lectures (Grafana, dashboard custom `/dashboard/history`) passent par un
**shim PromQL** qui évalue les requêtes directement sur le `Reader` redb.

La crate expose trois objets principaux (`crates/metrics-store/src/lib.rs`) :

- `MetricsStore` : ouvre le fichier, crée les tables, démarre le thread
  writer, fournit `writer()` / `reader()` / `spawn_maintenance()`.
- `Writer` : handle clonable côté producteur (envoie des `Sample` dans un
  canal mpsc vers le thread writer).
- `Reader` : snapshots MVCC lock-free (lectures concurrentes sans bloquer
  l'écriture).

### Flux complet

```
┌─────────────────────────────────────────────────────────────────────────┐
│  PRODUCTEURS (daly-bms-server)                                            │
│                                                                           │
│  RS485 /dev/ttyUSB0 ──► on_*_snapshot()                                   │
│   (2 BMS, 3 ET112, PRALRAN)         │                                     │
│                                     ▼                                     │
│  MQTT (cache D-Bus Victron) ──► redb_writes::write_*()                    │
│   (mppt, smartshunt, inverter,      │   + RateLimiter (5/30/60 s)         │
│    heatpumps, temperatures, ats)    │                                     │
│                                     ▼                                     │
│  energy-manager ──MQTT──► snapshots │  Writer::try_write(Sample)  [non    │
│   (em_*, wh_*, solar_*)             │                            bloquant]│
│                                     ▼                                     │
│  energy-manager ──HTTP POST────────►│  /api/v1/import/prometheus          │
│   /api/v1/import/prometheus         │  → Writer::try_write()              │
└─────────────────────────────────────┼─────────────────────────────────────┘
                                       │  tokio::sync::mpsc (queue_depth)
                                       ▼
                        ┌──────────────────────────────┐
                        │  Thread writer dédié          │
                        │  (writer.rs::run)             │
                        │  - drain par batch            │
                        │    (batch_max / flush_ms)     │
                        │  - LRU cache series_id        │
                        │  - 1 write-tx + commit (fsync)│
                        └───────────────┬───────────────┘
                                        ▼
                        ┌──────────────────────────────┐
                        │  Fichier redb                 │
                        │  /mnt/nvme/daly-bms/          │
                        │       metrics.redb            │
                        │  tables: series_by_key,       │
                        │   series_meta, meta,          │
                        │   metrics_raw, metrics_hourly,│
                        │   metrics_daily               │
                        └───────────────┬───────────────┘
        spawn_maintenance (tokio)       │       Reader (MVCC snapshot)
        raw→hourly→daily ◄──────────────┤
                                        ▼
                        ┌──────────────────────────────┐
                        │  Shim PromQL                  │
                        │  parse → validate (whitelist) │
                        │  → Evaluator (sélection tier) │
                        └───────────────┬───────────────┘
                                        ▼
                        ┌──────────────────────────────┐
                        │  HTTP :8080 (format Prom JSON)│
                        │  /api/v1/query[_range],       │
                        │  /api/v1/labels, /series,     │
                        │  /-/healthy, /api/v1/redb/*   │
                        └───────────────┬───────────────┘
                                        ▼
                        Grafana datasource (Prometheus-compat)
                        + dashboard custom /dashboard/history
```

Points clés :

- **Découplage producteur/writer** : les producteurs n'attendent jamais le
  disque. Ils déposent les `Sample` dans un canal mpsc et le thread writer
  les commit par lots.
- **Le writer est un `std::thread` dédié**, pas une task tokio : les appels
  `begin_write` / `commit` redb sont bloquants (fsync), et la crate ne doit
  pas imposer de runtime particulier (l'outil `import-vm` ne tourne pas sous
  tokio).
- **Les lectures sont déportées sur `tokio::task::spawn_blocking`** côté
  serveur (`state.rs`), car le scan B-tree mmap + l'évaluation PromQL peuvent
  être coûteux et ne doivent pas monopoliser un worker async.

---

## 2. Schéma redb détaillé

Défini dans `crates/metrics-store/src/tables.rs`. Les six tables sont créées
au premier `MetricsStore::open` (idempotent).

| Constante Rust          | Nom redb         | Type clé → valeur | Rôle |
|-------------------------|------------------|-------------------|------|
| `TABLE_SERIES_BY_KEY`   | `series_by_key`  | `&[u8] → u32`     | Index inverse `(metric + 0x00 + labels_json)` → `series_id`. Garantit l'unicité d'une série. |
| `TABLE_SERIES_META`     | `series_meta`    | `u32 → &[u8]`     | Métadonnées par série (`SeriesMeta` sérialisée en bincode). Catalogue. |
| `TABLE_META`            | `meta`           | `&str → u64`      | Compteur monotone, clé unique `"next_series_id"`. |
| `TABLE_RAW`             | `metrics_raw`    | `&[u8] → f64`     | Points bruts : clé = `enc_skey(series_id, ts_ms)`, valeur = la mesure. |
| `TABLE_HOURLY`          | `metrics_hourly` | `&[u8] → &[u8]`   | Buckets horaires compactés : clé = `enc_skey(series_id, bucket_ms)`, valeur = `AggBucket` bincode. |
| `TABLE_DAILY`           | `metrics_daily`  | `&[u8] → &[u8]`   | Buckets journaliers compactés (même format que hourly). |

### Encodage des clés composites (`encoding.rs`)

Les tables de points (`metrics_raw`, `metrics_hourly`, `metrics_daily`)
utilisent une clé composite `[u8; 12]` big-endian, `SKEY_LEN = 12` :

```
octets:   0  1  2  3 | 4  5  6  7  8  9 10 11
         └─ series_id ─┘ └────── ts_ms ──────┘
          u32 BE          (u64) ^ 0x8000_0000_0000_0000  BE
```

```rust
pub fn enc_skey(series_id: u32, ts_ms: i64) -> [u8; SKEY_LEN] {
    let mut k = [0u8; SKEY_LEN];
    k[0..4].copy_from_slice(&series_id.to_be_bytes());
    k[4..12].copy_from_slice(&((ts_ms as u64) ^ TS_SIGN_FLIP).to_be_bytes());
    k
}
```

Propriétés (vérifiées par les tests `encoding.rs`) :

- **`series_id` domine l'ordre** : tous les points d'une série sont
  contigus dans le B-tree (`enc_skey(1, i64::MAX) < enc_skey(2, i64::MIN)`).
  Un range scan sur une série est donc une plage contiguë.
- **Le `XOR 0x8000_0000_0000_0000` (`TS_SIGN_FLIP`)** mappe les `i64` signés
  vers un ordre `u64` non signé monotone : l'ordre lexicographique des octets
  correspond à l'ordre chronologique, y compris pour les timestamps négatifs.

### Clé de l'index inverse (`make_lookup_key`)

```rust
// metric + octet 0x00 + labels_json
pub fn make_lookup_key(metric: &str, labels_json: &str) -> Vec<u8>
```

Le `labels_json` est la **sérialisation canonique** des labels
(`labels.rs::canonical_json`) : un `BTreeMap` trié par nom de label, format
`{"<label1>":"<val1>",…}`. Deux séries identiques aux labels près de l'ordre
d'insertion produisent donc la **même** clé (pas de doublon). Exemple :
`{"bms_id":"1","kind":"soc"}` quel que soit l'ordre d'arrivée.

### Structures sérialisées (bincode)

```rust
pub struct SeriesMeta {
    pub metric: String,        // ex: "bms_voltage"
    pub labels_json: String,   // ex: r#"{"bms_id":"0x01"}"#
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
}

pub struct AggBucket {
    pub avg: f64,
    pub min: f64,
    pub max: f64,
    pub sum: f64,
    pub first: f64,  // première valeur de la fenêtre (pour increase())
    pub last: f64,   // dernière valeur de la fenêtre (idem)
    pub cnt: u32,
}

pub enum Tier { Raw, Hourly, Daily }
```

`first` / `last` sont indispensables : après compaction, les points raw sont
purgés, et `increase()` / `rate()` doivent pouvoir télescoper sur la valeur
de bord de chaque bucket.

---

## 3. Chemin d'écriture (write path)

### 3.1 Rate limiting (`daly-bms-server/src/redb_writes.rs`)

Chaque snapshot producteur appelle un `write_*()` qui pousse les samples via
un helper `push()` gardé par un `RateLimiter` (`Arc<Mutex<HashMap<String,
Instant>>>`). La clé est `(metric + labels)` formatée en chaîne (ex.
`bms_voltage:0x01`, `et112_power_w:0x09`, `pi5_load_avg:5m`). Une même
métrique avec des labels différents est rate-limitée indépendamment.

Trois intervalles minimaux par type de métrique :

| Constante               | Valeur | Métriques concernées |
|-------------------------|--------|----------------------|
| `MIN_WRITE_INTERVAL`    | 5 s    | grandeurs « rapides » : tension, courant, puissance, SoC, états, flags, alarmes, cellules… |
| `ENERGY_WRITE_INTERVAL` | 30 s   | compteurs d'énergie / capacités / cycles (varient lentement) : `*_energy_*`, `*_capacity_*`, `bms_charge_cycles`, `ats_cnt*`, totaux mémoire… |
| `TEMP_WRITE_INTERVAL`   | 60 s   | températures / humidité / pression / RSSI (varient très lentement). |

Ordre de grandeur (commentaire du module) : ~50 métriques actives × 1 write /
5 s ≈ 10 writes/s ≈ 36 000 writes/h ; le writer batche par 500 ⇒ ~72 commits
redb/h.

> **Exception** : l'endpoint `/api/v1/import/prometheus`
> (`api/prometheus_import.rs`) **n'applique PAS** le rate limiter —
> l'émetteur (energy-manager) gère lui-même sa cadence (60 s monitoring
> système, à la demande pour les règles). Appliquer un rate limiter ici
> écraserait des données légitimes.

Les producteurs couverts (un `write_*` par famille) : `write_bms`,
`write_et112`, `write_irradiance`, `write_venus_mppt`,
`write_venus_smartshunt`, `write_venus_inverter`, `write_venus_temperature`,
`write_venus_heatpump`, `write_ats`, `write_tasmota`, `write_shelly`,
`write_solar_total` / `write_solar_components` / `write_solar_yield`,
`write_monitor`, `write_em_metrics`, `write_wh_metrics`.

Les états textuels Victron (états MPPT, état/mode onduleur, état SmartShunt,
source ATS) sont **encodés en codes numériques** avant écriture (ex.
`mppt_state_to_code`, `inverter_state_to_code`, `shunt_state_to_code`).

### 3.2 Politique non bloquante : `try_write`

`push()` appelle `writer.try_write(sample)` (non bloquant). Si le canal mpsc
du writer est plein (`queue_depth`, défaut 10 000), le sample est
**silencieusement abandonné**. Cela protège les chemins critiques (poll
RS485, callbacks MQTT) contre toute back-pressure du disque.

```rust
fn push(writer: &Writer, rl: &RateLimiter, interval: Duration, sample: Sample, key: &str) {
    if rl.allow(key, interval) {
        let _ = writer.try_write(sample); // best-effort
    }
}
```

`Writer` expose aussi `write()` (async, attend si plein) et `blocking_write()`
(sync, utilisé par `import-vm`).

### 3.3 Batching du writer (`writer.rs`)

Le thread writer boucle sur `drain()` puis `commit_batch()` :

1. **`drain()`** : bloque sur `blocking_recv()` pour le premier message
   (sommeil efficace, pas de busy-poll), puis remplit le batch jusqu'à
   `batch_max` éléments **ou** la deadline `flush_ms`. Entre deux `try_recv`
   vides, il dort `poll_idle_ms` (borné à la deadline restante). Les
   sentinelles `Shutdown` ou la fermeture du canal arrêtent la boucle (le
   batch déjà collecté est tout de même commité avant l'arrêt).

2. **`commit_batch()`** : ouvre **une seule** write-tx redb pour tout le
   batch, résout chaque `series_id` (cache LRU `SeriesCache` de
   `SERIES_CACHE_CAPACITY = 50 000` entrées, keyé par fingerprint `u64`),
   insère les points dans `metrics_raw`, et `commit()` (le commit déclenche
   le fsync). Si le commit échoue, `next_id` est restauré et le cache est
   purgé (correctif R1, cohérence cache↔base).

`WriterConfig` (défauts dans `writer.rs`) :

| Champ          | Défaut | Rôle |
|----------------|--------|------|
| `batch_max`    | 500    | Taille max d'un batch avant commit forcé. |
| `flush_ms`     | 250    | Fenêtre de flush (≈ 4 fsync/s en régime nominal). |
| `poll_idle_ms` | 5      | Pause entre deux `try_recv` quand la file est vide. |

> **Fréquence de fsync** : un `commit()` ≈ un fsync. En régime nominal, le
> writer commite au plus toutes les `flush_ms` (250 ms) ⇒ **≈ 4 fsync/s**, ou
> plus tôt si `batch_max` est atteint. `import-vm` pousse ces valeurs
> beaucoup plus haut (batch 50 000, flush 250 ms) pour amortir le fsync sur
> de gros imports.

Le serveur ouvre le store avec `WriterConfig::default()` (cf.
`main.rs` : `writer: metrics_store::WriterConfig::default()`).

### 3.4 Résolution des `series_id`

- Cache-hit (cas nominal >99 %) : fingerprint `u64` calculé sans allocation,
  vérification d'identité `(metric, labels)` anti-collision, retourne le
  `series_id`.
- Cache-miss : calcul du `canonical_json`, lookup dans `series_by_key`. Si
  absent, allocation d'un nouvel `id = next_id` (incrément persisté dans
  `meta["next_series_id"]`), création de la `SeriesMeta`. Le compteur est
  monotone et **persiste à travers les redémarrages** (jamais de réutilisation
  d'id, testé par `writer_persists_next_series_id_across_reopen`).

---

## 4. Chemin de lecture (read path)

### 4.1 Reader MVCC (`reader.rs`)

`Reader { db: Arc<Database> }` ouvre des `ReadTransaction` redb. redb étant
MVCC, les lectures sont **lock-free** et ne bloquent pas le writer (snapshot
cohérent au moment du `begin_read`). Méthodes principales :

| Méthode | Usage |
|---------|-------|
| `query_range_raw` (+ `_with_tx`) | Scan `[from, to]` sur `metrics_raw`, points triés par ts. |
| `last_point_in_range_with_tx`    | Dernier point de la fenêtre (scan inverse O(1)) — instant selector. |
| `first_last_in_range_with_tx`    | Premier + dernier point — `delta` / `last_over_time`. |
| `last_two_in_range_with_tx`      | Deux derniers points — `irate`. |
| `query_range_buckets` (+ `_with_tx`) | Scan sur `metrics_hourly` / `metrics_daily`, `AggBucket` désérialisés. |
| `lookup_series_id` | `(metric, labels_json)` → `series_id`. |
| `list_series` (+ `_with_tx`) | Dump complet du catalogue `series_meta`. |

Robustesse : un intervalle inversé (`from > to`) renvoie un résultat vide
(garde anti-panic redb). Un `AggBucket` ou une `SeriesMeta` corrompu est
journalisé (`tracing::warn`) et sauté, sans avorter la requête (correctif M2).

### 4.2 Shim PromQL (`promql/`)

Pipeline `parse_and_validate` (`promql/mod.rs`) :

1. **parse** : délégué à `promql_parser::parser::parse` (crate
   `promql-parser 0.9`).
2. **validate** (`promql/validate.rs`) : walk de l'AST, rejette toute
   construction hors liste blanche (cf. §5). L'erreur a la forme Prometheus
   standard (`status=error`, `errorType=bad_data`).
3. **execute** (`promql/exec.rs::Evaluator`) : évalue l'AST sur un instant
   (`eval_instant`) ou une plage (`eval_range(start, end, step)`).

`PromQlError` (`promql/error.rs`) a trois variantes — `ParseError`,
`Unsupported`, `Execution` — et expose `body()` → `{status, error,
errorType}` (`bad_data` pour parse/unsupported, `execution` pour l'exécution).

### 4.3 Sélection automatique du tier (`exec.rs::tier_for_range`)

Pour les fonctions à fenêtre `f(m[range])`, le tier interrogé dépend de la
largeur de la fenêtre :

```
range ≤ 7 j           → Tier::Raw      (metrics_raw)
7 j < range ≤ 90 j    → Tier::Hourly   (metrics_hourly)
range > 90 j          → Tier::Daily    (metrics_daily)
```

Seuils : `HOURLY_THRESHOLD_MS = 7 j`, `DAILY_THRESHOLD_MS = 90 j`.

> Les instant selectors (sans `[range]`) lisent toujours `metrics_raw` via
> `last_point_in_range_with_tx` sur une fenêtre de lookback (`lookback_ms`,
> défaut 5 min, analogue à Prometheus).

### 4.4 Optimisations de l'Evaluator

L'`Evaluator` est scopé par requête et porte trois caches partagés sur toute
la durée d'un `eval_range` (cf. `docs/memory-leak-investigation.md §12`) :

1. **`read_txn`** : une seule `ReadTransaction` redb lazy-init (évite N×
   `begin_read` + `open_table`).
2. **`series_catalog`** : `Arc<Vec<(u32, SeriesMeta)>>` chargé une fois,
   partagé entre tous les VectorSelectors.
3. **`match_cache`** : keyé par **adresse mémoire** du `VectorSelector`
   (l'AST est immutable pendant l'évaluation, donc le pointeur est stable).
   Vidé au début de chaque `eval_instant` / `eval_range` (correctif M1, évite
   un faux positif d'adresse réutilisée).

`InstantSample.labels` est un `Arc<Labels>` (BTreeMap) propagé d'un step à
l'autre sans cloner.

### 4.5 Format JSON de sortie (Prometheus)

Deux chemins produisent exactement le format Prometheus HTTP API :

- `daly-bms-server/src/state.rs::redb_query_range_inner` /
  `redb_query_instant_inner` (servent `/api/v1/query[_range]` via
  `dispatched_query_*`).
- `daly-bms-server/src/api/redb.rs::run_query_range` / `run_query_instant`
  (servent les routes `/api/v1/redb/*`).

Range (`matrix`) :

```json
{
  "status": "success",
  "data": {
    "resultType": "matrix",
    "result": [
      { "metric": {"__name__":"bms_voltage","bms_id":"0x01"},
        "values": [[1700000000.0, "53.9"], [1700000060.0, "53.8"]] }
    ]
  }
}
```

Instant (`vector`) :

```json
{
  "status": "success",
  "data": {
    "resultType": "vector",
    "result": [
      { "metric": {"__name__":"bms_voltage","bms_id":"0x01"},
        "value": [1700000000.0, "53.9"] }
    ]
  }
}
```

Conventions de formatage :

- **Timestamps en secondes** (float), pas en ms : `ts_ms as f64 / 1000.0`.
- **Valeurs en chaînes**. `fmt_val` (state.rs) cap à 6 décimales et trime les
  zéros terminaux (`53.900001525878906` → `"53.900002"`, `0.0` → `"0"`),
  pour éviter le bruit décimal des conversions f32→f64. Les non-finis
  donnent `"NaN"`, `"+Inf"`, `"-Inf"`. (`api/redb.rs::val_str` fait
  l'équivalent sans le cap décimal — `v.to_string()`.)
- **`__name__`** est réinséré dans les labels de sortie pour la compatibilité
  Grafana (`match_series` ajoute `with_name`).

Les deserializers d'entrée (`api/redb.rs`) sont tolérants : `time`/`start`/
`end` acceptent secondes float, millisecondes int (≥ 1e12) ou RFC3339 ;
`step` accepte en plus les durations Prometheus (`30s`, `5m`, `1h`, `1d`,
`1w`, `…ms`).

---

## 5. PromQL supporté / non supporté

La liste blanche est définie dans `promql/validate.rs` (constantes) et
correspond au golden set `plan §6.5` étendu par
`docs/promql-compat-roadmap.md`. **Tout ce qui n'est pas listé ci-dessous
est rejeté** avec `errorType=bad_data`.

### 5.1 SUPPORTÉ

**Sélecteurs** : `VectorSelector` instantané avec matchers `=`, `!=`, `=~`,
`!~`. `MatrixSelector` (`m[range]`) uniquement comme argument d'une fonction
à fenêtre. Le modificateur **`offset`** (`m offset 5m`, `m[w] offset 1h`,
y compris négatif) est supporté : il décale l'instant d'évaluation à
`t − offset` (cf. `exec.rs::offset_ms`).

**Opérateurs arithmétiques** (`SUPPORTED_BINOPS`) : `+`, `-`, `*`, `/`
(vec×scalaire, scalaire×vec, vec×vec aligné sur tous les labels hors
`__name__`).

**Opérateurs de comparaison** (`SUPPORTED_CMP_OPS`) : `==`, `!=`, `>`, `<`,
`>=`, `<=`. Sans `bool` = filtre (valeur conservée) ; avec `bool` = 1.0/0.0.
Toute comparaison impliquant un `NaN` est fausse.

**Agrégateurs simples** (`SUPPORTED_AGGREGATORS`) : `sum`, `max`, `min`,
`avg`, `count` — avec ou sans `by (...)` / `without (...)`. Droppent
`__name__`.

**Agrégateurs paramétrés** (`PARAMETERIZED_AGGREGATORS`) : `topk`, `bottomk`
(exigent le paramètre `k`). **Conservent** les labels d'origine, y compris
`__name__`.

**Fonctions à fenêtre** (`SUPPORTED_RANGE_FUNCS`, 18) :
`increase`, `rate`, `irate`, `delta`, `deriv`, `predict_linear`, `changes`,
`resets`, `avg_over_time`, `sum_over_time`, `min_over_time`, `max_over_time`,
`count_over_time`, `last_over_time`, `stddev_over_time`, `stdvar_over_time`,
`quantile_over_time`, `absent_over_time`.

**Fonctions instantanées** (`SUPPORTED_INSTANT_FUNCS`) :
`abs`, `clamp_min`, `clamp_max`, `clamp`, `ceil`, `floor`, `round`
(2e arg `to_nearest` honoré), `sqrt`, `exp`, `ln`, `log2`, `log10`, `sgn`,
`absent`.

**Fonctions de labels** (`SUPPORTED_LABEL_FUNCS`) : `label_replace`,
`label_join` (seules fonctions à accepter des arguments string literals).

**Littéraux** : numériques (`NumberLiteral`), parenthèses, unaire `-`.

### 5.2 NON SUPPORTÉ (rejeté à la validation)

| Construction | Raison / message |
|--------------|------------------|
| **String literal nu** (hors arg de `label_*`) | `string literal` |
| **Subquery** `expr[Xh:Ym]` | `subquery …` — réécrire en deux requêtes côté client |
| **`@`** (modifier @-timestamp) | `@ modifier` |
| **Set ops** `and`, `or`, `unless` | `binary operator: and/or/unless` |
| **Vector matching** `on(...)`, `ignoring(...)`, `group_left`, `group_right` | `vector matching … non supporté` (seul l'alignement exact `OneToOne` tous-labels est géré) |
| **`bool` sur opérateur arithmétique** | `bool modifier` (garde défensive ; le parser le rejette en général avant) |
| **Agrégateurs hors liste** : `quantile`, `count_values`, `stddev`, `stdvar`, `group`, `topk`/`bottomk` sans `k`… | `aggregator: <op>` / `aggregator <op> requires a parameter` |
| **Fonctions hors liste** : `histogram_quantile`, `vector`, `idelta`, `histogram_*`, `time`, `timestamp`, `holt_winters`… | `function: <name>` |
| **`Extension`** (extensions du parser) | `extension expression` |

> Limite résiduelle documentée (pas un rejet) : sur tier compacté
> (hourly/daily), `deriv` / `predict_linear` / `stddev_over_time` /
> `stdvar_over_time` / `quantile_over_time` opèrent sur les valeurs `avg` des
> buckets (approximation) ; `changes` / `resets` sur la séquence
> `first,last` de chaque bucket ; un reset interne à un bucket reste invisible
> (points raw purgés).

---

## 6. Tiering & rétention

Implémenté dans `crates/metrics-store/src/tiering.rs`. Objectif : maîtriser
la taille de `metrics_raw` en agrégeant les vieux points en buckets, puis en
agrégeant ces buckets.

### 6.1 Compaction

Constantes : `HOURLY_MS = 3 600 000`, `DAILY_MS = 86 400 000`.

- **`compact_raw_to_hourly(db, cutoff_ms)`** : agrège tous les points raw
  `ts < cutoff_ms` par bucket horaire (`bucket_floor`, division Euclidienne)
  via `AggBucketBuilder::accumulate`, écrit/fusionne dans `metrics_hourly`,
  puis purge les raws compactés.
- **`compact_hourly_to_daily(db, cutoff_ms)`** : agrège les `AggBucket`
  horaires `ts < cutoff_ms` par bucket journalier via
  `AggBucketBuilder::merge_bucket`, écrit/fusionne dans `metrics_daily`, purge
  les hourlies compactés.

Garanties (correctifs des revues) :

- **R3** : la purge est bornée au `max_ts_read` réellement lu, pas au
  `cutoff` théorique — un point inséré tardivement dans `]max_ts_read,
  cutoff[` survit pour la passe suivante au lieu d'être purgé sans agrégation.
- **R4** : si un bucket existe déjà à la clé cible (compaction partielle
  antérieure), il est **fusionné** (`merge_bucket`), pas écrasé. Idempotent :
  une seconde passe identique n'écrit rien.
- **P3** : la purge est découpée en lots de `PURGE_CHUNK = 10 000` clés, avec
  un commit par lot, pour relâcher le write-lock entre lots (redb est
  mono-writer) et laisser le thread writer intercaler son ingestion.

Phases séparées : lecture/agrégation en mémoire (read-tx), écriture des
buckets (1 write-tx), puis purge par lots (write-tx courtes).

### 6.2 Tâche de maintenance (`spawn_maintenance`)

`tokio::spawn` qui :

1. attend **60 s** après le démarrage (correctif M4 : compacte tôt même si le
   service redémarre plus souvent que `interval_hours`) ;
2. en boucle, calcule `cutoff_raw = now - raw_retention_days` et
   `cutoff_hourly = now - hourly_retention_days`, exécute les deux
   compactions via `tokio::task::spawn_blocking` (journalise buckets écrits /
   points purgés), puis dort `interval_hours` heures (min 1 h).

Démarrée par `daly-bms-server/src/main.rs` au boot si
`maintenance_interval_hours > 0`, avec la `TierPolicy` issue de la config.

`MetricsStore::compact_now()` lance une passe unique synchrone (tests /
outillage).

### 6.3 Rétentions par défaut

`TierPolicy::default()` (lib.rs) **et** valeurs de production (`Config.toml`)
sont alignées :

| Champ                   | Défaut (`TierPolicy`) | Config.toml prod | Rôle |
|-------------------------|-----------------------|------------------|------|
| `raw_retention_days`    | 30                    | 30               | Au-delà, raw → hourly. |
| `hourly_retention_days` | 365                   | 365              | Au-delà, hourly → daily. |
| `daily_retention_days`  | 5 × 365 = 1825        | 1825 (5 ans)     | Horizon des buckets journaliers. |

> Note : `daily_retention_days` est exposé dans la config mais la boucle
> `spawn_maintenance` n'applique de cutoff que sur raw et hourly ; la purge
> du tier daily n'est pas réalisée par la maintenance courante (il n'y a pas
> d'étage suivant). Voir `tiering.rs` pour le détail exact.

---

## 7. Endpoints HTTP

Routage dans `crates/daly-bms-server/src/api/mod.rs`. Tous les endpoints de
requête acceptent **GET et POST** (Grafana envoie `httpMethod: POST` par
défaut, paramètres en `application/x-www-form-urlencoded`).

| Route | Méthode | Handler | Description |
|-------|---------|---------|-------------|
| `/api/v1/query` | GET/POST | `promql::query_instant[_post]` | Requête instantanée (vector). Délègue à `state.dispatched_query_instant` (spawn_blocking). |
| `/api/v1/query_range` | GET/POST | `promql::query_range[_post]` | Requête sur plage (matrix). Délègue à `state.dispatched_query_range`. Valide `start ≤ end` et `step > 0`. |
| `/api/v1/labels` | GET | `promql::list_metrics` | Noms de labels distincts (scan `series_meta`). |
| `/api/v1/series` | GET | `redb::list_series` | Catalogue de séries (labels + `__name__`). |
| `/api/v1/label/{name}/values` | GET | `redb::label_values` | Valeurs distinctes d'un label (`__name__` → liste des métriques). |
| `/-/healthy` | GET | `redb::healthy` | Health-check datasource Grafana : 200 si store présent, 503 sinon. |
| `/api/v1/import/prometheus` | POST | `prometheus_import::import_prometheus` | Ingestion format texte Prometheus exposition → `Writer::try_write`. Pas de rate limiter. Renvoie 204. |
| `/api/v1/redb/query` | GET/POST | `redb::query_instant[_post]` | Variante explicite redb (debug / parité). |
| `/api/v1/redb/query_range` | GET/POST | `redb::query_range[_post]` | Idem range. |
| `/api/v1/redb/series` | GET | `redb::list_series` | Catalogue (alias). |
| `/api/v1/redb/labels` | GET | `redb::list_labels` | Noms de labels distincts. |
| `/api/v1/redb/label/{name}/values` | GET | `redb::label_values` | Valeurs d'un label. |
| `/api/v1/redb/healthy` | GET | `redb::healthy` | Health-check (alias). |

Tous renvoient le format Prometheus (`status=success` + `data`) ou une
erreur `status=error` + `errorType`. Si `metrics_store` est `None`
(désactivé) : 503 « metrics-store backend not enabled / not configured ».

### Format d'ingestion `/api/v1/import/prometheus`

Format texte Prometheus exposition (`api/prometheus_import.rs`) :

```text
# commentaire ignoré
metric_name value [timestamp_ms]
metric_name{label1="val1",label2="val2"} value [timestamp_ms]
```

- Timestamp en **millisecondes** Unix (absent → maintenant).
- Valeurs non finies (NaN/Inf) ignorées.
- Découpage robuste via `rfind('}')` (tolère `}` dans une valeur de label).
- Remplace l'ancien endpoint VM homonyme utilisé par energy-manager
  (`monitoring.rs`, `lg_thinq.rs`, `rule_metrics.rs`, `water_heater/`).

> La crate `metrics-store` fournit aussi un parser exposition complet et
> tolérant (`prom_text.rs`, avec gestion des échappements `\"` `\\` `\n`,
> stats par ligne, et collecte des erreurs), distinct du parser inline léger
> de l'endpoint serveur.

---

## 8. Référence de configuration

Section `[metrics_store]` de `Config.toml` →
`crates/daly-bms-server/src/config.rs::MetricsStoreConfig`. Le service lit
`/etc/daly-bms/config.toml` (pas le dépôt — cf. CLAUDE.md §4).

| Champ | Type | Défaut (`config.rs`) | Valeur prod (`Config.toml`) | Rôle |
|-------|------|----------------------|-----------------------------|------|
| `enabled` | bool | `false` | `true` | Active l'ouverture du store. **Doit être `true` en prod** sinon plus aucune lecture/écriture TSDB. |
| `db_path` | String | `/mnt/nvme/daly-bms/metrics.redb` | idem | Chemin du fichier redb (créé s'il manque). |
| `cache_mb` | usize | `64` | `16` | Taille du cache de pages redb en MiB (→ `Options.cache_bytes = cache_mb × 1024²`). |
| `queue_depth` | usize | `10 000` | `10000` | Profondeur du canal mpsc producteurs→writer (`Options.writer_queue_depth`). |
| `maintenance_interval_hours` | u64 | `6` | `6` | Période de la passe de compaction (0 = désactivé). |
| `raw_retention_days` | u32 | `30` | `30` | Rétention raw. |
| `hourly_retention_days` | u32 | `365` | `365` | Rétention hourly. |
| `daily_retention_days` | u32 | `1825` (5×365) | `1825` | Rétention daily. |
| `default_backend` | String | `"redb"` | (absent) | **Obsolète / ignoré** depuis le retrait de VM. Conservé pour compat ascendante des vieux `Config.toml`. |

Au démarrage (`main.rs`), si `enabled` :
`MetricsStore::open(db_path, Options { cache_bytes, writer_queue_depth, writer:
WriterConfig::default() })`, puis `spawn_maintenance(TierPolicy{…}, interval)`
si `maintenance_interval_hours > 0`. En cas d'échec d'ouverture : un `warn` et
`metrics_store = None` (mode dégradé : les endpoints renvoient 503).

> `Options` (lib.rs) a aussi un champ `writer: WriterConfig` non exposé en
> TOML : le serveur utilise toujours `WriterConfig::default()` (batch 500 /
> flush 250 ms / poll 5 ms).

---

## 9. Commandes d'exploitation (ops)

Toutes sur le Pi5 (`pi5compute`), port 8080. Cf. CLAUDE.md §0.

| Objectif | Commande |
|----------|----------|
| Taille de la base | `du -sh /mnt/nvme/daly-bms/metrics.redb` |
| Nombre de séries en base | `curl -s http://localhost:8080/api/v1/redb/series \| jq '.data \| length'` |
| Lister les noms de labels | `curl -s http://localhost:8080/api/v1/labels \| jq .data` |
| Valeurs d'un label (ex. métriques) | `curl -s http://localhost:8080/api/v1/label/__name__/values \| jq .data` |
| Healthcheck datasource | `curl -s http://localhost:8080/-/healthy` |
| Requête instantanée | `curl -s 'http://localhost:8080/api/v1/query?query=bms_voltage' \| jq .` |
| Requête plage | `curl -s 'http://localhost:8080/api/v1/query_range?query=bms_voltage&start=...&end=...&step=60s' \| jq .` |
| Logs backend (inclut maintenance) | `journalctl -u daly-bms -f` |

Logs de maintenance attendus (info) : `compact raw→hourly OK buckets=… purged=…`
et `compact hourly→daily OK …`.

---

## 10. Note historique : remplacement de VictoriaMetrics

Le système utilisait auparavant **VictoriaMetrics** (`victoriametrics.service`,
binaire `victoria-metrics-prod`, rétention 5 ans). La migration vers
`metrics-store`/redb (cf. `docs/plan_migration_vm_redb.md`) a été motivée par :

- **Gain mémoire** : ~**135 Mo de RSS** récupérés (VM mesuré à 135 Mo sur
  Pi5, mai 2026).
- **Suppression de la dépendance C** : VM impliquait une dépendance native
  lourde à cross-compiler pour aarch64 (Pi5) ; `metrics-store` est
  **100 % Rust**, cross-compile aarch64 trivialement (gain CI, build
  incrémental ~2 s vs ~30 s).
- Le gain disque, lui, est devenu marginal (volume réel tractable).

L'historique VM a été rapatrié **une seule fois** via l'outil ponctuel
`crates/metrics-store/src/bin/import_vm.rs` (binaire `import-vm`) :

```bash
# daly-bms ARRÊTÉ (contention writer sur la même base redb)
curl -s 'http://127.0.0.1:8428/api/v1/export?match[]={__name__!=""}' \
  | import-vm --db /mnt/nvme/daly-bms/metrics.redb
```

Il lit le format JSONL `/api/v1/export` de VM (une série par ligne :
`{"metric":{…},"values":[…],"timestamps":[…]}`, ms natifs), sépare `__name__`
des autres labels, et pousse les samples via `blocking_write`. Idempotent
(mêmes `(series_id, ts)` réécrits, sérialisation canonique des labels). Options
notables : `--dry-run`, `--limit`, `--queue-depth` (200 000), `--batch-max`
(50 000), `--flush-ms` (250), `--cache-mb` (256). Cet outil n'a plus
d'utilité en régime courant — il est conservé à titre documentaire.

Résidu de compatibilité : le champ de config `default_backend` est conservé
mais **ignoré** ; il n'existe plus de dispatcher VM/redb (le shim PromQL redb
est le seul backend).
