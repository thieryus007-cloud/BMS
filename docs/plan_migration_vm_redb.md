# Plan de migration : VictoriaMetrics → redb (variante pure-Rust)

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

## 0. État d'avancement & démarrage de session

> Cette section est **maintenue à jour à chaque session de travail**. Elle sert
> de point d'entrée pour reprendre la migration sans avoir à relire les 800
> lignes qui suivent. **Toujours la mettre à jour avant de fermer une session.**

### 0.1 Statut actuel — Mai 2026

| Phase | Avancement |
|---|---|
| Plan rédigé et validé | ✅ document complet (15 sections + comparatif v2) |
| Décision SQLite vs redb | ⏸️ **non tranchée** — recommandation §15 = redb ; à confirmer avant Phase 0 |
| Audit PromQL exhaustif | ✅ cf. §6.5 ci-dessous (81 expressions, 7 fonctions, 1 subquery) |
| Crate `metrics-store` | ❌ pas démarrée |
| Endpoint `/api/v1/metrics/ingest` | ❌ pas démarré |
| Shim PromQL → redb | ❌ pas démarré |
| Dual-write VM+redb | ❌ pas démarré |
| Bascule Grafana | ❌ pas démarrée |
| Retrait VictoriaMetrics | ❌ pas démarré |
| **Volume actuel data dir** | `/mnt/nvme/victoria-metrics` (à mesurer en début de session : `du -sh`) |

### 0.2 Ce qui a changé dans le code depuis la rédaction (impacte le plan)

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

### 0.3 Kit de démarrage pour une nouvelle session

Quand tu ouvres une session pour cette migration, fais dans l'ordre :

```bash
# 1. Vérifier la branche active
git status && git branch --show-current

# 2. Lire ces fichiers DANS CET ORDRE
#    a. CLAUDE.md                              (contexte projet, ~3 min)
#    b. docs/plan_migration_vm_redb.md §0..§3  (état + archi, ~10 min)
#    c. docs/plan_migration_vm_redb.md §6.5    (audit PromQL exhaustif)
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

### 0.4 Décisions à prendre EN PREMIER lors de la prochaine session

1. **SQLite v2 ou redb ?** — relire §15 + §16 et trancher avant tout code.
   Tant que ce n'est pas tranché, la crate `metrics-store` ne peut pas
   commencer (interface publique identique mais implémentation différente).
2. **Migration historique** : import VM → nouveau backend, ou archive tar
   et démarrage à neuf ? Cf. §16-3. Recommandation tactique : **archive tar
   uniquement** — l'historique fin n'a de valeur que sur 30–90 j, on
   conserve un dump VM accessible *read-only* via un script ad hoc.
3. **Subquery panel 43** (§6.5) : la supporter dans le transpileur ou
   réécrire le panel en deux requêtes côté JS ? Recommandation : réécriture.

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

#[derive(serde::Serialize, serde::Deserialize)]
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

Identique au plan v2 §3.5 en lignes (~42 M points raw + 700 k hourly + 146 k
daily), mais en **bytes** :

| Table | Bytes / entrée | Total sur la rétention |
|---|---|---|
| `metrics_raw` | 12 (clé) + 8 (f64) + overhead B-tree ≈ 28 octets | 41 M × 28 = **~1,1 Go** |
| `metrics_hourly` | 12 + 40 + overhead ≈ 60 octets | 700 k × 60 = **42 Mo** |
| `metrics_daily` | 12 + 40 + overhead ≈ 60 octets | 146 k × 60 = **9 Mo** |
| `series_by_key` + `series_meta` | ~300 octets × 200 séries max | ~60 Ko |
| **Total** | | **~1,2 Go** |

> SQLite (plan v2) estimait 5–8 Go pour les mêmes lignes — la différence vient
> de l'overhead par ligne (SQLite stocke des entêtes B-tree + row IDs + types
> dynamiques, redb stocke des bytes nus). **redb est ~5× plus compact ici.**

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
#[derive(Clone)]
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
(`histogram_*`, `quantile`, `rate` sur compteurs non monotones, sous-requêtes,
`label_replace`, etc.). L'erreur retournée a la forme Prometheus :
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

**Pas** d'opérateur de comparaison (`>`, `<=`), pas de `bool` modifier,
pas de `unless`/`and`/`or` ensemblistes.

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
#[test]
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
# contrib/grafana/provisioning/datasources/daly-prometheus.yaml
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
# crates/metrics-store/src/bin/metrics-cli.rs
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
| Empreinte fichier données (5 ans) | ~3 Go (VM) | ~5–8 Go (SQLite) | **~1,2 Go (redb)** |
| Binaire daly-bms-server | baseline | +1,5 Mo (libsqlite3 statique) | **+0,25 Mo (redb crate)** |

> **Conclusion bench** : sur ce projet, redb est ~5× plus dense sur disque et
> ~1 Mo plus léger en binaire, pour une RAM quasi-identique à SQLite.

---

## 15. Comparaison finale : SQLite (v2) vs redb (ce document)

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
  et accepte d'investir dans un `metrics-cli` maison** → **redb (ce plan)**.

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
   relationnelles avec jointures occasionnelles, leur cas d'usage est différent.
2. **Version `redb`** : pin strict `=2.2.x` ou range `^2.2` ? Recommandation :
   pin strict en prod, range en dev.
3. **Migration historique** : import VM → redb via le script Python du plan
   v2 §9, ou simple archive `tar` de `/mnt/nvme/victoria-metrics` ? Même
   compromis que v2.
4. **Outil `metrics-cli` exposé en lecture seule** ou avec opérations
   destructives (compact, purge) ? Recommandation : lecture seule par défaut,
   flag `--admin` pour les écritures.
5. **Panel 43 (subquery `[24h:1m]`)** : réécrire en deux requêtes côté JS
   ou supporter la subquery dans le transpileur ? Cf. §6.5 — recommandation
   = réécriture (~30 min) vs implémentation générique (~1 j).
6. **Solar PV `increase` à large fenêtre** (`[30d]`) : assurer qu'à 5s de
   step raw, un range scan sur 30 j de raw (~518 400 points/série) reste
   sous 100 ms. Sinon : forcer le tier `daily` même pour `[30d]`. À
   benchmarker dès la Phase 2.

---

*Document complémentaire à `plan_migration_vm_sqlite_v2.md`. Les deux plans
sont alternatifs (pas additifs). Le choix se fait avant Phase 0.*

---

## 17. Changelog du document

| Date | Auteur | Changement |
|---|---|---|
| Mai 2026 | initial | Rédaction complète §1–§16 (commit `e4b8360`) |
| Mai 2026 | session Solar PV | Ajout §0 (état & démarrage), §6.5 (catalogue PromQL audité), §16-5/6 (décisions Solar PV) suite aux commits `08f23c7` (dashboard Solar PV) et `d9580fd` (onglets dashboard) qui élargissent la surface PromQL à transpiler. Aucun changement structurel sur §1–§15. |
