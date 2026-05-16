# Plan de migration : VictoriaMetrics → SQLite (v2 — détaillé)

> **Projet** : Daly-BMS-Rust
> **Objectif** : Remplacer VictoriaMetrics (~120–150 Mo RSS) par un fichier SQLite unique
> stocké sur le NVMe `/mnt/nvme`, en conservant 5 ans d'historique, la compatibilité
> Grafana, et **toute l'API REST PromQL exposée** (`/api/v1/query`, `/api/v1/query_range`,
> `/api/v1/labels` + les endpoints `chart/*`, `history/*`, `dashboards/panel/:id/data`).
> **Date** : Mai 2026
> **Auteur** : analyse approfondie du code existant (`crates/daly-bms-server`,
> `crates/energy-manager`, `contrib/`, `docs/`).

> Ce document remplace `plan_migration_vm_sqlite.md` (v1). La v1 était correcte
> dans les grandes lignes mais ignorait plusieurs réalités du projet :
> - **deux** producteurs écrivent vers VM (`daly-bms-server` ET `energy-manager`) ;
> - chaque catégorie a son **timer de throttle indépendant** (12 AtomicU64) ;
> - les métriques portent **2 à 4 labels** (`bms_id`, `address`, `name`, `instance`,
>   `phase`, `source`, `cell`, `ch`, `idx`, `window`, `process`, `pid`, `type`…)
>   qu'un schéma `(device, metric)` simple ne couvre pas ;
> - le binaire embarque `docs/grafana-ess_dashboard.json` via `include_str!` et
>   exécute ces requêtes PromQL telles quelles (`api/dashboards.rs`) ;
> - `history.rs` utilise `avg_over_time`, `increase`, `max_over_time` qu'il faudra
>   transposer en SQL côté serveur (ou conserver un mini-évaluateur PromQL).

---

## 1. Cartographie de l'existant (résultat de l'audit)

### 1.1 Producteurs de métriques

| Producteur | Fichier | Catégories | Throttle | Volumétrie |
|---|---|---|---|---|
| `daly-bms-server` | `vm_client.rs` (501 lignes) | BMS, ET112, irradiance, SmartShunt, inverter, MPPT, température, heatpump, ATS, Tasmota, Shelly, monitor (Pi5) | 12 timers AtomicU64 (5 s/catégorie, sauf solar via endpoint REST) | ~12 écritures HTTP/s |
| `energy-manager` | `monitoring.rs`, `rule_metrics.rs`, `logic/water_heater/mod.rs`, `http_clients/lg_thinq.rs` | métriques système `em_*`, métriques tokio, état chauffe-eau, état LG ThinQ, statistiques règles `.grl` | 60 s pour le système, ad hoc pour les autres | ~1 écriture/min sur le chemin chaud + bursts |

> **Conséquence** : la migration doit fournir une API d'écriture exploitable
> depuis les deux crates. Soit on partage un binding (crate `metrics-store`),
> soit `energy-manager` continue d'écrire via HTTP vers un endpoint hébergé
> dans `daly-bms-server`. La seconde option est plus simple à déployer (cf. §6).

### 1.2 Consommateurs

| Consommateur | Fichier | Forme des requêtes | Plage typique |
|---|---|---|---|
| Dashboard SSR `/chart/history` | `api/chart.rs` | PromQL bruts (`solar_total_w`, `avg(bms_soc)`, `et112_power_w{address="0x08"}`) | 1 min → 12 h, step 60 s–10 min |
| Dashboard SSR `/chart/edge-history` | `api/chart.rs` | PromQL + matcheurs `{bms_id="0x01"}` / `{address="0x09"}` | 1 min → 24 h |
| Dashboard SSR `/history/energy?period=day\|week\|month\|year` | `api/history.rs` | `avg_over_time(...[1h])`, `increase(...[30d])`, `max_over_time(...)` | 1 jour → 1 an |
| `/api/v1/query`, `/api/v1/query_range`, `/api/v1/labels` | `api/promql.rs` | proxy PromQL transparent | quelconque |
| `/api/v1/dashboards/panel/:id/data` | `api/dashboards.rs` | requêtes PromQL extraites de `grafana-ess_dashboard.json` (incluses dans le binaire) | imposé par Grafana (variables `${__from}`) |
| Grafana (service systemd, plugin Prometheus) | `contrib/grafana/provisioning/datasources/victoriametrics.yaml` | PromQL natif | 5 ans max |

### 1.3 Schéma actuel (Prometheus text format, ligne par ligne)

Exemple pour un BMS (`vm_client.rs:170`) :
```
bms_voltage{bms_id="0x01"} 52.84 1715900000000
bms_cell_voltage{bms_id="0x01",cell="cell_03"} 3.302 1715900000000
```

Listing exhaustif des métriques + labels — extrait pour le schéma cible :

| Métrique | Labels | Source | Fréquence (après throttle) |
|---|---|---|---|
| `bms_voltage`, `bms_current`, `bms_power`, `bms_soc`, `bms_capacity_ah`, `bms_cell_delta_mv`, `bms_temp_max`, `bms_temp_min`, `bms_charge_mos`, `bms_discharge_mos` | `bms_id` | `VmClient::bms_rows` | 5 s |
| `bms_cell_voltage` | `bms_id`, `cell` | idem | 5 s × 16 cellules × 2 BMS = 32 lignes |
| `et112_voltage_v`, `_current_a`, `_power_w`, `_apparent_power_va`, `_power_factor`, `_frequency_hz`, `_energy_import_wh`, `_energy_export_wh` | `address`, `name` | `et112_rows` | 5 s × 3 = 24 lignes |
| `irradiance_wm2` | `address` | `irradiance_rows` | 5 s |
| `venus_shunt_*` (8 métriques) | aucun | `smartshunt_rows` | 5 s |
| `venus_inverter_*` (7 métriques) | aucun | `inverter_rows` | 5 s |
| `venus_mppt_power_w`, `_pv_voltage_v`, `_dc_current_a`, `_yield_today_kwh`, `_max_power_today_w` | `instance`, `name` | `mppt_rows` | 5 s |
| `venus_temp_c`, `venus_humidity_percent` | `instance`, `name`, `type` | `temperature_rows` | 5 s |
| `venus_heatpump_state`, `_power_w`, `_energy_kwh`, `_temp_c`, `_target_temp_c` | `idx` | `heatpump_rows` | 5 s |
| `ats_sw1_closed`, `ats_sw2_closed`, `ats_active_source`, `ats_voltage_v`, `ats_freq_hz` | `source`, `phase` (selon le cas) | `ats_rows` | 5 s |
| `tasmota_power_on`, `_power_w`, `_voltage_v`, `_current_a`, `_energy_today_kwh` | `id`, `name` | `tasmota_rows` | 5 s |
| `shelly_output`, `shelly_power_w`, `_voltage_v`, `_current_a`, `_energy_wh` | `id`, `name`, `ch` | `shelly_rows` | 5 s × 2 canaux |
| `solar_total_w`, `dc_pv_power_w`, `pvinv_power_w`, `mppt_power_w`, `solar_yield_kwh` | aucun | `solar_rows` (POST /solar/mppt-yield) | 5 s |
| `pi5_cpu_percent`, `pi5_memory_percent`, `pi5_disk_percent`, `pi5_uptime_secs`, `pi5_mem_used_mb`, `pi5_swap_used_mb`, `pi5_net_rx_bps`, `pi5_net_tx_bps`, `pi5_load_avg`, `pi5_cpu_temp_c`, `pi5_disk_usage_*` | `window` (load_avg), divers | `monitor.rs:build_monitor_vm_rows` | 60 s |
| `em_cpu_percent`, `em_memory_percent`, `em_mem_used_mb`, `em_swap_used_mb`, `em_disk_percent`, `em_net_*`, `em_load_avg`, `em_cpu_temp_c`, `em_process_cpu_percent`, `em_process_mem_mb`, `em_tokio_task_*` | `window`, `process`, `pid`, `task` | `energy-manager/monitoring.rs` | 60 s |
| métriques chauffe-eau / LG ThinQ / rules (.grl) | divers | `energy-manager` (HTTP direct) | événementiel |

**Estimation cardinalité** : ~80 séries actives × ~12 points/min ≈ **86 400 points/h**,
soit ~750 millions de points en 5 ans **avant tiering**. Avec tiering 30 j raw + 1 an hourly + 5 ans daily, on tombe à **~120 millions de lignes** total.

### 1.4 Code à modifier (chemins exacts)

```
crates/daly-bms-server/
  src/
    main.rs                 ← VM init (lignes 282–297) + state init (300)
    config.rs               ← VmConfig (lignes 413–435)
    state.rs                ← AppState.vm + 12 timers throttle (lignes 449–557)
                              + 13 sites d'écriture vm.write_rows (lignes 643–1091)
    vm_client.rs            ← À supprimer après migration (501 lignes)
    monitor.rs              ← build_monitor_vm_rows + écritures lignes 427–509
    api/
      promql.rs             ← Remplacer le proxy par un évaluateur SQL
      chart.rs              ← Récrire 2 endpoints (lignes 33–105 et 108–200)
      history.rs            ← Récrire 1 endpoint (160 lignes)
      dashboards.rs         ← Adapter exec PromQL panel (lignes 77–179)
      system.rs             ← Endpoint /solar/mppt-yield (lignes 151–200)
    dashboards/
      mod.rs                ← Panel.queries.expr reste en PromQL côté JSON,
                              mais l'exécution passe par le shim PromQL→SQL
      grafana.rs            ← Conserver (parser dashboard JSON)

crates/energy-manager/
  src/
    monitoring.rs           ← Remplacer write_to_vm (ligne 125) par push vers
                              daly-bms-server /api/v1/metrics/ingest
    rule_metrics.rs         ← idem
    logic/water_heater/mod.rs ← idem
    http_clients/lg_thinq.rs ← idem (ligne 245)
    config.rs               ← Renommer vm_url → ingest_url ou metrics_url

contrib/
  victoriametrics.service   ← À retirer après bascule
  victoriametrics-scrape.yml← À retirer
  grafana/provisioning/datasources/victoriametrics.yaml ← Remplacer par SQLite

docs/
  Tuning-Victoria.md         ← Archiver (note d'historique)
  victoriametrics-queries.md ← Conserver comme référence des PromQL d'origine
  grafana-ess_dashboard.json ← Mettre à jour avec datasource sqlite OU
                                conserver tel quel si on garde le shim PromQL→SQL

Config.toml                  ← Remplacer [victoriametrics] par [metrics_store]
Cargo.toml                   ← rusqlite déjà en workspace (v0.31 + bundled)
```

---

## 2. Architecture cible

```
┌────────────────────────────────────────────────────────────────────────┐
│                       Raspberry Pi 5 CM (4 Go)                          │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │  daly-bms-server (Rust, axum)                                    │   │
│  │    ├── RS485 polling (state.rs)                                  │   │
│  │    ├── crate metrics-store (nouveau)                             │   │
│  │    │     ├── MetricsDb : pool de connexions rusqlite             │   │
│  │    │     ├── Writer task : mpsc + spawn_blocking → INSERT batché │   │
│  │    │     ├── Reader pool : N=4 connexions readonly mmap          │   │
│  │    │     └── PromQL→SQL transpiler (sous-ensemble)               │   │
│  │    ├── /api/v1/metrics/ingest (POST Prometheus text) ◄───────────┼───┐│
│  │    ├── /api/v1/query, /query_range, /labels (compat Prometheus)  │  ││
│  │    └── /chart/*, /history/*, /dashboards/panel/:id/data          │  ││
│  └──────────────────────────────────────────────────────────────────┘  ││
│                              │                                          ││
│                              ▼                                          ││
│  ┌──────────────────────────────────────────────────────────────────┐  ││
│  │  /mnt/nvme/daly-bms/metrics.db (WAL mode)                        │  ││
│  │    ├── series         (table de hachage label-set → series_id)   │  ││
│  │    ├── metrics_raw    (30 j, granularité originale)              │  ││
│  │    ├── metrics_hourly (1 an, AVG/MIN/MAX/COUNT/SUM)              │  ││
│  │    └── metrics_daily  (5 ans, AVG/MIN/MAX/COUNT/SUM)             │  ││
│  └──────────────────────────────────────────────────────────────────┘  ││
│                              ▲                                          ││
│                              │ readonly                                 ││
│  ┌──────────────────────────────────────────────────────────────────┐  ││
│  │  Grafana (systemd) + plugin frser-sqlite-datasource              │  ││
│  └──────────────────────────────────────────────────────────────────┘  ││
│                                                                          ││
│  ┌──────────────────────────────────────────────────────────────────┐  ││
│  │  energy-manager (Rust) — HTTP push texte Prometheus ─────────────┼──┘│
│  └──────────────────────────────────────────────────────────────────┘   │
└────────────────────────────────────────────────────────────────────────┘
```

**Décision clé** : `daly-bms-server` reste le **seul process écrivain** sur SQLite.
`energy-manager` continue de pousser au format Prometheus text via HTTP local
(`127.0.0.1:8081/api/v1/metrics/ingest` ou similaire), exactement comme aujourd'hui
vers VM. Cela évite tout problème de SQLite multi-writer et tout couplage de code.

---

## 3. Schéma SQLite cible

### 3.1 Pragmas (à appliquer à chaque ouverture)

```sql
PRAGMA journal_mode = WAL;            -- multi-lecteurs + 1 écrivain
PRAGMA synchronous = NORMAL;          -- fsync sur checkpoint uniquement
PRAGMA temp_store = MEMORY;
PRAGMA cache_size = -65536;           -- 64 MiB en cache page
PRAGMA mmap_size = 268435456;         -- 256 MiB de mmap (lectures O(1))
PRAGMA wal_autocheckpoint = 1000;     -- checkpoint auto à 1000 pages
PRAGMA busy_timeout = 5000;           -- 5 s pour attendre un lock
PRAGMA foreign_keys = ON;             -- pour la table series
```

### 3.2 Table `series` (normalisation des label-sets)

Toutes les combinaisons (`metric`, labels) reçoivent un `series_id` stable.
Cela évite de stocker des chaînes pour chaque ligne et permet des jointures
efficaces. Le label-set est sérialisé en **JSON canonique** (clés triées),
ce qui permet à SQLite de l'indexer avec un index unique.

```sql
CREATE TABLE IF NOT EXISTS series (
    series_id   INTEGER PRIMARY KEY,                       -- auto-incrément
    metric      TEXT NOT NULL,                             -- ex: "bms_voltage"
    labels_json TEXT NOT NULL DEFAULT '{}',                -- ex: '{"bms_id":"0x01"}'
    UNIQUE (metric, labels_json)
);

CREATE INDEX IF NOT EXISTS idx_series_metric ON series(metric);
```

> **Pourquoi pas `(device, metric)` comme dans le plan v1 ?**
> Le code actuel a des métriques avec 0, 1, 2 ou 3 labels (`bms_cell_voltage`
> = 2 labels, `ats_voltage_v` = 2 labels, `shelly_*` = 3 labels, `solar_*` = 0
> label). Une colonne `device` arbitraire ne couvre pas ce besoin et casse la
> compatibilité avec les PromQL existants (`et112_power_w{address="0x09"}` ne
> peut pas se réduire à `device="et112-grid"`).

### 3.3 Tables de points

Trois tables (une par granularité) pour exploiter `WITHOUT ROWID` et bénéficier
d'un index clustered serré sur `(series_id, ts)`.

```sql
CREATE TABLE IF NOT EXISTS metrics_raw (
    series_id INTEGER NOT NULL,
    ts        INTEGER NOT NULL,                            -- epoch ms (i64)
    value     REAL    NOT NULL,
    PRIMARY KEY (series_id, ts),
    FOREIGN KEY (series_id) REFERENCES series(series_id)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS metrics_hourly (
    series_id INTEGER NOT NULL,
    ts        INTEGER NOT NULL,                            -- début d'heure UTC, ms
    avg_val   REAL NOT NULL,
    min_val   REAL NOT NULL,
    max_val   REAL NOT NULL,
    sum_val   REAL NOT NULL,                               -- pour increase()
    cnt       INTEGER NOT NULL,
    PRIMARY KEY (series_id, ts),
    FOREIGN KEY (series_id) REFERENCES series(series_id)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS metrics_daily (
    series_id INTEGER NOT NULL,
    ts        INTEGER NOT NULL,                            -- 00:00 UTC, ms
    avg_val   REAL NOT NULL,
    min_val   REAL NOT NULL,
    max_val   REAL NOT NULL,
    sum_val   REAL NOT NULL,
    cnt       INTEGER NOT NULL,
    PRIMARY KEY (series_id, ts),
    FOREIGN KEY (series_id) REFERENCES series(series_id)
) WITHOUT ROWID;
```

> **`ts` en millisecondes** (et non secondes comme v1) : aligné sur les
> timestamps déjà produits dans `vm_client.rs` (`timestamp_millis()`) et sur
> la convention du frontend (`ts_ms`). On évite ainsi une conversion sur
> chaque point écrit.

> **`sum_val` ajouté** : `history.rs` utilise `increase(et112_energy_*[30d])`
> qui se traduit par `last - first` sur la fenêtre. Sans `sum_val`, on perdrait
> la possibilité de calculer `increase` correctement après compaction. En
> pratique on stocke aussi `first_val` et `last_val` (voir §5.4).

### 3.4 Index supplémentaires

`(series_id, ts)` est déjà l'index PK clustered. Ajouter un index par `ts` pour
les requêtes transverses « toutes séries entre T1 et T2 » :

```sql
CREATE INDEX IF NOT EXISTS idx_raw_ts    ON metrics_raw(ts);
CREATE INDEX IF NOT EXISTS idx_hourly_ts ON metrics_hourly(ts);
CREATE INDEX IF NOT EXISTS idx_daily_ts  ON metrics_daily(ts);
```

### 3.5 Volumétrie projetée (avec tiering)

| Niveau | Lignes / jour | Sur la rétention |
|---|---|---|
| `metrics_raw` (≈80 séries × 12/min × 1440 min) | 1,38 M | 30 j → 41 M lignes |
| `metrics_hourly` (80 × 24) | 1 920 | 365 j → 700 k lignes |
| `metrics_daily` (80 × 1) | 80 | 5 ans → 146 k lignes |
| **Total** | | **~42 M lignes, ~5–8 Go** |

Beaucoup moins que la projection v1 grâce à la normalisation `series` (qui économise
30–40 % par rapport à du label en clair répété sur chaque ligne).

---

## 4. Nouvelle crate `metrics-store`

Plutôt que de poser un fichier `db_client.rs` à côté de `vm_client.rs`, créer
une **crate workspace** réutilisable. Cela permet d'isoler les dépendances
SQLite et de tester indépendamment.

### 4.1 Création

```bash
cargo new --lib crates/metrics-store
```

Ajout au `Cargo.toml` racine :

```toml
[workspace]
members = [
    "crates/daly-bms-core",
    "crates/daly-bms-server",
    "crates/dbus-mqtt-venus",
    "crates/energy-manager",
    "crates/metrics-store",        # nouveau
    "crates/rs485-bus",
]
```

`crates/metrics-store/Cargo.toml` :

```toml
[package]
name = "metrics-store"
version = "0.1.0"
edition = "2021"

[dependencies]
rusqlite     = { workspace = true }
anyhow       = { workspace = true }
tokio        = { workspace = true, features = ["sync", "rt", "macros"] }
tracing      = { workspace = true }
serde        = { workspace = true, features = ["derive"] }
serde_json   = { workspace = true }
chrono       = { workspace = true }
parking_lot  = "0.12"               # locks rapides pour le cache series_id
```

### 4.2 API publique

```rust
// crates/metrics-store/src/lib.rs

pub mod schema;     // CREATE TABLE / migrations
pub mod writer;     // Writer (mpsc + spawn_blocking + batch)
pub mod reader;     // pool readonly + helpers query_range / query_instant
pub mod tiering;    // compaction et purge périodique
pub mod promql;     // mini-parser PromQL → AST → SQL
pub mod prom_text;  // parsing du format Prometheus text (POST ingest)

/// Point d'entrée principal — clonable, partage le pool en interne.
#[derive(Clone)]
pub struct MetricsStore { /* ... */ }

impl MetricsStore {
    pub fn open(db_path: &std::path::Path, opts: Options) -> anyhow::Result<Self>;
    pub fn writer(&self) -> Writer;
    pub fn reader(&self) -> Reader;
    pub fn spawn_maintenance(&self, tier: TierPolicy) -> tokio::task::JoinHandle<()>;
}

pub struct Options {
    pub cache_mib:        i64,                  // défaut 64
    pub mmap_mib:         i64,                  // défaut 256
    pub reader_pool_size: usize,                // défaut 4
    pub writer_batch_max: usize,                // défaut 500 lignes
    pub writer_flush_ms:  u64,                  // défaut 250 ms
}

pub struct TierPolicy {
    pub raw_retention_days:    i64,             // défaut 30
    pub hourly_retention_days: i64,             // défaut 365
    pub daily_retention_days:  i64,             // défaut 5 * 365
    pub interval_secs:         u64,             // défaut 3600
}
```

### 4.3 Writer (chemin chaud)

Le code actuel appelle `vm.write_rows(rows).await` ~12 fois par seconde. On
conserve la même signature de surface mais on factorise via un canal MPSC :

```rust
pub struct Writer {
    tx: tokio::sync::mpsc::Sender<Sample>,
}

pub struct Sample {
    pub metric: String,
    pub labels: smallvec::SmallVec<[(String, String); 4]>,
    pub ts_ms:  i64,
    pub value:  f64,
}

impl Writer {
    pub async fn push(&self, sample: Sample) { let _ = self.tx.send(sample).await; }
    pub async fn push_many(&self, samples: Vec<Sample>) {
        for s in samples { let _ = self.tx.send(s).await; }
    }
}
```

Le **task interne** consomme le canal, agrège jusqu'à `writer_batch_max` lignes
ou `writer_flush_ms` ms, puis exécute via `tokio::task::spawn_blocking` :

```rust
let conn = Connection::open(db_path)?;     // une seule connexion d'écriture
conn.execute_batch(&PRAGMAS)?;
let mut series_cache = HashMap::<(String,String), i64>::new();  // metric+labels_json → id

loop {
    let batch = drain(&mut rx, batch_max, flush_ms).await;
    if batch.is_empty() { continue; }

    let tx = conn.unchecked_transaction()?;
    let mut stmt_series_get = conn.prepare_cached(
        "SELECT series_id FROM series WHERE metric=?1 AND labels_json=?2")?;
    let mut stmt_series_ins = conn.prepare_cached(
        "INSERT INTO series(metric, labels_json) VALUES(?1, ?2)
         ON CONFLICT(metric, labels_json) DO UPDATE SET metric=metric
         RETURNING series_id")?;
    let mut stmt_insert = conn.prepare_cached(
        "INSERT OR REPLACE INTO metrics_raw(series_id, ts, value) VALUES(?1,?2,?3)")?;

    for s in batch {
        let labels_json = canonical_json(&s.labels);
        let id = series_cache.entry((s.metric.clone(), labels_json.clone()))
            .or_insert_with(|| /* SELECT puis INSERT RETURNING */);
        stmt_insert.execute(params![*id, s.ts_ms, s.value])?;
    }
    tx.commit()?;
}
```

Points-clés :

- **Une seule connexion d'écriture** sur tout le process → pas de verrouillage WAL.
- **Cache `series_id` en mémoire** (HashMap) → évite un `SELECT` à chaque ligne.
- **`ON CONFLICT DO UPDATE … RETURNING`** : nécessite SQLite ≥ 3.35 (Pi5 a 3.40+).
- **`spawn_blocking`** : SQLite est synchrone. Le batch de 250 ms moyen permet
  d'avoir 0,1 ms par ligne, soit < 25 ms de CPU bloquant tous les 250 ms.

### 4.4 Reader (pool readonly)

Pour les lectures concurrentes (Grafana via plugin, dashboard SSR, API
`/api/v1/query_range`) on utilise un **pool de connexions ouvertes en
read-only avec mmap actif** :

```rust
fn open_reader(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.execute_batch(PRAGMAS_READONLY)?;
    Ok(conn)
}
```

Avec WAL mode actif, les lectures **ne bloquent jamais** l'écrivain. Le pool
est dimensionné à `reader_pool_size = 4` par défaut (ajustable selon CPU).

---

## 5. Compatibilité PromQL (le point critique)

C'est ici que le plan v1 sous-estimait la complexité. Voici ce qu'il faut faire.

### 5.1 Inventaire exhaustif des PromQL en service

À l'audit, les usages observés sont **limités** à un sous-ensemble très précis :

| Pattern PromQL | Cas d'usage |
|---|---|
| `metric` simple | chart.rs (`solar_total_w`), dashboards |
| `metric{label="val"}` | chart.rs (`et112_power_w{address="0x08"}`), edge-history |
| `metric{label="val",l2="v2"}` | dashboards Grafana (panels avec instance + name) |
| `avg(metric)` | chart.rs (`avg(bms_soc)`) |
| `avg_over_time(metric{...}[1h])` | history.rs (toutes périodes) |
| `increase(metric{...}[1d])` | history.rs (énergies cumulées) |
| `max_over_time(metric[1h])` | history.rs (shunt Ah) |
| `sum by (label)(metric)` | grafana-ess_dashboard.json (quelques panels) |
| `rate(metric[1h])`, opérateurs arithmétiques | grafana-ess_dashboard.json (quelques) |

Les fonctions et opérateurs PromQL **non utilisés** (histograms, quantiles,
`label_replace`, sous-requêtes, etc.) seront **explicitement rejetés**.

### 5.2 Stratégie : transpilateur PromQL → SQL

Créer un module `metrics_store::promql` qui parse un sous-ensemble PromQL puis
émet une requête SQL équivalente. L'AST minimal :

```rust
enum Expr {
    Selector { metric: String, matchers: Vec<(String, MatchOp, String)> },
    Aggregate { op: AggOp, by: Vec<String>, expr: Box<Expr> },          // sum/avg/min/max
    RangeFn  { name: RangeFnName, expr: Box<Expr>, window_ms: i64 },    // avg_over_time, max_over_time, increase, rate
    BinOp    { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },             // +, -, *, /
    NumberLit(f64),
}
```

Transpilation par cas (sortie SQL paramétrée, exécutée sur la table appropriée
selon la granularité choisie automatiquement — voir §5.3) :

```text
metric                          → SELECT ts, value FROM metrics_raw
                                  WHERE series_id IN (… filtre matchers …)
                                  AND ts BETWEEN ?start AND ?end

metric{l=v}                     → idem + filtre via série pré-résolue

avg(metric)                     → SELECT ts, AVG(value) FROM metrics_raw …
                                  WHERE ts BETWEEN … GROUP BY ts/step

avg_over_time(m[1h])            → SELECT bucket_start, AVG(value)
                                  FROM (granularité optimale)
                                  WHERE … GROUP BY bucket_start
                                  (où bucket_start = ts - (ts MOD step_ms))

increase(m[1d])                 → SELECT bucket, MAX(value)-MIN(value)
                                  (ou last_val-first_val si compacté)

max_over_time(m[1h])            → SELECT bucket, MAX(value) FROM … GROUP BY …

sum by (label) (m)              → SELECT label_val, ts, SUM(value)
                                  FROM metrics_raw JOIN series USING(series_id)
                                  ... GROUP BY json_extract(labels_json, '$.label'), ts
```

### 5.3 Choix automatique de la granularité

La fonction de transpilation reçoit `(start_ms, end_ms, step_ms)` et choisit la
table optimale :

| Plage | Table |
|---|---|
| `end - start ≤ 7 j` | `metrics_raw` |
| `7 j < end - start ≤ 90 j` | `metrics_hourly` |
| `end - start > 90 j` | `metrics_daily` |

Avec override possible via un commentaire PromQL `# tier=daily` (utile pour les
dashboards "5 ans" qui peuvent rester sur `daily` même en zoom).

### 5.4 Préservation de `increase()` après compaction

Pour que `increase(et112_energy_import_wh[30d])` reste exact après que les points
raw ont été purgés, la compaction `hourly` et `daily` doit conserver
**`first_val`** (valeur à l'heure pleine) et **`last_val`** (dernier point) en
plus de `min/max/avg/sum`. Ajout au schéma :

```sql
ALTER TABLE metrics_hourly ADD COLUMN first_val REAL;
ALTER TABLE metrics_hourly ADD COLUMN last_val  REAL;
ALTER TABLE metrics_daily  ADD COLUMN first_val REAL;
ALTER TABLE metrics_daily  ADD COLUMN last_val  REAL;
```

`increase` se calcule alors comme `SUM(last_val - first_val)` sur la fenêtre,
ce qui est ce que `increase()` PromQL fait avec correction de reset.

### 5.5 Endpoint `/api/v1/query_range` : compatibilité de surface

Le shim doit retourner exactement la forme JSON Prometheus :

```json
{ "status":"success",
  "data":{"resultType":"matrix",
           "result":[{"metric":{"__name__":"bms_soc","bms_id":"0x01"},
                      "values":[[1715900000, "92.4"], …]}]}}
```

Ceci préserve la compatibilité avec `extract_series` dans `api/chart.rs`,
`api/history.rs`, `api/dashboards.rs`, et avec Grafana plugin Prometheus si on
décidait de le garder en transition (cf. §8 phase 2).

---

## 6. Endpoint d'ingestion HTTP (compat `energy-manager`)

Plutôt que de coupler `energy-manager` à la crate `metrics-store`, on expose
dans `daly-bms-server` un endpoint compatible Prometheus text format :

```
POST /api/v1/metrics/ingest
Content-Type: text/plain
Body: lignes Prometheus exposition format
```

Implémentation : `metrics_store::prom_text::parse(body) -> Vec<Sample>` puis
`writer.push_many(samples)`. C'est trivial et offre **zéro changement** dans le
chemin chaud de `energy-manager` (juste l'URL change).

`energy-manager/src/config.rs` :
```toml
[water_heater]
vm_url = "http://127.0.0.1:8080/api/v1/metrics/ingest"   # ← changement
```

Idem pour `[lg_thinq].vm_url` et l'`vm_url` partagé dans `monitoring.rs`.

---

## 7. Tiering automatique (révisé)

Le job de maintenance tourne dans `daly-bms-server` via
`tokio::time::interval(Duration::from_secs(3600))`. Différences vs plan v1 :

### 7.1 Compaction `raw` → `hourly`

```sql
INSERT INTO metrics_hourly
    (series_id, ts, avg_val, min_val, max_val, sum_val, cnt, first_val, last_val)
SELECT
    series_id,
    (ts / 3600000) * 3600000      AS bucket_ts,            -- début d'heure ms
    AVG(value),
    MIN(value),
    MAX(value),
    SUM(value),
    COUNT(*),
    -- first/last via fenêtre : SQLite ≥ 3.25 a window functions
    (SELECT value FROM metrics_raw r2
      WHERE r2.series_id=r.series_id
        AND r2.ts >= bucket_ts AND r2.ts < bucket_ts + 3600000
      ORDER BY r2.ts LIMIT 1),
    (SELECT value FROM metrics_raw r2
      WHERE r2.series_id=r.series_id
        AND r2.ts >= bucket_ts AND r2.ts < bucket_ts + 3600000
      ORDER BY r2.ts DESC LIMIT 1)
FROM metrics_raw r
WHERE ts < (strftime('%s','now','-30 days') * 1000)
  AND ts >= COALESCE((SELECT MAX(ts) FROM metrics_hourly), 0) + 3600000
GROUP BY series_id, bucket_ts
ON CONFLICT(series_id, ts) DO NOTHING;
```

Versions plus efficaces possibles avec `FIRST_VALUE()/LAST_VALUE()` en CTE.

### 7.2 Compaction `hourly` → `daily`

Symétrique, sur `(ts / 86_400_000) * 86_400_000`.

### 7.3 Purges

```sql
DELETE FROM metrics_raw    WHERE ts < strftime('%s','now','-30 days')*1000;
DELETE FROM metrics_hourly WHERE ts < strftime('%s','now','-1 year')*1000;
-- pas de purge daily (rétention = 5 y par config)
```

### 7.4 Maintenance fichier

- `PRAGMA wal_checkpoint(TRUNCATE);` à la fin de chaque maintenance.
- `PRAGMA optimize;` une fois par jour.
- `VACUUM;` une fois par mois, **hors heure de pointe** (3 h du matin), avec
  garde-fou : si `metrics.db` > 15 Go, sauter le VACUUM (il faudrait 2× cette taille de libre).

---

## 8. Plan de bascule en 4 phases (vs v1 monolithique)

### Phase 0 — préparation (sans impact production)

| # | Tâche | Fichier / Action | Durée |
|---|---|---|---|
| 0.1 | Vérifier état NVMe et permissions | `df -h /mnt/nvme`, `id pi5compute` | 10 min |
| 0.2 | Créer `/mnt/nvme/daly-bms/` + groupe `daly-metrics` | voir §10 | 15 min |
| 0.3 | Créer la crate `metrics-store` (skeleton compilable + tests) | `crates/metrics-store/` | 1 h |
| 0.4 | Implémenter `schema.rs` + `writer.rs` + tests d'insertion | idem | 4 h |
| 0.5 | Implémenter `reader.rs` + `tiering.rs` + tests sur DB pré-remplie | idem | 4 h |
| 0.6 | Implémenter `promql.rs` (parser + transpiler) + tests par cas | idem | 8 h |
| 0.7 | Implémenter `prom_text.rs` + tests round-trip | idem | 2 h |
| 0.8 | Bench rapide : 86 400 insertions/h, lecture range 1 an | `criterion` | 2 h |

Livrable : crate `metrics-store` testée, **aucun changement** dans le serveur.

### Phase 1 — dual-write (production sans risque)

| # | Tâche | Fichier | Durée |
|---|---|---|---|
| 1.1 | Ajouter `[metrics_store]` à `Config.toml` (à côté de `[victoriametrics]`) | Config.toml | 5 min |
| 1.2 | Étendre `AppState` avec `pub store: Option<MetricsStore>` | `state.rs:449` | 30 min |
| 1.3 | Sur chaque site `vm.write_rows(rows).await`, faire **aussi** `store.writer().push_many(samples).await` (12 sites) | `state.rs:643..1091`, `monitor.rs`, `system.rs` | 3 h |
| 1.4 | Adapter `VmRow → Sample` (helper de conversion `From<VmRow>` dans `vm_client.rs`) | `vm_client.rs` | 1 h |
| 1.5 | Exposer `/api/v1/metrics/ingest` (sans déconnecter VM côté energy-manager) | nouveau handler dans `api/system.rs` ou `api/metrics.rs` | 1 h |
| 1.6 | Déployer sur Pi5 : `make build-arm && make sync` puis copie binaire | — | 30 min |
| 1.7 | Observer 48 h : taille `metrics.db`, RSS, `journalctl -u daly-bms` | — | 2 jours |

Critère d'acceptation : `SELECT COUNT(*) FROM metrics_raw` croît de façon
stable (~86 400 lignes/h en somme), aucune erreur d'écriture, RSS ne dépasse
pas +20 Mo.

### Phase 2 — lectures sur SQLite (toujours dual-write)

| # | Tâche | Fichier | Durée |
|---|---|---|---|
| 2.1 | Implémenter le handler `query_range` côté SQLite via le shim PromQL→SQL | `api/promql.rs` (nouveau code parallèle) | 4 h |
| 2.2 | Ajouter un flag `read_from = "sqlite" \| "vm"` dans `[metrics_store]` | `config.rs` | 10 min |
| 2.3 | Faire pointer `chart.rs`, `history.rs`, `dashboards.rs` sur le bon backend | 3 fichiers | 2 h |
| 2.4 | Tester chaque endpoint à comparaison fonctionnelle (curl côte à côte) | scripts/`compare_vm_sqlite.sh` (nouveau) | 4 h |
| 2.5 | Migration des PromQL réellement utilisées : valider la transpilation case par case sur le dashboard | `docs/promql-cases.md` (nouveau) | 4 h |

À la fin de cette phase, le frontend lit déjà depuis SQLite, VM est toujours en
écriture pour assurance.

### Phase 3 — bascule Grafana

| # | Tâche | Fichier | Durée |
|---|---|---|---|
| 3.1 | Installer plugin `grafana-cli plugins install frser-sqlite-datasource` | shell sur Pi5 | 5 min |
| 3.2 | Provisioning datasource SQLite | `contrib/grafana/provisioning/datasources/sqlite.yaml` (nouveau) | 30 min |
| 3.3 | Retranscrire `contrib/grafana/dashboards/pv-solar-5y.json` en SQL | un dashboard à la fois | 4 h |
| 3.4 | Retranscrire `docs/grafana-ess_dashboard.json` (utilisé par le catalog Rust) — **OU** garder le PromQL et compter sur le shim côté API | décision | 1 h (option shim) à 8 h (réécriture) |

Recommandation : **garder le PromQL côté JSON** et utiliser le shim API
(`/api/v1/query_range`) — pas de réécriture côté Grafana, sauf si on veut
profiter de SQL natif pour des cas complexes (CTE, fenêtres). Le plugin
`frser-sqlite-datasource` n'est utilisé que pour les dashboards Grafana
nouveaux ou réécrits.

### Phase 4 — retrait VictoriaMetrics

| # | Tâche | Fichier | Durée |
|---|---|---|---|
| 4.1 | Passer `[victoriametrics].enabled = false` dans Config.toml | Config.toml | 1 min |
| 4.2 | Vérifier 24 h : tout fonctionne sans VM en écriture | — | 1 j |
| 4.3 | `sudo systemctl stop victoria-metrics && sudo systemctl disable victoria-metrics` | shell sur Pi5 | 1 min |
| 4.4 | Supprimer `crates/daly-bms-server/src/vm_client.rs` | — | 5 min |
| 4.5 | Supprimer toutes les références : `state.vm`, `vm_last_*_write`, `VmConfig`, init dans `main.rs` | grep + Edit | 1 h |
| 4.6 | Bascule `energy-manager` : `vm_url` → `metrics_url` (ingest endpoint) | `energy-manager/src/config.rs`, callers | 1 h |
| 4.7 | Supprimer `contrib/victoriametrics.service`, `contrib/victoriametrics-scrape.yml`, `contrib/grafana/provisioning/datasources/victoriametrics.yaml` | — | 5 min |
| 4.8 | Mettre à jour `Makefile` (retirer cibles `vm-*` si présentes), `CLAUDE.md` (ligne sur services), `Readme.md` (architecture, RAM) | 3 fichiers | 1 h |
| 4.9 | Archiver `docs/Tuning-Victoria.md` → `docs/archive/` et ajouter en-tête « OBSOLETE » | — | 5 min |
| 4.10 | `sudo apt remove victoria-metrics` (ou suppression manuelle binaire) | shell | 5 min |
| 4.11 | `sudo rm -rf /mnt/nvme/victoria-metrics` (après backup ultime) | shell | 5 min |

**Total dév** : ~35 h sur 3 semaines.
**Total validation production** : 5 jours dont 48 h dual-write + 24 h sans VM.

---

## 9. Migration des données historiques (optionnel)

L'audit `docs/Tuning-Victoria.md` indique ~50 séries actives avec une rétention
5 ans déjà entamée. Si l'on veut garder l'historique :

### 9.1 Export PromQL → JSON par série

Script (à créer dans `scripts/migrate_vm_to_sqlite.py`) :

```python
# Pseudo-code
metrics = list_via_GET("/api/v1/labels")
for m in metrics:
    series = list_via_GET(f"/api/v1/series?match[]={m}")
    for s in series:
        for chunk_start, chunk_end in monthly_chunks(now - 5y, now):
            data = GET(f"/api/v1/query_range?query={s}&start=...&step=60s")
            sqlite_insert("metrics_raw", data)
```

### 9.2 Compaction post-import

Après import, exécuter manuellement la procédure de tiering pour produire
`metrics_hourly` et `metrics_daily` :

```bash
curl -X POST http://127.0.0.1:8080/api/v1/admin/tiering/run-once
```

(endpoint protégé à ajouter dans `api/system.rs`, derrière l'api_key existante).

### 9.3 Décision

Pour ce projet, **archiver simplement le snapshot VictoriaMetrics** (`tar` de
`/mnt/nvme/victoria-metrics`) suffit probablement : les usages quotidiens
regardent ≤ 1 mois, et l'historique très long peut être retrouvé via le tar
en cas de besoin légal/diagnostic.

---

## 10. Permissions et déploiement

### 10.1 Création du répertoire NVMe

```bash
sudo mkdir -p /mnt/nvme/daly-bms
sudo groupadd -f daly-metrics
sudo usermod -aG daly-metrics grafana || true     # user du service Grafana
sudo usermod -aG daly-metrics daly-bms || true    # user du service daly-bms (vérifier le user effectif)
sudo chown -R root:daly-metrics /mnt/nvme/daly-bms
sudo chmod 2775 /mnt/nvme/daly-bms                # sticky-group : héritage du groupe
```

Vérification du user du service :

```bash
systemctl show daly-bms.service -p User -p Group
```

> Si `User=` est vide (= root), basculer vers un user dédié n'est **pas**
> dans le scope de cette migration : conserver l'état actuel et ajuster le
> chmod pour `0o664` après création du fichier (cf. `db_client.rs` v1 §9).

### 10.2 Unité systemd

`contrib/daly-bms.service` (vérifier `After=mnt-nvme.mount` et
`Requires=mnt-nvme.mount` pour éviter un démarrage avant le mount NVMe).

### 10.3 Backup

```bash
# /etc/cron.daily/daly-metrics-backup
sqlite3 /mnt/nvme/daly-bms/metrics.db ".backup /mnt/nvme/backups/metrics.$(date +%F).db"
find /mnt/nvme/backups -name 'metrics.*.db' -mtime +30 -delete
```

`.backup` est atomique et sûr en WAL mode (pas besoin de stopper le serveur).
Pour un backup off-site, rsync de `/mnt/nvme/backups/` vers le NAS.

---

## 11. Risques détaillés et mitigations

| # | Risque | Probabilité | Impact | Mitigation |
|---|---|---|---|---|
| R1 | Le shim PromQL→SQL ne couvre pas un cas du dashboard Grafana embarqué (`grafana-ess_dashboard.json`) | Moyen | Panel cassé | Phase 0.6 : tests par cas extraits du JSON. Plan B : conserver VM en lecture seule jusqu'à parité 100 %. |
| R2 | `increase()` faux après compaction (pas de `first_val`/`last_val`) | Élevé si oublié | Tendances annuelles trompeuses | Stocker explicitement `first_val`/`last_val` dès la 1ʳᵉ compaction (cf. §5.4) |
| R3 | Plugin Grafana SQLite ne supporte pas certaines features | Moyen | Dashboards Grafana cassés | Garder le shim API + datasource Prometheus pointée sur `/api/v1/query_range` (notre endpoint) — pas besoin du plugin SQLite si on ne veut pas |
| R4 | Concurrence WAL : Grafana lit, écrivain bloqué sur checkpoint | Faible | Lag d'écriture | `wal_autocheckpoint=1000` + `busy_timeout=5000`. WAL mode est conçu pour ça. |
| R5 | Cardinalité explose si un label devient « par seconde » (bug) | Faible | DB qui gonfle | Garde-fou : tâche de maintenance vérifie `COUNT(DISTINCT series_id) > 200` → alerte log |
| R6 | Backup pendant écriture | Faible | Backup incohérent | `.backup` API SQLite (atomique). Jamais `cp` direct. |
| R7 | Migration `energy-manager` casse en cours de bascule | Moyen | métriques `em_*` manquantes 2 min | Déployer **après** Phase 1 validée. URL fallback : si `metrics_url` down, tomber sur VM en parallèle pendant la transition |
| R8 | Volume `raw` dépasse 30 j par défaillance maintenance | Faible | NVMe plein | Sonde Pi5 existante (`pi5_disk_percent`) déclenche déjà alerte > 80 %. Ajouter alerte `metrics_db_size_gb` |
| R9 | Une régression cassée n'est détectée qu'après le retrait VM (Phase 4) | Moyen | Rollback complexe | Phase 4.2 : 24 h d'observation. Ne **pas** supprimer le binaire VM en Phase 4.4 — laisser le service présent mais désactivé pendant 30 j. |
| R10 | Plage 5 ans très lente en SQL (LIMIT, ORDER BY sans index) | Moyen | Timeout dashboard | Index `(series_id, ts)` clustered + table `metrics_daily` séparée. Tests bench Phase 0.8 |

---

## 12. Checklist finale (post-migration)

- [ ] `/mnt/nvme/daly-bms/metrics.db` créé, WAL mode actif (`SELECT * FROM pragma_journal_mode`)
- [ ] `series` table contient ~80 entrées, distinctes par `(metric, labels_json)`
- [ ] `metrics_raw` reçoit ~86 400 lignes/h
- [ ] `metrics_hourly` produit par la maintenance toutes les heures (vérifier `MAX(ts)` glissant)
- [ ] `metrics_daily` produit chaque jour
- [ ] Endpoint `/api/v1/metrics/ingest` accepte le format Prometheus text
- [ ] `energy-manager` écrit sur l'endpoint local (vérifier dans `journalctl -u energy-manager`)
- [ ] `/api/v1/query`, `/api/v1/query_range`, `/api/v1/labels` répondent au format Prometheus JSON
- [ ] Dashboard SSR `/dashboard/history` affiche les mêmes valeurs qu'avant
- [ ] Tous les panels du dashboard Grafana embarqué fonctionnent (curl `/api/v1/dashboards/panel/:id/data` pour chaque id)
- [ ] Grafana datasource SQLite (ou shim Prometheus → notre API) configuré
- [ ] `victoriametrics.service` arrêté + disabled, données archivées
- [ ] `vm_client.rs` supprimé, plus aucun import résiduel
- [ ] `Config.toml` ne contient plus de section `[victoriametrics]`
- [ ] `Cargo.toml` ne contient plus aucune dépendance VM exclusive (reqwest reste pour d'autres usages)
- [ ] `Readme.md` et `CLAUDE.md` mis à jour (architecture, RAM, ports)
- [ ] Backup quotidien actif (`/etc/cron.daily/daly-metrics-backup`)
- [ ] Mesure RSS avant/après : objectif **−120 Mo** sur le système global

---

## 13. Estimation RAM finale

| Service | Avant (VM) | Après (SQLite) |
|---|---|---|
| `daly-bms-server` | ~27 Mo | ~30–35 Mo (writer + cache + 4 readers mmap) |
| `victoria-metrics` | **~120–150 Mo** | **0 Mo** (supprimé) |
| `energy-manager` | ~25 Mo | ~25 Mo (inchangé) |
| `grafana-server` | ~80 Mo | ~80 Mo (inchangé, plugin SQLite léger) |
| **Total** | ~252 Mo | **~135 Mo** (−117 Mo, soit ~46 %) |

Pi5 dispose de 4 Go → la marge n'était pas critique, mais on gagne en réactivité
sous charge mémoire (compilations, navigateurs locaux pour debug, etc.) et on
supprime un point de défaillance (process VM séparé).

---

## 14. Décisions encore ouvertes

À trancher avant Phase 0 :

1. **Crate dédiée `metrics-store` vs module dans `daly-bms-server`** — le plan
   recommande la crate dédiée pour testabilité, mais module suffit si on veut
   minimiser la complexité workspace.
2. **Format `labels_json` : JSON canonique vs MessagePack** — JSON pour la
   compat avec `json_extract()` SQLite, qui est natif et zéro-allocation.
3. **Choix granularité automatique vs explicite via PromQL** — le plan choisit
   automatique, mais un dashboard Grafana peut avoir besoin de forcer `raw` sur
   une plage de 3 mois (zoom diagnostic). Comment exprimer l'override ?
4. **Retrait Grafana ?** — Grafana consomme 80 Mo, soit 60 % de l'économie de
   cette migration. Si les besoins « historique long » sont couverts par
   `/dashboard/history`, on peut envisager une 2ᵉ migration suppression
   Grafana → notre dashboard SSR uniquement. Hors scope de ce plan.

---

*Document généré à partir d'un audit code intégral de `crates/daly-bms-server`
et `crates/energy-manager` (mai 2026). Remplace `plan_migration_vm_sqlite.md`
v1 — voir l'en-tête pour la liste des écarts.*
