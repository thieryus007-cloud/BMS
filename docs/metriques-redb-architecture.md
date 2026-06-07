# Architecture des métriques — metrics-store (redb) — Daly-BMS-Rust

> Backend TSDB **pure-Rust redb** (`/mnt/nvme/daly-bms/metrics.redb`), embarqué dans
> `daly-bms-server` et interrogé via le shim PromQL sur le port **8080**. Ce document décrit
> la conception du moteur, le schéma, les chemins d'écriture/lecture, le tiering/rétention,
> et conserve en **annexe historique** la migration complète VictoriaMetrics → redb.
>
> **Source de vérité : le code réel** des crates `metrics-store` et `daly-bms-server`.
> Tout écart entre ce document et le code est un bug du document.
>
> Fait partie de l'[architecture documentaire](./ARCHITECTURE.md). Dernière consolidation : 2026-06-07.

## Table des matières

- [1. Vue d'ensemble + flux complet](#1-vue-densemble--flux-complet)
- [2. Schéma redb détaillé](#2-schéma-redb-détaillé)
- [3. Chemin d'écriture (write path)](#3-chemin-décriture-write-path)
- [4. Chemin de lecture (read path)](#4-chemin-de-lecture-read-path)
- [5. PromQL supporté / non supporté](#5-promql-supporté--non-supporté)
- [6. Tiering & rétention](#6-tiering--rétention)
- [7. Endpoints HTTP](#7-endpoints-http)
- [8. Référence de configuration](#8-référence-de-configuration)
- [9. Commandes d'exploitation (ops)](#9-commandes-dexploitation-ops)
- [10. Note historique : remplacement de VictoriaMetrics](#10-note-historique--remplacement-de-victoriametrics)
- [Séries et métriques stockées (inventaire)](#séries-et-métriques-stockées-inventaire)
- [Annexe historique — Migration VictoriaMetrics → redb](#annexe-historique--migration-victoriametrics--redb)

## Voir aussi

- [Catalogue des métriques & référence PromQL](./metriques-promql-reference.md) — noms de métriques, labels, requêtes d'exemple, fonctions PromQL supportées.
- [Grafana & dashboards](./grafana-dashboards.md) — datasource PromQL (UID `daly-metrics`) et provisioning.
- [daly-bms-server](./app-daly-bms-server.md) — serveur qui embarque le metrics-store et expose le shim PromQL.
- [Diagnostic & dépannage](./diagnostic-depannage.md) — investigation mémoire `/dashboard/history`.
- [Déploiement & exploitation](./deploiement-exploitation.md) — rétention, reset usine, taille de base.

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
la durée d'un `eval_range` (cf. `./diagnostic-depannage.md §12`) :

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
`./metriques-promql-reference.md`. **Tout ce qui n'est pas listé ci-dessous
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
`metrics-store`/redb (cf. `./metriques-redb-architecture.md`) a été motivée par :

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

---

## Séries et métriques stockées (inventaire)

> Vue d'ensemble des appareils et du nombre de séries temporelles présentes dans le
> metrics-store. Le **catalogue détaillé** (noms exacts des métriques, labels) est dans
> [metriques-promql-reference.md](./metriques-promql-reference.md).

### Appareils et Métriques:
2 x Daly BMS 16 cellules
- Métriques par BMS :
- Tension par cellule (16)
- Delta cellule mV
- Température par BMS 
- Courant de charge/décharge
- Tension totale
- État de santé (SOC, SOH, etc.) 
Total par BMS : ~40 métriques.  
Total pour 2 BMS : \( 40 imes 3 = 80 \) séries temporelles

###  3 x ET112 Energy Management
- Métriques par ET112 :
- Puissance active (1)
- Puissance réactive (1)
- Tension (1)
- Courant (1)
- Importée
- Exportée.  
Total par ET112 : ~6 métriques.  
Total pour 3 ET112 : \( 6 imes 3 = 18 \) séries temporelles

### 1 x Capteur Irradiance
- Métriques :
- Irradiance (1).  
Total : 1 série temporelle

### 2 x Chauffe-eau & Climatisation)
- Métriques :
- State
- Current temperature
- Target temperature
- Today kWh .  
Total : 4 x 2 = 8 série temporelle

### 1 x ATS (Automatic Transfer Switch)
- Métriques :
- État (ON/OFF) (3)
- Courant (3)
- Tension (3)
- xxx.  
Total : 10 séries temporelles

### 2 x MPPT Victron
- Métriques par MPPT :
- Puissance (1)
- Tension d'entrée (1)
- Courant d'entrée (1)
- Tension de sortie (1)
- Courant de sortie (1).  
Total par MPPT : ~5 métriques.  
Total pour 2 MPPT : \( 2 imes 5 = 10 \) séries temporelles

### 1 x SmartShunt Victron
- Métriques :
- Courant (1)
- Tension (1)
- SOC (1)
- Puissance (1)
- Energie chargée
- Energie déchargée.  
Total : 6 séries temporelles

### 1 x Easysolar II GX Victron
- Métriques :
- Puissance totale (3)
- Frequency
- Tension (3)
- Courant (3)
- Température (1)
- Ingnore AC IN.  
Total : 9 séries temporelles

### 1 x Capteur Température + Humidité
- Métriques :
- Température (1)
- Humidité (1).  
Total : 2 séries temporelles

### 6 x Switchs Tasmota Tonguou
- Métriques par switch :
- État (ON/OFF) (1)
- Puissance (1)
- Tension
- Courant
- Jours Énergie totale kWh(1).  
Total par switch : ~5 métriques.  
Total pour 6 switchs : \( 5 imes 6 = 30 \) séries temporelles

### 1 x Switch Shelly Pro 2 PM
- Métriques :
- État (ON/OFF) (2)
- Puissance (2)
- Énergie totale (2).  
Total : 6 séries temporelles

### Total des Séries Temporelles
| Appareil | Séries Temporelles |
|------------------------------|--------------------|
| 2 x Daly BMS 16 cellules | 80 |
| 3 x ET112 Energy Management | 12 |
| 1 x Capteur Irradiance | 1 |
| 2 x Chauffe-eau & Cmlimatisation | 8 |
| 1 x ATS | 10 |
| 2 x MPPT Victron | 10 |
| 1 x SmartShunt Victron | 6 |
| 1 x Easysolar II GX Victron | 9 |
| 1 x Capteur Température/Humidité | 2 |
| 6 x Switchs Tasmota Tonguou | 30 |
| 1 x Switch Shelly Pro 2 PM | 6|
| Total | ~200 |

Conclusion:.  
Nombre mini/maximal de séries temporelles : .  
## ~200/240 (en incluant des métriques supplémentaires ou des labels supplémentaires dans le metrics-store redb).

---

## Annexe historique — Migration VictoriaMetrics → redb

> Statut : ✅ **MIGRATION TERMINÉE — DOCUMENT HISTORIQUE.** VictoriaMetrics a été
> entièrement retiré (code, double écriture, service) ; **redb (`metrics-store`) est
> désormais la seule TSDB.** Cette annexe est conservée **intégralement** pour référence :
> le code source (`crates/metrics-store/src/tables.rs`, `encoding.rs`,
> `bin/import_vm.rs`, `promql/mod.rs`, ainsi que le `Makefile` et `scripts/deploy-pi5.sh`)
> cite encore ses numéros de section d'origine (§0.7, §4.1, §4.2, §6…), **préservés ci-dessous**.


> ## ✅ MIGRATION TERMINÉE — DOCUMENT HISTORIQUE
> VictoriaMetrics a été **entièrement retiré** (code, double écriture, service).
> **redb (`metrics-store`) est désormais la seule TSDB.** Ce document est
> conservé comme **trace historique** de la migration et de ses décisions.
> Pour l'architecture **actuelle**, voir le corps de ce document (sections 1 à 10 ci-dessus).
> Les références à VictoriaMetrics ci-dessous sont donc d'ordre historique.

> **Projet** : Daly-BMS-Rust
> **Objectif** : Remplacer VictoriaMetrics (~120–150 Mo RSS) par un fichier
> [`redb`](https://github.com/cberner/redb) (embedded key-value store, pur Rust,
> ACID, MVCC), stocké sur le NVMe `/mnt/nvme`, en conservant 5 ans d'historique
> et la compatibilité Grafana **via l'API HTTP existante** (pas de plugin natif).
> **Date** : Mai 2026
> **Document jumeau** : [`plan_migration_vm_sqlite_v2.md`](./plan_migration_vm_sqlite_v2.md)
> — ce plan reprend la même structure pour permettre une **comparaison
> directe** des deux options. Lire les §1 et §15 pour la décision.

---

### 0. État d'avancement & démarrage de session

> Cette section est **maintenue à jour à chaque session de travail**. Elle sert
> de point d'entrée pour reprendre la migration sans avoir à relire les 800
> lignes qui suivent. **Toujours la mettre à jour avant de fermer une session.**

#### 0.1 Statut actuel — Mai 2026

| Phase | Avancement |
|---|---|
| Plan rédigé et validé | ✅ document complet (15 sections + comparatif v2) |
| Décision SQLite vs redb | ✅ **redb retenu** (17 mai 2026, cf. §0.4) |
| Décision migration historique | ✅ **import script** depuis VM (volume tractable 48 Mo) |
| Purge cardinalité (code) | ✅ label `pid` retiré + agrégation par nom (commit `7e37e7e`) |
| Purge cardinalité (fantômes en base) | ✅ exécutée 17 mai 2026 : 277 → 209 séries, 0 série avec `pid` (cf. §0.1.2) |
| Audit PromQL exhaustif | ✅ cf. §6.5 ci-dessous (81 expressions, 7 fonctions, 1 subquery) |
| Crate `metrics-store` | ✅ Phase 0 complète (cf. branche `claude/migration-vm-redb-kqUG8`) |
| Shim PromQL → redb | ✅ parse+validate+exec, golden test 81/81 (panel 43 rejet explicite) |
| Dual-write VM+redb | ✅ déployé Pi5 17 mai 2026 (204 → 271 séries après import historique) |
| Endpoint `/api/v1/metrics/ingest` | ❌ pas démarré (parser `prom_text` prêt, reste handler HTTP — non-bloquant pour Phase 4) |
| Bascule Grafana | ✅ exécutée 17 mai 19:20 via `scripts/grafana-redb-switch.sh switch`. Datasource pointe sur `:8080`, `default_backend = "redb"` |
| Import historique VM → redb | ✅ exécuté 18 mai 06:48 (271 séries, ~19M samples, base à ~1.2 Go) |
| Migration routes internes (`/chart`, `/history`, `/dashboards`) | ✅ commit `0839b84` 18 mai — dashboard custom `/dashboard/history` passe aussi par le dispatcher |
| Observation 24 h post-migration complète | ⏳ en cours (lancée 18 mai matin, J1 attendu 19 mai matin) |
| **Phase 4 — Retrait VM** |  ✅ OK, cf. §0.8 |
| **Phase 5 — Cleanup code** |  ✅ OK, cf. §0.9 |
| **Volume actuel data dir** | redb `/mnt/nvme/daly-bms/metrics.redb` ~1.2 Go ; VM `/mnt/nvme/victoria-metrics` 50 Mo |

##### 0.1.1 État réel mesuré sur Pi5 prod (Mai 2026)

Première mesure de référence, **session du 17 mai 2026** — à comparer aux
projections initiales pour calibrer le plan :

| Mesure | Projection initiale du plan | Réel Pi5 prod | Écart |
|---|---|---|---|
| `du -sh /mnt/nvme/victoria-metrics` | (non mesuré) | **48 Mo** | — |
| Total séries (`totalSeries`) | ~10 000 hypothétique (R8) | **277** (avant purge §0.1.2) → **209** (après purge label `pid`) | ÷48 vs hypothèse |
| Total label-value pairs | (non mesuré) | **715** | — |
| Noms de métriques distincts | 25 (audit dashboards) + interne | **96** (`__name__` distinct) | x4 vs golden set |
| Cardinalité top : `bms_cell_voltage` | — | **32** (2 BMS × 16 cell) | OK |
| Cardinalité top : `pi5_process_cpu/mem` | — | **24** chacune | dette : voir §0.1.2 |
| Cardinalité top : `em_process_cpu/mem` | — | **22** chacune | dette : voir §0.1.2 |

Source : `curl http://localhost:8428/api/v1/status/tsdb`. **Service systemd
`victoria-metrics.service` introuvable** — VictoriaMetrics tourne sur le Pi5
mais sous un autre nom d'unit (à identifier — cf. §0.1.3).

**Conséquences à prendre en compte avant Phase 0** :

1. **Gain disque redb devient anecdotique** : §14 projetait 1,2 Go redb vs
   3 Go VM. La base actuelle ne fait que 48 Mo et croît à ~10 Mo/an
   (extrapolation linéaire — à valider avec un 2ᵉ point de mesure dans 1 mois).
   Projection 5 ans réaliste : **~250–300 Mo**, pas 3 Go. Le poids des
   binaires/RSS devient le critère dominant, pas le disque.
2. **Risque R8 (cardinalité) déclassé** : 277 séries actuelles, marge x36
   avant le seuil 10 000 hypothétique. Le LRU sur `series_cache` reste
   pertinent mais n'est plus une priorité.
3. **L'argument principal pro-redb se déplace** : "gain disque ×5" devient
   secondaire. Les arguments qui restent **forts** : (a) suppression
   dépendance C cross-compile aarch64, (b) suppression RSS VictoriaMetrics
   (~120–150 Mo soit ~50 % de la RAM totale des services), (c) design forcé
   via shim PromQL = propreté du code et test golden §6.5.
4. **Migration historique** simplifiée : 48 Mo de VM peuvent être convertis
   en quelques minutes (cf. §0.4-2 ⇒ option "import script" redevient
   tractable vs "archive tar"). À reconsidérer.

##### 0.1.2 Métriques internes qui dominent la cardinalité (audit Mai 2026)

Top 5 par nombre de séries : `bms_cell_voltage` (32), `pi5_process_cpu_percent`
(24), `pi5_process_mem_mb` (24), `em_process_cpu_percent` (22),
`em_process_mem_mb` (22). Les 4 derniers représentent **92 séries soit
33 %** du total et **proviennent du monitoring interne** (un point par
processus enfant).

**Audit code (17 mai 2026)** — 4 producteurs, 0 consommateur :

| Métrique | Fichier producteur | Consommateur (dashboard / alerte / API) |
|---|---|---|
| `pi5_process_cpu_percent{process,pid}` | `crates/daly-bms-server/src/monitor.rs:449` | **aucun** |
| `pi5_process_mem_mb{process,pid}` | `crates/daly-bms-server/src/monitor.rs:455` | **aucun** |
| `em_process_cpu_percent{process,pid}` | `crates/energy-manager/src/monitoring.rs:107` | **aucun** |
| `em_process_mem_mb{process,pid}` | `crates/energy-manager/src/monitoring.rs:111` | **aucun** |

`grep -rn "pi5_process\|em_process"` retourne uniquement les 4 sites de
production. Aucune lecture ailleurs dans le code, aucun dashboard JSON,
aucune alerte. **Le seul cas d'usage déclaré dans le commentaire
`monitor.rs:444` est "identifier le coupable lors d'un freeze"** —
investigation manuelle ad hoc.

**Diagnostic root-cause cardinalité** — la stat
`seriesCountByLabelValuePair` du Pi5 prod montre :

```
process=cargo  → 16 séries actives en base
process=rustc  → 16 séries actives en base
```

Ce sont des **PIDs de compilateurs Rust éphémères**. Chaque
`make build-arm` lance `cargo` + N×`rustc`, qui meurent en quelques minutes
mais dont les séries restent 5 ans (rétention VM). La label `pid` rend
chaque processus unique → cardinalité non-bornée dans le temps. Seuil de
sélection `cpu > 0.1%` (trop bas — un pic momentané suffit à créer la série).

**Trois options de remédiation, par effort croissant** :

1. **Retirer le label `pid`** (1 ligne de code par site, 4 sites au total).
   Résultat : 1 série par nom de process, plus de fantômes. Perte d'info
   marginale (rare d'avoir besoin de distinguer 2 processus du même nom).
   **Recommandé.**
2. **Augmenter le seuil** de 0.1 % à 5 % CPU sustained sur 3 mesures.
   Élimine les pics momentanés (gzip, ls, ssh-keygen). Plus complexe (état).
3. **Retirer complètement** ces 4 métriques de VM et exposer un endpoint
   REST `/api/v1/system/top-processes` qui lit `/proc/*/stat` à la demande.
   Aucune série stockée, mais pas d'historique au moment du freeze (perte).

**Action faite (17 mai 2026)** : option 1 implémentée.
- `daly-bms-server/src/monitor.rs:444-471` — label `pid` retiré, agrégation
  par nom via `HashMap<&str, (f32 cpu, f32 mem)>` qui somme tous les
  processus du même nom dans le snapshot courant.
- `energy-manager/src/monitoring.rs:104-122` — même logique sur le tuple
  `(cpu, mem, name, pid)` produit par `collect_processes()`.
- Compilation OK, zéro warning.

**Action restante (à exécuter sur Pi5 après déploiement)** : purger les
séries fantômes existantes en base VM.

```bash
# 1. Déployer le nouveau code (daly-bms-server + energy-manager)
ssh pi5 'cd Daly-BMS-Rust && make sync && make build-arm && make build-energy-arm \
  && sudo systemctl stop daly-bms energy-manager \
  && sudo cp target/aarch64-unknown-linux-gnu/release/daly-bms-server /usr/local/bin/ \
  && sudo cp target/aarch64-unknown-linux-gnu/release/energy-manager /usr/local/bin/ \
  && sudo systemctl start daly-bms energy-manager'

# 2. Purger les séries fantômes (PIDs morts) - les nouvelles écritures
#    sans pid reprendront immédiatement avec la cardinalité bornée
ssh pi5 'for m in pi5_process_cpu_percent pi5_process_mem_mb \
                  em_process_cpu_percent em_process_mem_mb; do
  curl -s -X POST "http://localhost:8428/api/v1/admin/tsdb/delete_series?match\[\]=$m"
done'

# 3. Vérification : la cardinalité doit être tombée
ssh pi5 'curl -s http://localhost:8428/api/v1/status/tsdb | jq .data.totalSeries'
```

**Résultat mesuré (17 mai 2026, après déploiement)** :

| Métrique | Avant purge | Cible théorique | **Réel après purge** |
|---|---|---|---|
| `totalSeries` | 277 | 185 | **209** (−25 %) |
| Séries portant le label `pid` | 92 | 0 | **0** ✅ |
| Cardinalité `pi5_process_cpu_percent` | 24 (instable, fuite par PID) | — | **6** (borné, 1 par nom) |
| Cardinalité `em_process_cpu_percent` | 22 (instable) | — | **6** (borné) |

Le delta vs cible théorique (209 vs 185) vient des top-processes désormais
**stables et bornés** : 4 métriques × ~6 noms de processus actifs = 24
séries de top-process **non-éphémères** (vs 92 éphémères avant qui
fuyaient à chaque `make build-arm`). Verification : la requête
`{__name__=~"pi5_process_.+|em_process_.+",pid!=""}` retourne 0 série.

**Status** : ✅ purge terminée, base prête pour la migration redb Phase 0.

##### 0.1.3 Service VictoriaMetrics — identifié 17 mai 2026

| Élément | Valeur |
|---|---|
| Unit systemd | **`victoriametrics.service`** (sans tiret) |
| Fichier unit | `/etc/systemd/system/victoriametrics.service` |
| Binaire | `/usr/local/bin/victoria-metrics-prod` |
| User | `victoriametrics` (apparaît tronqué `victori+` dans `ps`) |
| RSS mesuré (Mai 2026) | **135 Mo** (aligné avec estimation §14 : 120–150 Mo) |
| Listen | `0.0.0.0:8428` |
| Uptime au moment du diag | 19 h continues, état `active (running)` |
| Args clés | `-storageDataPath=/mnt/nvme/victoria-metrics`, `-retentionPeriod=5y`, `-maxLabelsPerTimeseries=30`, `-search.maxQueryDuration=30s`, `-search.maxConcurrentRequests=4`, `-selfScrapeInterval=0` (= self-scrape désactivé, important pour la migration : pas de métriques internes VM à reproduire côté redb) |

`CLAUDE.md` §0 mis à jour avec les commandes correctes
(`systemctl status victoriametrics`, etc.).

Pour la phase 4 (retrait VM), la séquence sera :
```bash
sudo systemctl stop victoriametrics
sudo systemctl disable victoriametrics
sudo rm /etc/systemd/system/victoriametrics.service
sudo systemctl daemon-reload
# /mnt/nvme/victoria-metrics conservé read-only comme archive froide
sudo mv /mnt/nvme/victoria-metrics /mnt/nvme/victoria-metrics.archive
```

#### 0.2 Ce qui a changé dans le code depuis la rédaction (impacte le plan)

Aucun changement sur les producteurs (`vm_client.rs`, `energy-manager`) ni
sur les consommateurs (`api/history.rs`, `api/dashboards.rs`). Les évolutions
récentes ne touchent **que** le frontend dashboard, mais elles **étendent la
surface PromQL** à transpiler :

1. **`docs/grafana-solar_pv_dashboard.json`** (commit `08f23c7`, 11 panels
   IDs 1001–1011) — ajoute des expressions `increase(...[24h|30d|1h|1d])`
   avec arithmétique vectorielle `(a - b) / a * 100`. **À couvrir par le
   transpileur dès Phase 2.** Détail §6.5.
2. **Onglets `/dashboard/history`** (commit `d9580fd`) — refactor frontend
   uniquement (clé `tab_id` ajoutée au storage `dashboards.db` SQLite).
   **Aucun impact sur le plan redb** : la base `dashboards.db` (rusqlite)
   reste en place, c'est uniquement `metrics.db`/VictoriaMetrics qui migre.
3. **Catalog multi-sources** (`Catalog::load_default` charge 2 fichiers JSON)
   — le shim doit servir les exprs des deux dashboards sans distinction.

#### 0.3 Kit de démarrage pour une nouvelle session

Quand tu ouvres une session pour cette migration, fais dans l'ordre :

```bash
# 1. Vérifier la branche active
git status && git branch --show-current

# 2. Lire ces fichiers DANS CET ORDRE
#    a. CLAUDE.md                              (contexte projet, ~3 min)
#    b. ./metriques-redb-architecture.md §0..§3  (état + archi, ~10 min)
#    c. ./metriques-redb-architecture.md §6.5    (audit PromQL exhaustif)
#    d. Pour la phase visée : §4 (schéma), §5 (crate), §10 (bascule)

# 3. Examiner le code existant qu'on remplace
#    - crates/daly-bms-server/src/vm_client.rs                  (producteur principal)
#    - crates/daly-bms-server/src/api/history.rs                (consommateur, 159 l)
#    - crates/daly-bms-server/src/api/dashboards.rs:get_panel_data  (consommateur)
#    - crates/daly-bms-server/src/metrics.rs                    (ingest endpoint actuel ?)

# 4. Vérifier l'état réel sur Pi5 (volume, RSS VM, lag)
ssh pi5 'du -sh /mnt/nvme/victoria-metrics && \
         systemctl status victoria-metrics --no-pager | head -20 && \
         curl -s http://localhost:8428/api/v1/status/tsdb | head -50'

# 5. Mettre à jour §0.1 (statut) à la fin de la session
```

**Branche de travail recommandée** : `claude/migration-vm-redb-<jour>` —
**ne pas réutiliser** `claude/detailed-migration-plan-BJxVd` (cette dernière
contient les ajouts Solar PV et doit rester focalisée dessus).

#### 0.5 État d'avancement du code (Mai 2026 — fin Phase 0)

Branche : `claude/migration-vm-redb-kqUG8`. 6 commits, 35 tests verts
(`cargo test -p metrics-store`), workspace `cargo check` OK.

| Ticket | Livrable | Commit |
|---|---|---|
| 0.3 | Squelette crate `metrics-store` (Cargo.toml + module skeleton) | `e48d922` |
| 0.4-0.5a | `writer.rs` (thread dédié batché 4 fsync/s, cache LRU series_id), `reader.rs` (query_range_raw / query_range_buckets / list_series), `agg.rs` (Avg/Min/Max/Sum/Count/First/Last) | `ecd8d56` |
| 0.5b | `tiering.rs` : `compact_raw_to_hourly`, `compact_hourly_to_daily`, `spawn_maintenance` périodique, `AggBucketBuilder` chainable raw→hourly→daily | `672972b` |
| 0.6 | `promql/` : parser via `promql-parser 0.9`, `validate` (whitelist §6.5), `exec::Evaluator` (selector + binops + aggrégations + range fns + tier auto §6.3) + golden test `tests/golden_promql.rs` 81/81 (panel 43 marqué `KNOWN_UNSUPPORTED_PANELS`) | `addf22f` |
| 0.7 | `prom_text.rs` : parser format Prometheus exposition (commentaires, échappements, virgule terminale, `+inf`/`-inf`/`nan`, lignes en erreur tracées) | `74058e3` |
| 0.8 | Benches criterion `insert_batch_raw` + `range_scan_raw` ; ~150 Kelem/s insert + ~13 Melem/s scan sur x86 dev | `7e51678` |
| Phase 1 (code) | `daly-bms-server` : section `[metrics_store]` dans `AppConfig`, hook dual-write dans `VmClient::write_rows` (try_write best-effort), init `MetricsStore` + `spawn_maintenance` dans `main.rs`, ligne `[metrics_store]` ajoutée à `Config.toml` (défaut `enabled = false`) | (commit en cours) |

**Versions notables** : `redb = "4.1"` (bump 2.2 → 4.1 demandé), pas
de dépendance C, pure-Rust 100 %.

**Reste à faire pour la Phase 1 ops** (intervention humaine Pi5) :
```bash
# 1. Activer le dual-write dans Config.toml
sed -i 's/^enabled = false$/enabled = true/' Config.toml  # section [metrics_store]

# 2. Déployer (cf. §0 du CLAUDE.md)
make sync
make build-arm
sudo systemctl stop daly-bms
sudo cp target/aarch64-unknown-linux-gnu/release/daly-bms-server /usr/local/bin/
sudo cp Config.toml /etc/daly-bms/config.toml
mkdir -p /mnt/nvme/daly-bms      # s'assurer que db_path est writable
sudo systemctl start daly-bms

# 3. Observer (≥ 24 h)
journalctl -u daly-bms -f | grep -i 'metrics-store\|dual-write'
du -sh /mnt/nvme/daly-bms/metrics.redb   # doit croître
# Pas d'erreurs "queue pleine" en régime nominal.
```

À noter : la **migration historique** des 48 Mo de VictoriaMetrics
(décision §0.4-2 : import script) reste à scripter. Elle peut se faire
en parallèle du dual-write — pas de blocage.

#### 0.6 Bascule Grafana (Phase 3, code + script livrés 17 mai 2026)

Le serveur expose les endpoints Prometheus standard avec un **dispatch
dynamique** piloté par `[metrics_store].default_backend` :

| Route | `default_backend = "vm"` | `default_backend = "redb"` |
|---|---|---|
| `GET /api/v1/query` | proxy VictoriaMetrics | shim PromQL redb |
| `GET /api/v1/query_range` | proxy VictoriaMetrics | shim PromQL redb |
| `GET /api/v1/labels` | liste statique | scan dynamique `series_meta` |
| `GET /api/v1/series` | (toujours redb) | shim |
| `GET /api/v1/label/:name/values` | (toujours redb) | shim |
| `GET /-/healthy` | (toujours redb) | shim |
| `GET /api/v1/redb/*` | (toujours redb) | shim — utile pour parité diff |

**Bascule automatisée via `scripts/grafana-redb-switch.sh`** (commit
`8d7e718`). Le script change l'URL de la datasource existante
(uid=`victoriametrics`) de `:8428` → `:8080`, donc **aucun dashboard ni
alerte n'a besoin d'être modifié** (l'UID reste le même, les 5 alertes
Grafana suivent automatiquement). Backup horodaté auto sous
`/var/backups/migration-redb/`. Health-checks à chaque étape. Idempotent.

```bash
sudo scripts/grafana-redb-switch.sh status     # voir l'état avant/après
sudo scripts/grafana-redb-switch.sh switch     # bascule (≈ 15 s)
sudo scripts/grafana-redb-switch.sh rollback   # restauration (≈ 15 s)
```

Rollback inverse instantané — VM reste alimenté tant que le dual-write
Phase 1 tourne.

#### 0.7 État au 18 mai 2026 (matin) — Phase 3 terminée

Récap des étapes franchies depuis hier soir :

| Étape | Date | Commit | Résultat |
|---|---|---|---|
| Bascule Grafana via script | 17 mai 19:20 | `grafana-redb-switch.sh switch` exécuté | `default_backend = "redb"` + datasource `:8080` |
| 1ère validation visuelle | 17 mai 19:30 | — | ESS Overview OK ; PV Solar partiellement OK (manque d'historique pré-bascule) |
| Fix POST + secondes float | 17 mai 19:42 | `2e9b43e` | Grafana hit notre dispatcher en POST avec timestamps Prom-standard |
| Import historique VM → redb | 18 mai 06:48 | binaire `import-vm` (`5fc897f`) | 271 séries, ~19M samples lus en 51 s ; ~95 % écrits avant kill (deadlock drop fixé après dans `d21a506`) |
| Migration routes internes | 18 mai matin | `0839b84` | `/api/v1/chart/*`, `/api/v1/history/*`, `/api/v1/dashboards/*` dispatchent vm/redb |

**État courant** : Grafana ET le dashboard custom `/dashboard/history`
servent tous les deux depuis redb (via dispatcher). VM tourne en
parallèle pour le dual-write (reçoit aussi les écritures). Aucun point
de lecture ne touche encore VM directement (sauf si on remet
`default_backend = "vm"`).

**Reste à faire** : observation 24 h, puis Phase 4 (retrait VM), puis
Phase 5 (cleanup code). Détails ci-dessous.

##### J0 — Validation post-migration routes internes

```bash
# Sur Pi5 — déployer le binaire avec les routes internes migrées
make sync && make build-arm
sudo systemctl stop daly-bms
sudo cp target/aarch64-unknown-linux-gnu/release/daly-bms-server /usr/local/bin/
sudo systemctl start daly-bms

# 1. Le dashboard custom /dashboard/history doit afficher les courbes
#    avec le même historique qu'avant (puisque redb a l'historique post-import)
curl -s "http://127.0.0.1:8080/api/v1/chart/history?minutes=60" | jq '.solar | length'
curl -s "http://127.0.0.1:8080/api/v1/history/energy?period=day" | jq 'keys'

# 2. Test côte à côte (toggle backend) pour confirmer la parité
sudo sed -i 's/^default_backend = "redb"$/default_backend = "vm"/' /etc/daly-bms/config.toml
sudo systemctl restart daly-bms; sleep 5
curl -s "http://127.0.0.1:8080/api/v1/chart/history?minutes=60" > /tmp/vm.json
sudo sed -i 's/^default_backend = "vm"$/default_backend = "redb"/' /etc/daly-bms/config.toml
sudo systemctl restart daly-bms; sleep 5
curl -s "http://127.0.0.1:8080/api/v1/chart/history?minutes=60" > /tmp/redb.json
diff <(jq -S . /tmp/vm.json) <(jq -S . /tmp/redb.json) | head -30
# Écarts attendus : formatage f64 ("12.34" vs "12.340000076"), timestamps
# secondes float vs ms. Pas d'écart sémantique sur les valeurs.
```

Critères go pour Phase 4 :
- Dashboard custom `/dashboard/history` 100% fonctionnel en mode redb
- Aucune route `/api/v1/chart/*` ou `/api/v1/history/*` ne casse
- Diff `chart/history?minutes=60` vm vs redb : 0 écart sémantique

##### J0 → J1 — Observation 24 h

```bash
# Boucle d'observation (lancée en daemon)
nohup bash -c 'while sleep 1800; do
  TS=$(date -Iseconds)
  REDB_SIZE=$(du -sh /mnt/nvme/daly-bms/metrics.redb | cut -f1)
  ERRS=$(journalctl -u daly-bms --since "30 minutes ago" --no-pager 2>/dev/null \
    | grep -ciE "PromQl|parse error|panic|unsupported")
  echo "[$TS] redb=$REDB_SIZE err_count=$ERRS"
done' > /tmp/redb-observation.log 2>&1 &
disown
```

Critères pour passer en Phase 4 (à vérifier le lendemain) :
- err_count = 0 sur les 24 h
- Croissance redb linéaire et prédictible (~10-50 Mo/jour selon volume)
- Grafana visuellement OK sur 6h, 24h, 7j, 30j
- Dashboard custom `/dashboard/history` visuellement OK
- Aucun ticket de support / panel "No data" non attendu

#### 0.8 Phase 4 — Retrait VictoriaMetrics ( ✅ Fait)


## 4.3 Grafana + dashboard custom continuent à servir depuis redb  ✅ Fait
curl -s 'http://127.0.0.1:8080/-/healthy' && echo
curl -s 'http://127.0.0.1:8080/api/v1/query?query=up' | jq '.status'
curl -s 'http://127.0.0.1:8080/api/v1/chart/history?minutes=60' | jq '.solar | length'
```
### 0.9 Phase 5 — Cleanup code ( ✅ Fait)

#### Commit 5.1 — Retrait des écritures VM (côté daly-bms-server)

But : couper le dual-write côté daly-bms-server (qui échoue silencieusement
depuis Phase 4 mais consomme de la CPU pour rien à chaque sample).

Fichiers :
- `crates/daly-bms-server/src/vm_client.rs` : retirer le wrapping HTTP
  vers VM dans `write_rows()`. Le method devient un simple fan-out
  vers le writer redb (renommer en `MetricsWriter` éventuellement).
- `crates/daly-bms-server/src/main.rs` : retirer le bloc
  `let vm_handle = if config.victoriametrics.enabled { ... }` —
  remplacer par une initialisation directe du writer redb.
- `crates/daly-bms-server/src/state.rs` : retirer le champ
  `pub vm: Option<Arc<VmClient>>` et tous ses usages dans les call sites
  `state.vm` (qui restent uniquement pour les fallbacks "VM_disabled" qui
  ne s'appliquent plus). Garder uniquement le test
  `state.metrics_store.is_none()` pour les pré-flights.
- `Config.toml` : retirer la section `[victoriametrics]`.

Critère de succès :
- `grep -rn "VmClient\|victoriametrics\|VmConfig" crates/` ne retourne
  rien dans le code de prod (seul subsiste éventuellement
  `import-vm` qui en a besoin pour parser l'export).

#### Commit 5.2 — Retrait du dispatch dans api/promql.rs

But : maintenant qu'il n'y a plus qu'un seul backend, simplifier
`api/promql.rs` qui dispatchait vm/redb.

Fichiers :
- `crates/daly-bms-server/src/api/promql.rs` : retirer `use_redb()`,
  les branches `if use_redb { ... } else { ... }`. Toutes les routes
  appellent directement le shim redb (via `redb_api::run_*`).
- `crates/daly-bms-server/src/state.rs` : retirer
  `dispatched_query_range`, `dispatched_query_instant`,
  `redb_query_range_json`, `redb_query_instant_json`, `query_backend`.
  Remplacer les appels par un appel direct au `Evaluator` du shim.
- `crates/daly-bms-server/src/config.rs` : retirer le champ
  `default_backend` et la fonction `default_metrics_query_backend()`.
- `Config.toml` : retirer `default_backend = "vm"` de `[metrics_store]`.

Critère de succès :
- `grep -rn "default_backend\|use_redb\|dispatched_query" crates/` retourne
  zéro résultat.
- L'API `/api/v1/query[_range]` continue à fonctionner à l'identique
  (regression test : curl avant/après doit donner même JSON).

#### Commit 5.3 — Retrait des routes `/api/v1/redb/*` redondantes

But : maintenant que `/api/v1/query[_range]` parle directement redb,
les routes `/api/v1/redb/*` (qui étaient des aliases pour debug pendant
la transition) sont redondantes. Les supprimer simplifie le surface
d'API.

Fichiers :
- `crates/daly-bms-server/src/api/mod.rs` : retirer les routes
  `.route("/api/v1/redb/...", ...)` (6 routes).
- `crates/daly-bms-server/src/api/redb.rs` : renommer le module en
  `crates/daly-bms-server/src/api/query.rs` (ou fusionner dans
  `promql.rs`). Garder uniquement les `run_*` fonctions, supprimer
  les wrappers Axum (`query_instant`, `query_range`, etc.) qui
  étaient sur les routes `/redb/*`.

**Décision à prendre** : on peut aussi GARDER `/api/v1/redb/*` comme
alias permanent (zéro coût mainteneur, utile pour debug). Recommandation :
**garder** — `/api/v1/redb/healthy` reste un endpoint utile pour
monitoring externe.

#### Commit 5.4 — Mise à jour de la documentation

Fichiers :
- `CLAUDE.md` § 0 : retirer les commandes `systemctl status
  victoriametrics`, `journalctl -u victoriametrics`, `curl
  http://localhost:8428/api/v1/status/tsdb`. Ajouter les équivalents
  redb (`du -sh /mnt/nvme/daly-bms/metrics.redb`, `curl
  http://localhost:8080/api/v1/redb/series | jq 'length'`).
- `./metriques-redb-architecture.md` § 0.1 (statut) : tout coché ✅,
  marquer le plan comme **terminé**.
- `docs/Tuning-Victoria.md` : marquer obsolète (référencer en note
  historique).

#### Commit 5.5 — Cleanup Grafana provisioning

Fichiers :
- `contrib/grafana/provisioning/datasources/victoriametrics.yaml` :
  renommer le `name:` interne à "Daly Metrics (redb)" pour refléter
  la réalité (l'UID reste `victoriametrics` pour ne pas casser les
  dashboards). Le fichier `daly-metrics.yaml` ajouté Phase 3 peut être
  supprimé (il faisait doublon — l'UID `daly-metrics` n'est référencé
  par aucun dashboard).
- Pi5 : `sudo cp contrib/grafana/provisioning/datasources/victoriametrics.yaml /etc/grafana/provisioning/datasources/` puis `sudo systemctl restart grafana-server`.

#### Commit 5.6 — Retrait du service VM des contribs

Fichiers :
- `contrib/victoriametrics.service` : supprimer
- `contrib/victoriametrics-scrape.yml` : supprimer
- `contrib/install-systemd.sh` : retirer les références à victoriametrics
- `scripts/migrate-vm-to-nvme.sh` : marquer obsolète ou supprimer

#### Commit 5.7 — energy-manager (optionnel)

Si `crates/energy-manager` utilise un client VM pour écrire des
métriques, le retirer aussi. Sinon, laisser tel quel.

```bash
grep -rn "VmClient\|victoriametrics\|8428" crates/energy-manager/src/
```

Si des écritures VM existent côté energy-manager, le hook dual-write
côté daly-bms-server n'a JAMAIS capté ces métriques. Ce qui veut dire
que redb ne les a pas. **Avant de cliquer Phase 4**, il fallait
s'assurer que energy-manager n'a pas son propre path d'écriture VM
qui contournerait notre dispatcher. À auditer.

### 0.10 État final attendu (snapshot post-cleanup)

```
Pi5 (192.168.1.141, pi5compute)
  mosquitto-broker (systemd, :1883 + :9001 WS) — inchangé
  daly-bms-server (systemd, :8080)
    ├── RS485 /dev/ttyUSB0 → 2 BMS + 3 ET112 + 1 PRALRAN
    ├── REST API + WebSocket :8080 — inchangé côté API
    ├── MQTT subscribe/publish → 127.0.0.1:1883
    └── metrics-store (redb à /mnt/nvme/daly-bms/metrics.redb)
        ├── writer batché (4 fsync/s)
        ├── reader MVCC (snapshots lock-free)
        ├── tier maintenance raw→hourly→daily (4x/jour)
        └── shim PromQL (sert /api/v1/query, /api/v1/query_range,
                         /api/v1/labels, /api/v1/series,
                         /api/v1/label/:n/values, /-/healthy)
  energy-manager (systemd, :8081) — inchangé
```

**Diff vs avant migration** :
- ❌ victoriametrics.service (supprimé)
- ❌ /usr/local/bin/victoria-metrics-prod (binaire C 28 Mo)
- ❌ /mnt/nvme/victoria-metrics/ (50 Mo data, archivé 30 j puis supprimé)
- ✅ /mnt/nvme/daly-bms/metrics.redb (200-400 Mo prévus à 30 j, max ~2 Go à 5 ans)
- ✅ crates/metrics-store/ (pure-Rust, 0 dep C, cross-compile aarch64 trivial)
- RSS gagné : ~135 Mo (cf. §0.1.3)
- Surface code mainteneur : -500 lignes (vm_client.rs+promql.rs dispatch),
  +1500 lignes (metrics-store crate). Net : +1000 lignes mais une seule
  source de vérité, ownership clair.

**Bénéfices opérationnels confirmés** :
1. Cross-compile aarch64 sans dep C externe (gain CI)
2. Pas de fichier de config externe (la base redb est self-contained)
3. Plus de scraping interne (selfScrapeInterval=0 dans VM était déjà ainsi,
   donc gain = 0 sur cet axe)
4. Pas de mainteneur tiers à suivre (VM = Roman / VictoriaMetrics Ltd,
   redb = Christopher Berner). Trade-off mais maîtrise du shim PromQL.

### 0.4 Décisions prises (17 mai 2026)

1. ✅ **Backend = redb** (pure Rust). Décision actée après calibrage §0.1.1
   qui a fait tomber l'argument disque. Critères retenus : suppression
   dépendance C cross-compile aarch64 (gain CI), gain RAM 120 Mo via
   retrait de VictoriaMetrics (commun aux deux backends), design forcé
   via shim PromQL.
2. ✅ **Migration historique** = **import script** (et non archive tar).
   Le volume mesuré (48 Mo) rend la conversion tractable en quelques
   minutes via `curl /api/v1/export` + parser. L'archive tar VM reste
   conservée comme sauvegarde froide.
3. **Subquery panel 43** (§6.5) : à trancher quand on attaquera le
   transpileur. Recommandation tenue : réécriture en deux requêtes JS.
4. ✅ **Purge cardinalité avant bascule** (§0.1.2) : retirer le label
   `pid` des 4 métriques de top-processus + delete_series sur les
   fantômes. À faire avant Phase 1 dual-write.

---

## 1. Pourquoi évaluer redb ?

### 1.1 Comparaison synthétique vs SQLite (le candidat du plan v2)

| Critère | SQLite (rusqlite 0.31, bundled) | redb 2.x |
|---|---|---|
| Langage | C compilé en bundled (~800 Ko de code C) | **100 % Rust** |
| Format | SQL + B-tree pages | B-tree COW (copy-on-write) |
| ACID | Oui (WAL) | Oui (MVCC natif) |
| Concurrence | 1 writer + N readers via WAL | 1 writer + N readers via snapshots MVCC |
| Index | À déclarer (`CREATE INDEX`) | Implicite par clé (la clé EST l'index) |
| Requêtes | SQL complet (jointures, agrégats, fenêtres) | KV pur — agrégats à coder en Rust |
| API | `prepare_cached` + `execute` + `query_map` | `open_table()`, `range()`, `insert()`, `get()` |
| Compilation | dépendance C (~30 s incrémental) | pure Rust (~2 s incrémental) |
| Binaire ajouté | ~1,5 Mo (libsqlite3) | ~250 Ko (redb) |
| Outils CLI / GUI | `sqlite3`, DataGrip, DB Browser, mille options | aucun outil tiers — debug par code Rust |
| Plugin Grafana | `frser-sqlite-datasource` (existant, communautaire) | **aucun** — accès Grafana uniquement via notre API HTTP |
| Backup à chaud | `sqlite3 .backup` (atomique, copy-on-write) | `Database::compact` ou copie du fichier hors transaction |
| Maturité | 25 ans, plus utilisée au monde | jeune (v1.0 en 2023, v2.x stable), single-maintainer (cberner) |
| Performance écritures séquentielles | très bonne (WAL batché) | excellente (1 transaction = 1 fsync) |
| Performance scans clé ordonnée | excellente | **encore meilleure** (clé = index naturel, zéro overhead SQL) |
| RAM (cache pages) | configurable (PRAGMA cache_size) | configurable (`Builder::set_cache_size`) |
| Migration de schéma | `ALTER TABLE` flexible | recréer la table ou écrire du code de migration |

### 1.2 Bénéfices spécifiques pour notre cas

1. **Build plus simple** : pas de compilation C dans le pipeline `make build-arm`
   (cross-compile aarch64). On supprime une dépendance native — utile sur Pi5
   où le toolchain C est lourd. L'image binaire `daly-bms-server` perd ~1 Mo.
2. **Densité de scan ordonnée** : nos requêtes les plus chaudes sont des range
   scans `(series_id, ts BETWEEN T1 AND T2)`. redb stocke les paires triées
   par clé : c'est exactement le pattern où il bat SQLite (pas de parsing SQL,
   pas de planner, juste un `range()` itérateur).
3. **MVCC sans WAL séparé** : pas de fichier `-wal` qui grossit à surveiller.
   Les snapshots de lecture sont des copies COW de l'arbre B-tree.
4. **API plus simple à raisonner** : pas de `prepare_cached`, pas de mutex
   autour d'une `Connection`. Une `Database` est partageable directement
   entre threads (`Arc<Database>`).

### 1.3 Coûts spécifiques

1. **Pas de SQL = tout en Rust** : `avg_over_time`, `increase`, `sum by` doivent
   être implémentés à la main au-dessus des range scans. C'est un coût initial
   non négligeable mais déterministe (et qui sera de toute façon nécessaire
   pour le transpilateur PromQL, même côté SQLite — cf. plan v2 §5).
2. **Pas de plugin Grafana natif** : on **doit** passer par l'API HTTP
   `/api/v1/query_range` du serveur. Décision structurante (cf. §8).
3. **Pas de `sqlite3` CLI pour l'ops** : pour débugger en prod, il faut écrire
   des binaires Rust (`metrics-store-cli`). Coût : ~1 jour de dev pour un outil
   minimal (`dump`, `count`, `top-series`, `verify`).
4. **Maturité moindre** : redb est mature et bien testé (CI quickcheck), mais
   c'est un projet single-maintainer. Le format de fichier est stable depuis
   1.0, mais il faut prévoir un plan de migration si une montée de version
   majeure cassait la compat (jamais arrivé en 1.x → 2.x mais possible en
   3.x). SQLite n'a pas ce risque.

---

## 2. Cartographie de l'existant (identique au plan v2)

Identique au plan jumeau §1 (deux producteurs, 12 timers throttle, 13 sites
d'écriture, PromQL embarqué via `include_str!`). Ne pas dupliquer ici — se
reporter à [`plan_migration_vm_sqlite_v2.md` §1](./plan_migration_vm_sqlite_v2.md#1-cartographie-de-lexistant-résultat-de-laudit).

> **Différence importante** vs v2 §1.4 : les fichiers à modifier sont les
> mêmes mais la dépendance dans `Cargo.toml` est :
> ```toml
> redb = "2.2"        # remplace rusqlite (workspace)
> ```
> `rusqlite 0.31 bundled` reste utilisé pour `alerts.db` et
> `dashboard_storage`, on ne le retire pas — les deux coexistent (rusqlite =
> état applicatif, redb = métriques time-series).

---

## 3. Architecture cible

Identique à v2 §2, à un détail près : la couche d'accès Grafana **n'a plus**
de plugin natif. Tout passe par l'API Prometheus-compatible du serveur Rust.

```
┌────────────────────────────────────────────────────────────────────────┐
│                       Raspberry Pi 5 CM (4 Go)                          │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │  daly-bms-server (Rust, axum)                                    │   │
│  │    ├── RS485 polling (state.rs)                                  │   │
│  │    ├── crate metrics-store (nouveau, backend redb)               │   │
│  │    │     ├── Database (Arc<redb::Database>)                      │   │
│  │    │     ├── Writer task : mpsc → batch → write_txn              │   │
│  │    │     ├── Reader API : read_txn snapshots MVCC (lock-free)    │   │
│  │    │     ├── Aggregator : avg/min/max/sum en Rust streamé        │   │
│  │    │     └── PromQL→ops transpiler (AST → plan d'exécution)      │   │
│  │    ├── /api/v1/metrics/ingest  (Prometheus text) ◄───────────────┼───┐
│  │    └── /api/v1/query, /query_range, /labels                      │   │
│  │       ▲                                                           │   │
│  │       │  (Grafana datasource Prometheus pointe ici, PAS de        │   │
│  │       │   plugin SQLite — c'est l'unique chemin d'accès)          │   │
│  └──────────────────────────────────────────────────────────────────┘   │
│                                  │                                       │
│                                  ▼                                       │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │  /mnt/nvme/daly-bms/metrics.redb (single file, B-tree COW)       │   │
│  │    ├── TABLE_SERIES_BY_KEY  ((metric, labels_json) → series_id) │   │
│  │    ├── TABLE_SERIES_META    (series_id → SeriesMeta)            │   │
│  │    ├── TABLE_RAW            ((series_id, ts_ms) → value)        │   │
│  │    ├── TABLE_HOURLY         ((series_id, bucket_ms) → AggBucket)│   │
│  │    └── TABLE_DAILY          ((series_id, bucket_ms) → AggBucket)│   │
│  └──────────────────────────────────────────────────────────────────┘   │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │  energy-manager — POST Prometheus text vers daly-bms-server      │   │
│  └──────────────────────────────────────────────────────────────────┘   │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │  Grafana (systemd) — datasource Prometheus                        │   │
│  │  URL : http://127.0.0.1:8080  ← API du serveur, pas plugin SQLite│   │
│  └──────────────────────────────────────────────────────────────────┘   │
└────────────────────────────────────────────────────────────────────────┘
```

---

## 4. Schéma redb cible

### 4.1 Types et encodages

`redb` impose des types fortement typés pour les clés/valeurs via `Key` / `Value`
trait. On peut soit :
- (a) utiliser les types primitifs (`u64`, `&str`, `&[u8]`) avec encodage manuel ;
- (b) dériver `redb::Value` sur des structs via `redb_derive` (option simple
  mais ajoute une dépendance) ;
- (c) encoder en `&[u8]` big-endian pour les clés composites (préserve l'ordre
  lexicographique = ordre numérique).

Choix retenu : **(c) big-endian explicite** pour les clés composites
(`series_id u32 + ts_ms i64` → `[u8; 12]`), et `&[u8]` pour les valeurs
agrégées sérialisées avec `bincode` (compact, déterministe).

```rust
// crates/metrics-store/src/encoding.rs

pub fn enc_skey(series_id: u32, ts_ms: i64) -> [u8; 12] {
    let mut k = [0u8; 12];
    k[0..4].copy_from_slice(&series_id.to_be_bytes());
    // ts_ms : décaler pour préserver l'ordre des négatifs (rare, mais propre)
    k[4..12].copy_from_slice(&(ts_ms as u64 ^ 0x8000_0000_0000_0000).to_be_bytes());
    k
}
pub fn dec_skey(k: &[u8]) -> (u32, i64) {
    let s = u32::from_be_bytes(k[0..4].try_into().unwrap());
    let raw = u64::from_be_bytes(k[4..12].try_into().unwrap());
    let ts = (raw ^ 0x8000_0000_0000_0000) as i64;
    (s, ts)
}
```

> L'ordre big-endian garantit qu'un scan `range((s, t1)..(s, t2))` retourne
> les points triés par `ts_ms` croissant pour une série fixée — exactement le
> motif des range queries du frontend et de Grafana.

### 4.2 Définitions de tables

```rust
// crates/metrics-store/src/tables.rs

use redb::{TableDefinition, U32, U64};

// Index inverse : recherche d'un series_id à partir de (metric, labels_json).
// La clé sérialise (metric + 0x00 + labels_json) en bytes pour ordre stable.
pub const TABLE_SERIES_BY_KEY: TableDefinition<&[u8], u32> =
    TableDefinition::new("series_by_key");

// Métadonnées par série : metric, labels_json, last_seen_ts, first_seen_ts.
// Valeur sérialisée en bincode (≤ 200 octets par série).
pub const TABLE_SERIES_META: TableDefinition<u32, &[u8]> =
    TableDefinition::new("series_meta");

// Compteur monotone pour générer le prochain series_id.
pub const TABLE_META: TableDefinition<&str, u64> =
    TableDefinition::new("meta");                                // clé "next_series_id"

// Points bruts : clé = (series_id, ts_ms) encodés big-endian (12 octets).
pub const TABLE_RAW: TableDefinition<&[u8], f64> =
    TableDefinition::new("metrics_raw");

// Points compactés : clé = (series_id, bucket_ms big-endian).
// Valeur = AggBucket bincode (40 octets : 5×f64 + u32 cnt).
pub const TABLE_HOURLY: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("metrics_hourly");
pub const TABLE_DAILY: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("metrics_daily");

##[derive(serde::Serialize, serde::Deserialize)]
pub struct AggBucket {
    pub avg:   f64,
    pub min:   f64,
    pub max:   f64,
    pub sum:   f64,
    pub first: f64,                // pour increase() après compaction
    pub last:  f64,                // pour increase() après compaction
    pub cnt:   u32,
}
```

> **Pourquoi pas une table séparée par série ?** redb supporte des milliers de
> tables, mais ouvrir/fermer une table par requête est plus coûteux que faire
> un range scan sur un préfixe `series_id`. Avec ~80 séries on est largement
> dans le régime où une table unique préfixée bat N tables.

### 4.3 Volumétrie projetée

> ⚠️ **Cette section a été recalibrée avec les chiffres réels mesurés
> sur Pi5 prod (cf. §0.1.1).** La projection initiale supposait ~200 séries
> et un taux de poll continu — la réalité est 277 séries dont 92 internes
> potentiellement purgeables, et 48 Mo après plusieurs mois d'exploitation.

**Projection initiale (hypothèse théorique, ~42 M points raw)** :

| Table | Bytes / entrée | Total théorique |
|---|---|---|
| `metrics_raw` | 12 (clé) + 8 (f64) + overhead B-tree ≈ 28 octets | 41 M × 28 = ~1,1 Go |
| `metrics_hourly` | 12 + 40 + overhead ≈ 60 octets | 700 k × 60 = 42 Mo |
| `metrics_daily` | 12 + 40 + overhead ≈ 60 octets | 146 k × 60 = 9 Mo |
| `series_by_key` + `series_meta` | ~300 octets × 200 séries max | ~60 Ko |
| **Total théorique** | | **~1,2 Go** |

**Projection réaliste (calibrée Mai 2026)** :

| Mesure | Valeur |
|---|---|
| VM actuel (`du -sh /mnt/nvme/victoria-metrics`) | **48 Mo** |
| Croissance estimée (à confirmer par 2ᵉ point) | ~10 Mo / an |
| Projection VM 5 ans | **~100–150 Mo** |
| Ratio densité redb vs VM observé sur d'autres workloads | ×0,8 à ×1,2 |
| **Projection redb 5 ans** | **~80–180 Mo** |

> SQLite (plan v2) reste plus volumineux par ligne, projection ~400–600 Mo
> sur la même base réelle. **L'argument "redb est 5× plus compact" tient
> en pourcentage mais en valeur absolue on parle de quelques dizaines de
> Mo** — sur un NVMe de 256 Go, ce n'est plus un critère décisionnel.

L'arbitrage redb vs SQLite se fait donc sur les **autres axes** : compilation
(redb gagne — pas de dep C), maturité (SQLite gagne), outillage ops (SQLite
gagne — `sqlite3` CLI). Cf. §15 pour le récap.

### 4.4 Configuration du `Database`

```rust
use redb::{Builder, Database};

let db = Builder::new()
    .set_cache_size(64 * 1024 * 1024)           // 64 MiB de cache pages
    .create(&db_path)?;                          // ouvre ou crée
let db = Arc::new(db);

// Initialisation : créer les tables si elles n'existent pas
{
    let tx = db.begin_write()?;
    let _  = tx.open_table(TABLE_SERIES_BY_KEY)?;
    let _  = tx.open_table(TABLE_SERIES_META)?;
    let _  = tx.open_table(TABLE_META)?;
    let _  = tx.open_table(TABLE_RAW)?;
    let _  = tx.open_table(TABLE_HOURLY)?;
    let _  = tx.open_table(TABLE_DAILY)?;
    tx.commit()?;
}
```

Il n'y a **pas de PRAGMA** à équivaloir : `redb` n'expose pas de paramètres
de durabilité (toujours ACID, fsync à chaque `commit()`). Pour réduire le coût
fsync sur le chemin chaud, on **batche** les transactions (cf. §5.3).

---

## 5. Nouvelle crate `metrics-store` (backend redb)

### 5.1 Squelette

```bash
cargo new --lib crates/metrics-store
```

`Cargo.toml` :
```toml
[package]
name = "metrics-store"
version = "0.1.0"
edition = "2021"

[dependencies]
redb       = "2.2"
anyhow     = { workspace = true }
tokio      = { workspace = true, features = ["sync", "rt", "macros"] }
tracing    = { workspace = true }
serde      = { workspace = true, features = ["derive"] }
bincode    = "1.3"
smallvec   = "1"
parking_lot= "0.12"
```

Workspace `Cargo.toml` ajoute `crates/metrics-store` dans `members` (identique
à plan v2 §4.1).

### 5.2 API publique (identique de l'extérieur au plan v2)

```rust
##[derive(Clone)]
pub struct MetricsStore { db: Arc<redb::Database>, /* ... */ }

impl MetricsStore {
    pub fn open(db_path: &Path, opts: Options) -> anyhow::Result<Self>;
    pub fn writer(&self) -> Writer;
    pub fn reader(&self) -> Reader;                 // clonable, lit via begin_read()
    pub fn spawn_maintenance(&self, tier: TierPolicy) -> JoinHandle<()>;
}

pub struct Sample {
    pub metric: String,
    pub labels: SmallVec<[(String,String);4]>,
    pub ts_ms:  i64,
    pub value:  f64,
}
```

**Différence vs plan v2** : pas besoin de `reader_pool_size` — chaque appel à
`db.begin_read()` produit un snapshot lock-free, instantané et illimité. Pas
de pool à dimensionner.

### 5.3 Writer (chemin chaud) — critique pour redb

C'est ici qu'on doit être prudent avec redb : chaque `commit()` déclenche un
fsync. Si on commitait par ligne (12 lignes/s), ce serait 12 fsync/s ≈ trop
de I/O sur NVMe. Solution : **batch obligatoire**.

```rust
pub struct Writer { tx: tokio::sync::mpsc::Sender<Sample> }

// Tâche interne (spawn_blocking, une instance pour tout le process)
fn writer_loop(db: Arc<Database>, mut rx: mpsc::Receiver<Sample>,
               batch_max: usize, flush_ms: u64) -> anyhow::Result<()> {
    let mut series_cache: HashMap<(String,String), u32> = HashMap::new();
    let mut next_id: u32 = load_next_id(&db)?;

    loop {
        let batch = drain_blocking(&mut rx, batch_max, flush_ms);
        if batch.is_empty() { continue; }

        let wtx = db.begin_write()?;
        {
            let mut t_raw   = wtx.open_table(TABLE_RAW)?;
            let mut t_skey  = wtx.open_table(TABLE_SERIES_BY_KEY)?;
            let mut t_smeta = wtx.open_table(TABLE_SERIES_META)?;
            let mut t_meta  = wtx.open_table(TABLE_META)?;

            for s in batch {
                let labels_json = canonical_json(&s.labels);
                let key = (s.metric.clone(), labels_json.clone());
                let series_id = *series_cache.entry(key).or_insert_with(|| {
                    let lookup_key = make_skey(&s.metric, &labels_json);
                    if let Ok(Some(g)) = t_skey.get(&lookup_key[..]) {
                        g.value()
                    } else {
                        let id = next_id;
                        next_id += 1;
                        t_skey.insert(&lookup_key[..], id).unwrap();
                        let meta = bincode::serialize(&SeriesMeta{
                            metric: s.metric.clone(),
                            labels_json: labels_json.clone(),
                            first_seen: s.ts_ms,
                            last_seen:  s.ts_ms,
                        }).unwrap();
                        t_smeta.insert(id, &meta[..]).unwrap();
                        id
                    }
                });
                let k = enc_skey(series_id, s.ts_ms);
                t_raw.insert(&k[..], s.value)?;
            }
            t_meta.insert("next_series_id", next_id as u64)?;
        }
        wtx.commit()?;                  // 1 fsync = 1 batch
    }
}
```

**Tuning du batch**: `flush_ms = 250 ms`, `batch_max = 500` → 4 fsync/s en
régime nominal, soit moins que SQLite WAL en synchronous=NORMAL (qui fsync
au checkpoint, ~tous les 1000 pages soit ~4 Mo). Sur NVMe c'est négligeable
(< 1 ms par fsync).

### 5.4 Reader (snapshot MVCC, lock-free)

```rust
impl Reader {
    pub fn query_range(&self, series_id: u32, from_ms: i64, to_ms: i64,
                       table: Tier) -> anyhow::Result<Vec<(i64, f64)>> {
        let rtx = self.db.begin_read()?;             // snapshot instantané
        let table = match tier {
            Tier::Raw    => rtx.open_table(TABLE_RAW)?,
            Tier::Hourly => rtx.open_table(TABLE_HOURLY)?,
            Tier::Daily  => rtx.open_table(TABLE_DAILY)?,
        };
        let k_lo = enc_skey(series_id, from_ms);
        let k_hi = enc_skey(series_id, to_ms);
        let mut out = Vec::new();
        for entry in table.range(&k_lo[..] ..= &k_hi[..])? {
            let (k, v) = entry?;
            let (_, ts) = dec_skey(k.value());
            out.push((ts, v.value()));
        }
        Ok(out)
    }
}
```

C'est **2× plus court** que l'équivalent SQLite `prepare_cached + query_map`,
et sans allocation intermédiaire. La performance est dominée par le seek
B-tree (O(log n)) + la copie du résultat.

### 5.5 Agrégats en Rust (remplace le SQL `AVG/MIN/MAX/SUM`)

```rust
pub fn agg_over_range(reader: &Reader, series_id: u32,
                      from: i64, to: i64, op: AggOp) -> f64 {
    let pts = reader.query_range(series_id, from, to, Tier::Raw)?;
    match op {
        AggOp::Avg => pts.iter().map(|p| p.1).sum::<f64>() / pts.len() as f64,
        AggOp::Min => pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min),
        AggOp::Max => pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max),
        AggOp::Sum => pts.iter().map(|p| p.1).sum(),
    }
}
```

Le coût mémoire pour 1 jour = 86 400 / 5 s × 1 série × 16 octets = ~280 Ko.
Pour 5 ans en raw c'est ~500 Mo → on **doit** lire les tables hourly/daily
pour les longues plages (sélection automatique, cf. §6.3).

---

## 6. Compatibilité PromQL : transpileur AST → plan d'exécution

Identique à plan v2 §5 en intention, **mais la sortie change** : au lieu
d'émettre du SQL, on émet un **plan d'exécution Rust** (suite d'appels au
Reader + opérations d'agrégation streamées).

### 6.1 AST

```rust
enum Expr {
    Selector { metric: String, matchers: Vec<(String, MatchOp, String)> },
    Aggregate { op: AggOp, by: Vec<String>, expr: Box<Expr> },
    RangeFn { name: RangeFnName, expr: Box<Expr>, window_ms: i64 },
    BinOp { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },
    NumberLit(f64),
}
```

### 6.2 Exécution

Chaque variant compile en une fonction Rust :

| PromQL | Exécution redb |
|---|---|
| `metric` | `resolve_series_ids(metric, [])` → pour chacun : `reader.query_range(id, start, end, tier)` |
| `metric{l=v}` | `resolve_series_ids(metric, matchers)` puis idem |
| `avg(metric)` | union des séries → bucket par `step_ms` → moyenne |
| `avg_over_time(m[1h])` | scan + bucketing par fenêtre glissante (1 h = 720 points en raw) |
| `increase(m[1d])` | si `tier=raw` : `last_raw - first_raw` sur la fenêtre ; si compacté : `Σ (bucket.last - bucket.first)` |
| `max_over_time(m[1h])` | scan + max par fenêtre |
| `sum by (label)(m)` | resolve_series_ids → group_by `labels[label]` → somme alignée sur step |

### 6.3 Choix automatique de la table

Identique à plan v2 §5.3 :

| Plage | Table redb |
|---|---|
| ≤ 7 j | `TABLE_RAW` |
| 7 j – 90 j | `TABLE_HOURLY` |
| > 90 j | `TABLE_DAILY` |

### 6.4 Limites explicites du transpilateur

Comme en v2, on rejette explicitement les fonctions PromQL non utilisées
(`histogram_*`, `quantile`, `count_values`, sous-requêtes `[r:s]`, `offset`,
`@`, `label_replace`/`label_join`, set ops `and`/`or`/`unless`, vector matching
`on`/`ignoring`/`group_left`/`group_right`, etc.). L'erreur retournée a la forme
Prometheus :
```json
{"status":"error","error":"unsupported function: histogram_quantile",
 "errorType":"bad_data"}
```

### 6.5 Catalogue PromQL audité (golden set Mai 2026)

> Audit exhaustif des **81 expressions** présentes dans les 2 catalogues
> dashboard embarqués (`docs/grafana-ess_dashboard.json` 70 exprs +
> `docs/grafana-solar_pv_dashboard.json` 11 exprs). Le shim **doit
> couvrir 100 % de cette liste** avant la bascule Phase 3. Reproduire l'audit :
>
> ```bash
> python3 -c "
> import json, re
> for path in ['docs/grafana-ess_dashboard.json','docs/grafana-solar_pv_dashboard.json']:
>     d = json.load(open(path))
>     def w(ps):
>         for p in ps:
>             for t in p.get('targets',[]):
>                 e = t.get('expr','').strip()
>                 if e: print(path.split('/')[-1], p['id'], e)
>             if p.get('panels'): w(p['panels'])
>     w(d.get('panels',[]))
> "
> ```

**Fonctions PromQL utilisées** (7 au total) :

| Fonction | Occurrences | Plan d'exécution redb |
|---|---|---|
| `increase(m[w])` | la plupart des kWh | `last_raw - first_raw` sur fenêtre (raw) ou `Σ(bucket.last - bucket.first)` (compacté) |
| `avg_over_time(m[w])` | quelques cas (panel 43, …) | scan + moyenne par fenêtre glissante |
| `clamp_min(m, k)` / `clamp_max(m, k)` | panel 43 | wrapper trivial sur l'itérateur de valeurs |
| `abs(m)` | panel 43 | wrapper trivial |
| `max(...)` / `sum(...)` | quelques agrégats instantanés | aggregator existant §5.5 |

**Fenêtres** : `[1h]`, `[24h]`, `[1d]`, `[30d]` uniquement. Aucune fenêtre
exotique (`[5m]`, `[7d]`, etc.) dans le golden set actuel — si un futur
panel en introduit, mettre à jour le test `golden_promql_coverage`.

**Opérateurs binaires** : `+`, `-`, `*`, `/`. Toujours entre :
- vecteur ⊗ scalaire (ex: `m / 1000`) — simple map sur les valeurs ;
- vecteur ⊗ vecteur **alignés** (ex: `(yield - exported) / yield`) — join
  par timestamp aligné sur `step_ms` puis op point-à-point.

**Comparaisons** (depuis la roadmap promql-compat Phase 3a) : `== != > < >=
<=`, en mode filtre (samples vrais conservés, valeur inchangée) ou avec le
modifier `bool` (1.0/0.0). `__name__` retiré, toute comparaison avec `NaN`
fausse. Toujours **pas** de set ops `unless`/`and`/`or`.

**Extensions roadmap promql-compat** (au-delà du golden set ci-dessus, cf.
`./metriques-promql-reference.md`) :
- **Groupement** `by (…)` / `without (…)` sur `sum max min avg count`
  (Phase 2).
- **`topk(k, vec)` / `bottomk(k, vec)`** (Phase 3b) — labels d'origine
  conservés, support optionnel de `by (…)`.
- **`irate(m[w])`** (Phase 3c) — taux instantané sur les 2 derniers points.
- **Math instant** (Phase 4) : `sqrt exp ln log2 log10 sgn clamp(v,min,max)`
  (en plus de `abs clamp_min clamp_max ceil floor round`). `round(v,to_nearest)`
  honore désormais le 2ᵉ argument (demi-vers-le-haut).
- **Manipulation de labels** (Phase 4) : `label_replace(v,dst,repl,src,regex)`
  (regex ancrée, expansion `$1`/`${name}`) et `label_join(v,dst,sep,src…)`.
  Seules fonctions à accepter des arguments string.
- **Prédiction / stats** (Phase 5, P2) : `deriv`, `predict_linear(v[w],T)`,
  `quantile_over_time(φ,v[w])`, `stddev_over_time`, `stdvar_over_time`.
- **Alerting / compteurs** (Phase 5, P3) : `absent(v)` / `absent_over_time(v[w])`
  (labels = matchers d'égalité du sélecteur), `changes(v[w])`, `resets(v[w])`.
- Sur tier compacté, `deriv`/`predict_linear`/`stddev`/`stdvar`/`quantile_over_time`
  opèrent sur les `avg` des buckets et `changes`/`resets` sur la séquence
  `first,last` (approximation documentée — exact sur tier raw, ≤ 7 j).

**Subqueries** : une seule occurrence — `[24h:1m]` dans le panel 43
(`grafana-ess_dashboard.json`, calcul du proxy de cyclage batterie) :

```promql
(abs(avg_over_time(clamp_min(venus_shunt_current_a,0)[24h:1m]))
 + abs(avg_over_time(clamp_max(venus_shunt_current_a,0)[24h:1m])))
 * 24 / 680 * 100
```

**Décision recommandée** : ne pas implémenter de support subquery générique.
Soit (a) réécrire le panel 43 en deux requêtes côté JS additionnées dans
ECharts, soit (b) ajouter un cas spécial `[Xh:Ym]` qui pré-bucketize en
mémoire avant la fonction `*_over_time`. La complexité de (b) (~1 j) est
disproportionnée pour un panel. **Retenir (a).**

> **✅ Implémenté (2026-05)** : le numérateur se simplifie exactement —
> `abs(avg(clamp_min(I,0))) + abs(avg(clamp_max(I,0)))` = `avg(clamp_min(I,0) −
> clamp_max(I,0))` = `avg(|I|)`. On émet donc la métrique dérivée
> `venus_shunt_current_abs = |I|` (cf. `write_venus_smartshunt`) et le panel 43
> devient `avg_over_time(venus_shunt_current_abs[24h]) * 24 / 680 * 100` —
> supporté nativement, sans subquery, exact (et plus précis que le
> sous-échantillonnage 1 min puisqu'on agrège tous les points bruts).

**Offset, label_replace, label_join, vector(), scalar()** : aucune
occurrence — rejetés sans implémentation.

**Métriques référencées** (25 noms distincts) — toutes produites par
`vm_client.rs` ou `energy-manager` aujourd'hui :

```
solar_total_w
dc_pv_power_w                 pvinv_power_w
venus_shunt_power_w           venus_shunt_current_a         venus_shunt_soc_percent
venus_mppt_power_w            venus_mppt_yield_today_kwh    venus_mppt_max_power_today_w
venus_inverter_ac_output_power_w   venus_inverter_voltage_v
venus_heatpump_power_w        venus_heatpump_temp_c         venus_temp_c
et112_power_w                 et112_voltage_v               et112_frequency_hz
et112_energy_import_wh        et112_energy_export_wh
bms_v                         bms_soc                       bms_current
bms_temp_max                  bms_cell_delta_mv             bms_id (label)
ats_a                         irradiance_w
```

**Test de non-régression à ajouter dans la crate `metrics-store`** :

```rust
// crates/metrics-store/tests/golden_promql.rs
##[test]
fn golden_promql_coverage() {
    // Parse les 2 dashboards JSON et vérifie que toutes les exprs sont
    // acceptées par le transpileur (compile, pas forcément exécutable
    // sans data). Échoue à l'ajout d'une expr non supportée.
    let ess = include_str!("../../../docs/grafana-ess_dashboard.json");
    let pv  = include_str!("../../../docs/grafana-solar_pv_dashboard.json");
    let mut failed = vec![];
    for src in [ess, pv] {
        for expr in extract_exprs(src) {
            if let Err(e) = transpile(&expr) {
                failed.push((expr.clone(), e.to_string()));
            }
        }
    }
    assert!(failed.is_empty(), "exprs non transpilables: {:#?}", failed);
}
```

Ce test fige la couverture et fait échouer la CI si quelqu'un ajoute une
fonction PromQL non encore supportée dans un dashboard JSON.

---

## 7. Endpoint d'ingestion (compat `energy-manager`)

Identique au plan v2 §6 :

```
POST /api/v1/metrics/ingest
Content-Type: text/plain
```

`metrics_store::prom_text::parse(body) -> Vec<Sample>` puis `writer.push_many`.

---

## 8. Grafana sans plugin natif — le point structurant

C'est la **seule différence opérationnelle majeure** vs le plan v2.

### 8.1 Datasource Grafana

```yaml
## contrib/grafana/provisioning/datasources/daly-prometheus.yaml
apiVersion: 1
datasources:
  - name: Daly Metrics
    type: prometheus
    access: proxy
    url: http://127.0.0.1:8080         # daly-bms-server, PAS un VM/SQLite plugin
    uid: daly-metrics
    isDefault: true
    jsonData:
      httpMethod: GET
      timeInterval: 60s
      queryTimeout: 30s
```

Grafana parle Prometheus → notre serveur expose `/api/v1/query`,
`/api/v1/query_range`, `/api/v1/labels` → ces endpoints invoquent le shim
PromQL→redb. **Aucun changement** côté dashboards `.json` existants.

### 8.2 Endpoints supplémentaires requis pour Grafana

Pour que Grafana fonctionne complètement, certains endpoints Prometheus sont
attendus en plus de ceux déjà présents :

| Endpoint | Statut actuel | À faire pour redb |
|---|---|---|
| `GET /api/v1/query` | OK | maintenir |
| `GET /api/v1/query_range` | OK | maintenir |
| `GET /api/v1/labels` | OK | maintenir |
| `GET /api/v1/label/<name>/values` | **manquant** | ajouter — scan `series_meta` |
| `GET /api/v1/series` | **manquant** | ajouter — scan `series_meta` filtré |
| `GET /api/v1/metadata` | utile | optionnel |
| `GET /-/healthy`, `GET /-/ready` | **manquant** | ajouter pour datasource health-check |

Effort : ~0,5 j pour les 4 endpoints manquants (tous des scans simples sur
`TABLE_SERIES_META` filtrés par label_matchers).

### 8.3 Pourquoi pas le plugin `frser-sqlite-datasource` ?

Parce qu'il **ne s'applique pas** : redb n'est pas SQLite, pas de driver
ODBC, pas de SQL. La seule façon raisonnable d'interroger redb depuis Grafana
est de passer par notre serveur Rust. Cette contrainte n'est pas un
inconvénient en soi (le serveur est déjà obligatoire), mais elle **élimine
la possibilité d'écrire des requêtes SQL ad hoc** dans Grafana pour des cas
complexes (CTE, jointures multi-séries). En contrepartie : on peut toujours
exposer un endpoint REST custom pour ces cas.

---

## 9. Tiering et maintenance

### 9.1 Compaction `raw` → `hourly`

```rust
fn compact_to_hourly(db: &Database, cutoff_ms: i64) -> anyhow::Result<()> {
    // 1. Lecture : itérer raw < cutoff_ms, grouper par (series_id, bucket_ms_hourly)
    let rtx = db.begin_read()?;
    let t_raw = rtx.open_table(TABLE_RAW)?;
    let mut buckets: HashMap<(u32, i64), AggBucketBuilder> = HashMap::new();

    for entry in t_raw.iter()? {                            // O(N) sur les raws
        let (k, v) = entry?;
        let (sid, ts) = dec_skey(k.value());
        if ts >= cutoff_ms { continue; }
        let bucket_ms = (ts / 3_600_000) * 3_600_000;
        buckets.entry((sid, bucket_ms))
               .or_default()
               .accumulate(v.value(), ts);
    }
    drop(rtx);

    // 2. Écriture en une transaction unique
    let wtx = db.begin_write()?;
    {
        let mut t_hour = wtx.open_table(TABLE_HOURLY)?;
        for ((sid, b), builder) in buckets {
            let agg = builder.finalize();
            let key = enc_skey(sid, b);
            t_hour.insert(&key[..], &bincode::serialize(&agg)?[..])?;
        }
    }
    wtx.commit()?;

    // 3. Purge des raws compactés (transaction séparée pour borner la durée)
    let wtx2 = db.begin_write()?;
    {
        let mut t_raw = wtx2.open_table(TABLE_RAW)?;
        let k_hi = enc_max_skey(cutoff_ms);
        let _ = t_raw.drain(.. &k_hi[..])?;
    }
    wtx2.commit()?;
    Ok(())
}
```

Coût mémoire : 80 séries × 720 heures sur 30 j = ~57 600 buckets en RAM ≈ 3 Mo.
Acceptable. Si on voulait borner strictement la RAM, on streame en chunks
de 1 série à la fois.

### 9.2 `redb::Database::compact()` — équivalent VACUUM

Après les purges, le fichier `metrics.redb` contient des pages libres
(B-tree COW). Pour récupérer l'espace disque :

```rust
db.compact()?;                       // équivalent VACUUM SQLite
```

À exécuter **mensuellement, hors heure de pointe** (3 h du matin), en
**bloquant les écritures** pendant la durée du compact (~30 s sur 1 Go).
Le serveur doit donc soit :
- (a) faire une pause d'écriture (writer ignore les samples pendant le
  compact, avec backlog dans le canal — risque de buffer plein) ;
- (b) faire le compact à un moment où l'activité est minimale (nuit, pas
  d'irradiance, peu de variations BMS) ;
- (c) ne pas faire de compact périodique tant que la taille reste raisonnable
  (< 5 Go par exemple).

Recommandation : **(c)** par défaut, déclencher manuellement via un endpoint
admin `POST /api/v1/admin/compact` quand nécessaire (audit trimestriel).

### 9.3 Pas de checkpoint à faire

Contrairement à SQLite WAL, redb n'a pas de fichier `-wal` à truncate.
Chaque `commit()` finalise une page racine COW, l'ancienne devient libre.
**Suppression** de l'étape `PRAGMA wal_checkpoint(TRUNCATE)` du plan v2.

---

## 10. Plan de bascule en 4 phases

Identique à plan v2 §8 en structure, durées équivalentes. Différences détail :

### Phase 0 — préparation

| # | Tâche | Différence vs SQLite |
|---|---|---|
| 0.3 | `cargo new --lib crates/metrics-store` | identique |
| 0.4 | `tables.rs` + `encoding.rs` + `writer.rs` | un peu plus de code (encodage manuel des clés) |
| 0.5 | `reader.rs` + `tiering.rs` | plus simple côté reader (snapshots MVCC, pas de pool) ; plus de code pour les agrégats (en Rust) |
| 0.6 | `promql.rs` — parser identique, mais **émetteur de plan Rust** au lieu de SQL | +1–2 h vs SQLite (les agrégats sont à coder) |
| 0.7 | `prom_text.rs` | identique |
| 0.8 | Bench `criterion` : insertion + range scan | **comparer aux mêmes benchs SQLite** pour pouvoir trancher |

Estimation : ~22 h (vs 23 h SQLite — équivalent, le surplus PromQL est
compensé par l'absence de gestion pool/PRAGMA).

### Phase 1 — dual-write

Identique. `redb` peut très bien recevoir les samples en parallèle de VM
sans interférer.

### Phase 2 — lectures sur redb

| # | Tâche | Différence vs SQLite |
|---|---|---|
| 2.1 | Handler `query_range` via shim PromQL→redb | code différent, complexité équivalente |
| 2.4 | Comparaison curl côte à côte | identique, **plus important encore** car pas d'outil tiers pour valider |

### Phase 3 — bascule Grafana

| # | Tâche | Différence vs SQLite |
|---|---|---|
| 3.1 | Plugin Grafana | **SAUTÉ** — pas de plugin redb, on garde la datasource Prometheus pointée sur notre API |
| 3.2 | Provisioning datasource | mise à jour de l'URL (point vers `127.0.0.1:8080` au lieu de `:8428`) |
| 3.3 | Réécrire `pv-solar-5y.json` | **NON** — il reste en PromQL, le shim s'en charge |
| 3.4 | Endpoints Prometheus complémentaires | **AJOUT** — `/api/v1/label/<name>/values`, `/api/v1/series`, `/-/healthy` (cf. §8.2) |

L'effort Grafana est **plus faible** qu'en option SQLite, car on ne réécrit
aucun dashboard. C'est le gain principal de cette option.

### Phase 4 — retrait VictoriaMetrics

Identique au plan v2 §8 phase 4.

---

## 11. Outillage ops (point d'attention spécifique redb)

Pas d'équivalent de `sqlite3 metrics.db "SELECT …"`. Il faut ajouter un binaire :

```bash
## crates/metrics-store/src/bin/metrics-cli.rs
metrics-cli --db /mnt/nvme/daly-bms/metrics.redb count                # total points
metrics-cli --db ... list-series [--metric bms_voltage]               # series_meta dump
metrics-cli --db ... query --metric bms_soc --label bms_id=0x01       # range 1h
metrics-cli --db ... compact                                          # appel db.compact()
metrics-cli --db ... verify                                           # cohérence index/raw
metrics-cli --db ... export-csv --from T1 --to T2 > out.csv           # export plat
```

Effort initial : ~1 jour. Le binaire est inclus dans le `Makefile` (cible
`make metrics-cli`).

Bonus : `metrics-cli verify` lance `Database::check_integrity()` (méthode
fournie par redb) — pratique pour auditer la santé après crash.

---

## 12. Risques détaillés et mitigations

| # | Risque | Probabilité | Impact | Mitigation |
|---|---|---|---|---|
| R1 | Maturité redb (single-maintainer cberner) | Moyen | Bugs non corrigés en cas d'absence | Pinner la version (`redb = "=2.2.x"`), CI hebdomadaire de smoke test, garder snapshot SQLite en parallèle pendant 6 mois |
| R2 | Format de fichier breaking change (3.x) | Faible | Migration nécessaire | redb publie un script de migration entre majeures. Prévoir un fallback : `metrics-cli export-csv` permet de reconstruire la base à partir de zéro |
| R3 | Pas d'outil tiers pour debug en prod | Élevé | Difficulté ops | `metrics-cli` (§11) ; tests `verify` automatiques après crash |
| R4 | Agrégats en Rust : régression de précision vs PromQL | Moyen | Valeurs différentes vs VM | Tests par échantillon en Phase 2 (comparaison curl) sur 10–20 PromQL réels |
| R5 | Fsync par batch trop fréquent → I/O élevée | Faible | Wear NVMe | Tuning batch (250 ms / 500 lignes) → 4 fsync/s ≈ négligeable sur NVMe (mais à mesurer) |
| R6 | `Database::compact()` bloque les écritures | Moyen | 30 s sans collecte | Backlog mpsc dimensionné à 10 000 samples (~13 min de prod) ; déclenchement manuel hors heure |
| R7 | Pas de plugin Grafana → endpoints Prometheus manquants cassent un panel | Élevé | Panel HS | Implémenter `/api/v1/label/<n>/values` et `/api/v1/series` dès Phase 3.1 |
| R8 | Cardinalité explose : ~10 000 séries hypothétiques → `HashMap` cache trop gros | Faible | RSS qui gonfle | `series_cache` borné à 500 entrées avec LRU |
| R9 | Pas de migration in-place depuis VM (export-import obligatoire) | Moyen | 2 j de boulot script | Option : ne pas migrer l'historique, garder tar VM (cf. plan v2 §9) |
| R10 | Bug d'encodage clé big-endian / ordre | Faible | Range scan retourne mauvais points | Tests unitaires sur `enc_skey`/`dec_skey` (round-trip + ordre), property-based test (proptest) |

---

## 13. Checklist finale

Identique à plan v2 §12 sauf :
- [ ] **Pas** de fichier `-wal` (vérifier qu'aucun `.wal` n'apparaît à côté du `.redb`)
- [ ] Endpoints Prometheus complémentaires implémentés et testés via `curl`
- [ ] Binaire `metrics-cli` packagé et déployable (`make install-metrics-cli`)
- [ ] `db.check_integrity()` retourne `Ok` après 24 h d'usage
- [ ] Taille fichier `metrics.redb` cohérente avec projection (~1,2 Go à 5 ans)

---

## 14. Estimation RAM et empreinte disque

| Service | Avant (VM) | Après (SQLite v2) | Après (redb) |
|---|---|---|---|
| `daly-bms-server` | ~27 Mo | ~30–35 Mo | **~28–32 Mo** (pas de cache rusqlite supplémentaire) |
| `victoria-metrics` | ~120–150 Mo | 0 | 0 |
| `energy-manager` | ~25 Mo | ~25 Mo | ~25 Mo |
| `grafana-server` | ~80 Mo | ~80 Mo | ~80 Mo |
| **Total RSS** | **~252 Mo** | **~135 Mo** (−117 Mo) | **~133 Mo** (−119 Mo) |
| Empreinte fichier données (5 ans, projection théorique) | ~3 Go (VM) | ~5–8 Go (SQLite) | ~1,2 Go (redb) |
| **Empreinte fichier données (5 ans, recalibrée Mai 2026 ⇒ cf §4.3)** | **~100–150 Mo (VM)** | **~400–600 Mo** | **~80–180 Mo** |
| Binaire daly-bms-server | baseline | +1,5 Mo (libsqlite3 statique) | **+0,25 Mo (redb crate)** |

> **Conclusion bench mise à jour** : sur ce projet, redb reste ~3–5× plus
> dense sur disque que SQLite et ~1 Mo plus léger en binaire. La RAM est
> quasi-identique. **Mais en valeur absolue, sur la base réelle mesurée
> (48 Mo aujourd'hui), l'écart disque devient anecdotique** — le gain RAM
> de ~120 Mo (suppression de `victoria-metrics`) est désormais le seul
> argument quantitativement fort, indépendamment du backend choisi.

---

## 15. Comparaison finale : SQLite (v2) vs redb (ce document) **Decision: redb .**

| Axe | SQLite v2 | redb | Verdict |
|---|---|---|---|
| Effort dev total | ~35 h | ~37 h | quasi-équivalent (+2 h redb pour agrégats Rust) |
| Validation Grafana | Réécriture optionnelle dashboards SQL ou shim | Shim obligatoire | redb impose le shim → **mais cela force un design propre** |
| Outils ops | `sqlite3` CLI gratuit + DataGrip | `metrics-cli` à écrire (1 j) | SQLite gagne pour le debug ad hoc |
| Stabilité format | inégalable (25 ans) | jeune mais stable depuis v1.0 | SQLite |
| Empreinte disque | 5–8 Go | **1,2 Go** | redb gagne nettement |
| Binaire ajouté | +1,5 Mo | **+0,25 Mo** | redb |
| RAM | équivalent | équivalent | nul |
| Compilation | dep C (cross-compile aarch64 lourd) | **100 % Rust** | redb |
| Évolution schéma | `ALTER TABLE` flexible | Migration de code | SQLite gagne |
| Communauté / écosystème | massive | restreinte | SQLite |
| Performance scans temporels | très bonne | **excellente** (clé = index naturel) | redb |
| Performance écritures batchées | très bonne (WAL) | très bonne (1 fsync/batch) | équivalent |

### Recommandation par profil

- **Si l'équipe valorise la facilité d'ops, le debug avec des outils standards,
  et la possibilité de requêtes SQL ad hoc** → **SQLite (plan v2)**.
- **Si l'équipe valorise la pureté Rust, le poids minimal sur disque et binaire,
  et accepte d'investir dans un `metrics-cli` maison** → **redb (ce plan)**.     **Decision: redb .**

Dans le contexte exact de Daly-BMS-Rust (Pi5 4 Go, NVMe 256 Go, équipe
réduite, peu d'analyses SQL ad hoc — toutes les requêtes passent déjà par
l'API du serveur), **redb est le choix techniquement supérieur** :

- gain disque 4–7× (sur un NVMe non saturé mais utile pour les backups),
- aucune compilation C dans le pipeline ARM,
- design forcé via shim PromQL → propreté du code,
- pas de moins-value Grafana puisque le frontend principal est `/dashboard/*`
  (Askama, déjà servi par le serveur Rust),
- pas de plugin Grafana tiers à maintenir.

Le seul vrai sacrifice est l'absence de `sqlite3` CLI pour le debug ad hoc.
Le binaire `metrics-cli` (~1 jour de dev) compense largement.

---

## 16. Décisions encore ouvertes

1. **Coexistence `redb` + `rusqlite`** : on garde `rusqlite` pour `alerts.db`
   et `dashboard_storage` ? Recommandation : oui — ce sont des données
   relationnelles avec jointures occasionnelles, leur cas d'usage est différent.**Decision: OUI .**
2. **Version `redb`** : pin strict `=2.2.x` ou range `^2.2` ? Recommandation :
   pin strict en prod, range en dev. **Decision: pin strict en prod .**
3. **Migration historique** : import VM → redb via le script Python du plan
   v2 §9, ou simple archive `tar` de `/mnt/nvme/victoria-metrics` ? Même
   compromis que v2. **Decision: migration si possible .**
4. **Outil `metrics-cli` exposé en lecture seule** ou avec opérations
   destructives (compact, purge) ? Recommandation : lecture seule par défaut,
   flag `--admin` pour les écritures. **Decision: suivre la recommendation .**
5. **Panel 43 (subquery `[24h:1m]`)** : réécrire en deux requêtes côté JS
   ou supporter la subquery dans le transpileur ? Cf. §6.5 — recommandation
   = réécriture (~30 min) vs implémentation générique (~1 j).**Decision: suivre la recommendation .**
6. **Solar PV `increase` à large fenêtre** (`[30d]`) : assurer qu'à 5s de
   step raw, un range scan sur 30 j de raw (~518 400 points/série) reste
   sous 100 ms. Sinon : forcer le tier `daily` même pour `[30d]`. À
   benchmarker dès la Phase 2. **Decision: suivre la recommendation .**

---

*Document complémentaire à `plan_migration_vm_sqlite_v2.md`. Les deux plans
sont alternatifs (pas additifs). Le choix se fait avant Phase 0.*

---

## 17. Changelog du document

| Date | Auteur | Changement |
|---|---|---|
| Mai 2026 | initial | Rédaction complète §1–§16 (commit `e4b8360`) |
| Mai 2026 | session Solar PV | Ajout §0 (état & démarrage), §6.5 (catalogue PromQL audité), §16-5/6 (décisions Solar PV) suite aux commits `08f23c7` (dashboard Solar PV) et `d9580fd` (onglets dashboard) qui élargissent la surface PromQL à transpiler. Aucun changement structurel sur §1–§15. |
| 17 mai 2026 | calibrage Pi5 prod | Ajout §0.1.1/0.1.2/0.1.3 (chiffres réels mesurés : 48 Mo VM, 277 séries dont 92 internes potentiellement purgeables, service systemd à identifier). Recalibrage §4.3 et §14 : projection 5 ans descend de 1,2 Go théorique à 80–180 Mo réaliste. Conséquence majeure : le gain disque redb cesse d'être un argument décisif (~quelques dizaines de Mo en valeur absolue) ; reste fort le gain RAM (−120 Mo) et la suppression de la dep C cross-compile aarch64. |
| 17 mai 2026 | décisions actées | §0.4 : redb retenu (pure Rust), migration historique = import script (volume tractable), purge label `pid` avant Phase 1. §0.1.2 enrichie d'un audit code complet (4 producteurs / 0 consommateur des métriques top-process, root-cause cardinalité = label `pid` éphémère sur compilateurs Rust). §0.1.3 finalisée : service identifié `victoriametrics.service` (binaire `victoria-metrics-prod`, RSS 135 Mo, retention 5y). `CLAUDE.md` §0 mis à jour avec les commandes VM. |

```

---

> _Note : l'annexe historique ci-dessus est reproduite **verbatim** depuis le document
> d'origine, qui contenait un bloc de code non fermé ; la fence ci-dessus le clôt pour
> préserver le rendu du reste du document. Aucun contenu n'a été retiré._

## Sources consolidées

Ce document fusionne et **remplace** les anciens fichiers suivants :
`docs/architecture-redb.md`, `docs/plan_migration_vm_redb.md`, `docs/timeseries.md`.
