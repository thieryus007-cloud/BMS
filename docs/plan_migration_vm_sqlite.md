# Plan de migration : VictoriaMetrics → SQLite

> **Projet** : Daly-BMS-Rust  
> **Objectif** : Remplacer VictoriaMetrics (~150 Mo RAM) par SQLite (~3 Mo RAM) tout en conservant 5 ans d'historique et la compatibilité Grafana.  
> **Stockage** : NVMe 256 Go dédié (`/mnt/nvme`)  
> **Date** : Mai 2026

---

## 1. Contexte et architecture actuelle

### Stack existante
- `daly-bms-server` (Rust/Axum) : polling RS485, REST API, Dashboard SSR, bridges MQTT/VM
- `vm_client.rs` : client VictoriaMetrics (insertion + requête PromQL)
- VictoriaMetrics 2.x : TSDB Go, ~150–350 Mo RAM, rétention 5 ans
- Grafana (service systemd) : dashboards avec agrégations mensuelles/annuelles
- AlertEngine : déjà sur SQLite (`rusqlite`)

### Pourquoi SQLite ?
| Critère | VictoriaMetrics | SQLite |
|---|---|---|
| RAM | 150 Mo | ~3 Mo |
| Empreinte disque | Binaire Go + données | Fichier unique + index |
| Grafana | Plugin natif Prometheus | Plugin `frser-sqlite-datasource` |
| Agrégations SQL | PromQL | SQL natif (`GROUP BY strftime`) |
| Maintenance | Processus serveur | Fichier embarqué, WAL mode |

---

## 2. Architecture cible

```
┌─────────────────────────────────────────────────────────────┐
│                    Raspberry Pi 5 CM                        │
│                                                               │
│  daly-bms-server (Rust)                                       │
│    ├── RS485 polling (tokio-serial)                          │
│    ├── db_client.rs ──► /mnt/nvme/daly-bms/metrics.db      │
│    │                      (SQLite, WAL mode, tiering)       │
│    ├── Dashboard SSR (Askama + ECharts)                     │
│    ├── MQTT Bridge (rumqttc)                                │
│    └── AlertEngine (SQLite existant)                        │
│                                                               │
│  Grafana (service systemd)                                    │
│    └── Plugin frser-sqlite-datasource                       │
│           └── lecture directe de /mnt/nvme/daly-bms/metrics.db│
│                                                               │
│  NVMe 256 Go : /mnt/nvme                                    │
│    └── daly-bms/metrics.db  (< 20 Go à terme, 5 ans)      │
└─────────────────────────────────────────────────────────────┘
```

---

## 3. Stratégie de stockage : tiering automatique

Même sur NVMe 256 Go, stocker 5 ans de points bruts à la seconde ferait exploser le volume. Le tiering simule le comportement d'une TSDB.

| Niveau | Granularité | Rétention | Volume estimé | Usage |
|---|---|---|---|---|
| **raw** | Tous les points (5s) | 30 jours | ~6–10 Go | Temps réel, dashboard SSR, alertes |
| **hourly** | AVG, MIN, MAX, COUNT | 1 an | ~100 Mo | Grafana "mois/année" |
| **daily** | AVG, MIN, MAX, COUNT | 5 ans | ~50 Mo | Grafana "tendance 5 ans" |
| **Total** | | | **< 15 Go** | |

### Principe
- Toutes les nouvelles données entrent en `raw`
- Toutes les heures : un job compacte `raw` > 30j → `hourly`
- Tous les jours : un job compacte `hourly` > 1 an → `daily`
- Purge automatique des niveaux obsolètes

---

## 4. Schéma SQLite

### Fichier
`/mnt/nvme/daly-bms/metrics.db`

### Pragmas (optimisées NVMe)
```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA temp_store = MEMORY;
PRAGMA cache_size = -64000;          -- 64 Mo cache page
PRAGMA mmap_size = 268435456;        -- 256 Mo mmap pour lectures rapides
```

### Table principale
```sql
CREATE TABLE IF NOT EXISTS metrics (
    timestamp   INTEGER NOT NULL,         -- Unix epoch (seconds)
    device      TEXT NOT NULL,            -- 'bms-01', 'et112-pv', 'irradiance'
    metric      TEXT NOT NULL,            -- 'soc', 'voltage', 'power', 'current'
    granularity TEXT NOT NULL CHECK(granularity IN ('raw','hourly','daily')),
    value       REAL NOT NULL,            -- AVG pour hourly/daily
    min_val     REAL,                     -- NULL pour raw
    max_val     REAL,                     -- NULL pour raw
    count       INTEGER DEFAULT 1,        -- 1 pour raw
    PRIMARY KEY (device, metric, granularity, timestamp)
) WITHOUT ROWID;                        -- Index clustered, -20% taille
```

### Index
```sql
CREATE INDEX IF NOT EXISTS idx_metrics_time 
ON metrics(timestamp) WHERE granularity = 'raw';

CREATE INDEX IF NOT EXISTS idx_metrics_hourly 
ON metrics(timestamp) WHERE granularity = 'hourly';

CREATE INDEX IF NOT EXISTS idx_metrics_daily 
ON metrics(timestamp) WHERE granularity = 'daily';
```

---

## 5. Implémentation Rust : `db_client.rs`

### Localisation
`crates/daly-bms-server/src/db_client.rs`

### Dépendances (Cargo.toml)
```toml
[dependencies]
rusqlite = { version = "0.29", features = ["bundled"] }
anyhow = "1.0"
```

### Module complet
```rust
use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct MetricPoint {
    pub timestamp: u64,     // Unix epoch seconds
    pub device: String,
    pub metric: String,
    pub value: f64,
}

pub struct MetricsDb {
    conn: Arc<Connection>,
}

impl MetricsDb {
    /// Ouvre (ou crée) la base avec schéma et pragmas optimisés
    pub fn open(db_path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch("
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA cache_size = -64000;
            PRAGMA temp_store = MEMORY;
            PRAGMA mmap_size = 268435456;

            CREATE TABLE IF NOT EXISTS metrics (
                timestamp   INTEGER NOT NULL,
                device      TEXT NOT NULL,
                metric      TEXT NOT NULL,
                granularity TEXT NOT NULL CHECK(granularity IN ('raw','hourly','daily')),
                value       REAL NOT NULL,
                min_val     REAL,
                max_val     REAL,
                count       INTEGER DEFAULT 1,
                PRIMARY KEY (device, metric, granularity, timestamp)
            ) WITHOUT ROWID;

            CREATE INDEX IF NOT EXISTS idx_metrics_time 
            ON metrics(timestamp) WHERE granularity = 'raw';

            CREATE INDEX IF NOT EXISTS idx_metrics_hourly 
            ON metrics(timestamp) WHERE granularity = 'hourly';

            CREATE INDEX IF NOT EXISTS idx_metrics_daily 
            ON metrics(timestamp) WHERE granularity = 'daily';
        ")?;

        Ok(Self { conn: Arc::new(conn) })
    }

    /// Insertion batchée synchrone (à appeler via spawn_blocking)
    pub fn insert_batch(&self, points: &[MetricPoint]) -> anyhow::Result<()> {
        let mut stmt = self.conn.prepare_cached("
            INSERT OR REPLACE INTO metrics 
            (timestamp, device, metric, granularity, value, min_val, max_val, count)
            VALUES (?1, ?2, ?3, 'raw', ?4, NULL, NULL, 1)
        ")?;

        let tx = self.conn.unchecked_transaction()?;
        for p in points {
            stmt.execute(params![
                p.timestamp as i64,
                &p.device,
                &p.metric,
                p.value
            ])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Requête pour dashboard SSR ou Grafana
    pub fn query_range(
        &self,
        device: &str,
        metric: &str,
        granularity: &str,
        from: u64,
        to: u64,
    ) -> anyhow::Result<Vec<(u64, f64)>> {
        let mut stmt = self.conn.prepare_cached("
            SELECT timestamp, value 
            FROM metrics 
            WHERE device = ?1 AND metric = ?2 AND granularity = ?3
              AND timestamp >= ?4 AND timestamp <= ?5
            ORDER BY timestamp
        ")?;

        let rows = stmt.query_map(
            params![device, metric, granularity, from as i64, to as i64],
            |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, f64>(1)?))
        )?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Requête avec agrégation côté SQL (pour Grafana)
    pub fn query_aggregated(
        &self,
        device: &str,
        metric: &str,
        granularity: &str,
        from: u64,
        to: u64,
        bucket: &str,  -- ex: '%Y-%m' pour mensuel
    ) -> anyhow::Result<Vec<(String, f64)>> {
        let sql = format!("
            SELECT 
                strftime('{}', datetime(timestamp, 'unixepoch')) as bucket,
                AVG(value) as avg_val
            FROM metrics 
            WHERE device = ?1 AND metric = ?2 AND granularity = ?3
              AND timestamp >= ?4 AND timestamp <= ?5
            GROUP BY bucket
            ORDER BY bucket
        ", bucket);

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params![device, metric, granularity, from as i64, to as i64],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        )?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}
```

### Threading async (tokio)
```rust
use tokio::task;

pub async fn insert_metrics_async(
    db: Arc<MetricsDb>,
    points: Vec<MetricPoint>,
) -> anyhow::Result<()> {
    task::spawn_blocking(move || db.insert_batch(&points))
        .await
        .map_err(|e| anyhow::anyhow!("insert panic: {}", e))??;
    Ok(())
}
```

---

## 6. Maintenance automatique (tiering + purge)

### Localisation
Intégré dans `daly-bms-server` via `tokio::time::interval`.

### Job toutes les heures
```rust
use tokio::time::{interval, Duration};

pub async fn maintenance_task(db: Arc<MetricsDb>) {
    let mut ticker = interval(Duration::from_secs(3600));
    loop {
        ticker.tick().await;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;

        // 1. Raw (30j+) → Hourly
        let _ = db.conn.execute("
            INSERT OR REPLACE INTO metrics 
            (timestamp, device, metric, granularity, value, min_val, max_val, count)
            SELECT 
                strftime('%s', datetime(timestamp, 'unixepoch'), 'start of hour') as ts,
                device, metric, 'hourly',
                AVG(value), MIN(value), MAX(value), COUNT(*)
            FROM metrics
            WHERE granularity = 'raw'
              AND timestamp < strftime('%s', 'now', '-30 days')
              AND timestamp >= strftime('%s', 'now', '-1 year')
            GROUP BY ts, device, metric
        ", []);

        // 2. Hourly (1an+) → Daily
        let _ = db.conn.execute("
            INSERT OR REPLACE INTO metrics 
            (timestamp, device, metric, granularity, value, min_val, max_val, count)
            SELECT 
                strftime('%s', datetime(timestamp, 'unixepoch'), 'start of day') as ts,
                device, metric, 'daily',
                AVG(value), MIN(value), MAX(value), COUNT(*)
            FROM metrics
            WHERE granularity = 'hourly'
              AND timestamp < strftime('%s', 'now', '-1 year')
            GROUP BY ts, device, metric
        ", []);

        // 3. Purge raw > 30j
        let _ = db.conn.execute("
            DELETE FROM metrics 
            WHERE granularity = 'raw' 
              AND timestamp < strftime('%s', 'now', '-30 days')
        ", []);

        // 4. Purge hourly > 1 an
        let _ = db.conn.execute("
            DELETE FROM metrics 
            WHERE granularity = 'hourly' 
              AND timestamp < strftime('%s', 'now', '-1 year')
        ", []);

        // 5. WAL checkpoint (évite que -wal file ne grossisse indéfiniment)
        let _ = db.conn.execute("PRAGMA wal_checkpoint(TRUNCATE)", []);
    }
}
```

---

## 7. Intégration Grafana

### 7.1 Installation du plugin
```bash
sudo grafana-cli plugins install frser-sqlite-datasource
sudo systemctl restart grafana-server
```

### 7.2 Configuration du datasource
- **Type** : SQLite
- **Path** : `/mnt/nvme/daly-bms/metrics.db`
- **Permissions** : voir section 9

### 7.3 Exemples de requêtes SQL

#### Dashboard "Temps réel" (7 derniers jours)
```sql
SELECT 
    datetime(timestamp, 'unixepoch') as time,
    value
FROM metrics
WHERE device = 'bms-01'
  AND metric = 'soc'
  AND granularity = 'raw'
  AND timestamp >= ${__from:date:seconds}
  AND timestamp <= ${__to:date:seconds}
ORDER BY timestamp
```

#### Dashboard "Historique annuel"
```sql
SELECT 
    datetime(timestamp, 'unixepoch') as time,
    value,
    min_val,
    max_val
FROM metrics
WHERE device = 'bms-01'
  AND metric = 'soc'
  AND granularity = 'hourly'
  AND timestamp >= ${__from:date:seconds}
  AND timestamp <= ${__to:date:seconds}
ORDER BY timestamp
```

#### Dashboard "Tendance 5 ans"
```sql
SELECT 
    datetime(timestamp, 'unixepoch') as time,
    value
FROM metrics
WHERE device = 'bms-01'
  AND metric = 'soc'
  AND granularity = 'daily'
  AND timestamp >= ${__from:date:seconds}
  AND timestamp <= ${__to:date:seconds}
ORDER BY timestamp
```

#### Agrégation mensuelle/annuelle (SQL natif)
```sql
SELECT 
    strftime('%Y-%m-01', datetime(timestamp, 'unixepoch')) as time,
    AVG(value) as "SOC moyen",
    MIN(min_val) as "SOC min",
    MAX(max_val) as "SOC max"
FROM metrics
WHERE device = 'bms-01'
  AND metric = 'soc'
  AND granularity = 'daily'
  AND timestamp >= ${__from:date:seconds}
  AND timestamp <= ${__to:date:seconds}
GROUP BY strftime('%Y-%m', datetime(timestamp, 'unixepoch'))
ORDER BY time
```

---

## 8. Plan de travail (ordre chronologique)

| # | Étape | Fichier / Livrable | Durée estimée |
|---|---|---|---|
| 1 | **Préparation NVMe** | Monter `/mnt/nvme`, créer `/mnt/nvme/daly-bms/`, permissions | 30 min |
| 2 | **Créer `db_client.rs`** | `crates/daly-bms-server/src/db_client.rs` | 3h |
| 3 | **Intégration dual-write** | Modifier `state.rs` et `monitor.rs` pour écrire dans SQLite ET VM simultanément | 2h |
| 4 | **Thread maintenance** | Intégrer `maintenance_task` dans le cycle de vie tokio | 3h |
| 5 | **Migration dashboard SSR** | Adapter routes `/chart/*` et `/history/*` pour lire SQLite | 2h |
| 6 | **Plugin Grafana** | Installer `frser-sqlite-datasource`, configurer datasource | 30 min |
| 7 | **Dashboard test Grafana** | Recréer un dashboard "SOC BMS-01" en SQL pour valider | 1h |
| 8 | **Validation 48h dual-write** | Comparer données VM vs SQLite, vérifier cohérence | 2 jours |
| 9 | **Migration dashboards complets** | Transcrire tous les dashboards PromQL → SQL | 4h |
| 10 | **Retrait VM** | Supprimer `vm_client.rs`, service systemd VM, `contrib/victoriametrics-scrape.yml` | 1h |
| 11 | **Optimisation finale** | `PRAGMA optimize`, `VACUUM` annuel programmé, documentation | 1h |

**Total développement** : ~15h  
**Total validation** : 2–3 jours

---

## 9. Permissions et sécurité fichier

Grafana et `daly-bms-server` doivent lire/écrire le même fichier SQLite (WAL mode).

### Groupe commun
```bash
sudo groupadd daly-metrics
sudo usermod -aG daly-metrics grafana      # user service Grafana
sudo usermod -aG daly-metrics daly-bms       # user service daly-bms-server
```

### Répertoire NVMe
```bash
sudo mkdir -p /mnt/nvme/daly-bms
sudo chown -R root:daly-metrics /mnt/nvme/daly-bms
sudo chmod 2775 /mnt/nvme/daly-bms          # sticky bit pour héritage groupe
```

### Permissions fichier (dans Rust)
```rust
use std::os::unix::fs::PermissionsExt;

// Après Connection::open()
std::fs::set_permissions(db_path, std::fs::Permissions::from_mode(0o664))?;
```

---

## 10. Risques et mitigations

| Risque | Probabilité | Impact | Mitigation |
|---|---|---|---|
| Plugin Grafana ne supporte pas les CTE complexes | Moyen | Dashboards bloqués | Utiliser uniquement `SELECT ... WHERE` simples. Éviter `WITH`, `UNION`, sous-requêtes. |
| Fichier SQLite verrouillé (EXCLUSIVE) | Faible | Crash écriture | WAL mode obligatoire. Jamais de transaction longue. |
| Fragmentation après 5 ans | Moyen | Lenteur lectures | `VACUUM` annuel via cron. Fichier restera < 20 Go. |
| Corruption NVMe (coupure courant) | Faible | Perte données | `synchronous = NORMAL` + WAL. Pour paranoïa : `FULL` (pénalité ~20%). |
| Requête Grafana "5 ans" trop lente | Moyen | Timeout | Forcer `granularity = 'daily'` dans les requêtes longues. |
| Volume raw imprévu (> 30j) | Faible | eMMC/NVMe plein | Monitoring taille fichier + alerte si > 20 Go. |

---

## 11. Checklist post-migration

- [ ] `metrics.db` créé sur NVMe avec WAL mode actif
- [ ] `daly-bms-server` écrit dans SQLite (vérifier via `SELECT COUNT(*) FROM metrics`)
- [ ] Dashboard SSR (`/dashboard`) affiche les données SQLite
- [ ] Grafana datasource SQLite configuré et testé
- [ ] Au moins un dashboard Grafana "5 ans" fonctionne en SQL
- [ ] Maintenance tiering exécutée sans erreur (logs `journalctl`)
- [ ] `victoriametrics.service` arrêté et désactivé
- [ ] `vm_client.rs` supprimé du workspace
- [ ] `Makefile` mis à jour (retirer cibles VM)
- [ ] README mis à jour (architecture, RAM estimée)
- [ ] Backup `metrics.db` programmé (rsync vers NAS/cloud)

---

## 12. Estimation RAM post-migration

| Service | Avant (VM) | Après (SQLite) |
|---|---|---|
| daly-bms-server | ~27 Mo | ~30 Mo (+3 Mo SQLite) |
| VictoriaMetrics | **~150 Mo** | **0** (supprimé) |
| **Total économisé** | | **~150 Mo** |

**RAM totale Pi5** : ~450 Mo au lieu de ~600 Mo (sur 4 Go disponibles).

---

*Document généré pour le projet Daly-BMS-Rust — Migration VictoriaMetrics → SQLite*
