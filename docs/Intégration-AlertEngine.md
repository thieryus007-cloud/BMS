**Plan de base pour intégrer vmalert** au stack Rust SSR + VictoriaMetrics.
Attention : ce plan DOIT etre adapté pour etre en cohérence avec l'infrastructure existante.
---

## 📋 Intégration vmalert + webhook Rust + dashboard alertes

### 🎯 Résumé des changements

1. Charger les règles depuis un fichier externe (TOML ou YAML)
   → modifier bridge/alerts.rs pour lire /etc/daly-bms/alert-rules.toml
   
2. Étendre AlertContext avec toutes les sources
   → BmsSnapshot (déjà là)
   → VenusSmartShunt (venus_shunt_current_a, soc)  
   → Et112Snapshot (puissance réseau)
   → VenusInverter (tension DC bus)

3. Support `for: duration` per règle
   → Remplacer cooldown par un début de déclenchement + durée requise
   → Déjà partiellement là avec RuleState, ajouter pending_since: Option<Instant>

4. API REST + dashboard Askama
   → Lire la table alert_events existante
   → Ajouter acknowledge
---

### 📁 Fichiers ajoutés/modifiés en fonction de notre infrastructure

#### 1️⃣ **docker-compose.infra.yml** - Ajout du service vmalert **Attention, je PREFERE SANS DOCKER**

```yaml
# Ajouter après VictoriaMetrics dans docker-compose.infra.yml
  vmalert:
    image: victoriametrics/vmalert:v1.105.0
    container_name: dalybms-vmalert
    command:
      - "-datasource.url=http://dalybms-VictoriaMetrics:8428"
      - "-notifier.url=http://dalybms-server:8080/api/v1/alerts/webhook"
      - "-remoteWrite.url=http://dalybms-VictoriaMetrics:8428"
      - "-remoteRead.url=http://dalybms-VictoriaMetrics:8428"
      - "-rule=/rules/*.yaml"
      - "-evaluationInterval=30s"
      - "-httpListenAddr=0.0.0.0:8880"
    ports:
      - "8880:8880"  # Interface web vmalert (optionnelle)
    volumes:
      - ./vmalert/rules:/rules:ro
      - vmalert_data:/vmalertdata
    depends_on:
      - VictoriaMetrics
      - dalybms-server
    restart: unless-stopped
    networks:
      - daly-bms-net
    mem_limit: 64m
    mem_reservation: 32m

# Ajouter dans les volumes (à la fin du fichier)
volumes:
  vmalert_data:
    driver: local
```

---

#### 2️⃣ **vmalert/rules/ess-alerts.yaml** - Règles d'alerte 
**Attention, je PREFERE pouvoir changer les regles sans re-compilation si possible, VERIFIER si POSSIBLE integration avec rust-rule-engine**

```yaml
groups:
  - name: ess-battery-critical
    rules:
      - alert: BatterySOCCritical
        expr: venus_battery_soc < 15
        for: 3m
        labels:
          severity: critical
          category: battery
        annotations:
          summary: "SOC Batterie Critique"
          description: "Le SOC de la batterie est à {{ $value | printf \"%.1f\" }}% depuis plus de 3 minutes"
          value: "{{ $value }}"
          threshold: "15%"

      - alert: BatterySOCWarning
        expr: venus_battery_soc < 20 and venus_battery_soc >= 15
        for: 5m
        labels:
          severity: warning
          category: battery
        annotations:
          summary: "SOC Batterie Faible"
          description: "Le SOC de la batterie est à {{ $value | printf \"%.1f\" }}% depuis plus de 5 minutes"
          value: "{{ $value }}"
          threshold: "20%"

  - name: ess-current-voltage
    rules:
      - alert: HighDischargeCurrent
        expr: venus_shunt_current_a > 100
        for: 2m
        labels:
          severity: warning
          category: current
        annotations:
          summary: "Courant de Décharge Élevé"
          description: "Courant de décharge de {{ $value | printf \"%.1f\" }}A détecté"
          value: "{{ $value }}"
          threshold: "100A"

      - alert: HighChargeCurrent
        expr: venus_shunt_current_a < -100
        for: 2m
        labels:
          severity: warning
          category: current
        annotations:
          summary: "Courant de Charge Élevé"
          description: "Courant de charge de {{ $value | printf \"%.1f\" | abs }}A détecté"
          value: "{{ $value }}"
          threshold: "-100A"

      - alert: BatteryVoltageHigh
        expr: venus_battery_voltage > 57
        for: 1m
        labels:
          severity: critical
          category: voltage
        annotations:
          summary: "Tension Batterie Trop Haute"
          description: "Tension batterie: {{ $value | printf \"%.2f\" }}V (> 57V)"
          value: "{{ $value }}"
          threshold: "57V"

      - alert: BatteryVoltageLow
        expr: venus_battery_voltage < 44
        for: 1m
        labels:
          severity: critical
          category: voltage
        annotations:
          summary: "Tension Batterie Trop Basse"
          description: "Tension batterie: {{ $value | printf \"%.2f\" }}V (< 44V)"
          value: "{{ $value }}"
          threshold: "44V"

  - name: ess-energy
    rules:
      - alert: HighDailyCyclage
        expr: >
          (avg_over_time(clamp_min(venus_shunt_current_a,0)[24h]) 
          - avg_over_time(clamp_max(venus_shunt_current_a,0)[24h])) * 24 / 200 * 100 > 80
        for: 0m
        labels:
          severity: info
          category: energy
        annotations:
          summary: "Taux de Cyclage Quotidien Élevé"
          description: "Le taux de cyclage sur 24h est de {{ $value | printf \"%.1f\" }}%"
          value: "{{ $value }}"
          threshold: "80%"

  - name: ess-temperature
    rules:
      - alert: BatteryTempHigh
        expr: venus_battery_temp_avg > 45
        for: 3m
        labels:
          severity: warning
          category: temperature
        annotations:
          summary: "Température Batterie Élevée"
          description: "Température moyenne: {{ $value | printf \"%.1f\" }}°C"
          value: "{{ $value }}"
          threshold: "45°C"

  - name: ess-recording-rules
    rules:
      - record: ess:cyclage_quotidien_pct
        expr: >
          (avg_over_time(clamp_min(venus_shunt_current_a,0)[24h]) 
          - avg_over_time(clamp_max(venus_shunt_current_a,0)[24h])) * 24 / 200 * 100

      - record: ess:charge_ah_24h
        expr: avg_over_time(clamp_min(venus_shunt_current_a, 0)[24h]) * 24

      - record: ess:discharge_ah_24h
        expr: -avg_over_time(clamp_max(venus_shunt_current_a, 0)[24h]) * 24
```

---

#### 3️⃣ **crates/daly-bms-server/src/api/alerts.rs** - Nouveau fichier API

```rust
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use time::OffsetDateTime;
use tower_http::services::ServeDir;
use utoipa::ToSchema;

use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/webhook", axum::routing::post(handle_webhook))
        .route("/list", axum::routing::get(list_alerts))
        .route("/count", axum::routing::get(count_alerts))
        .route("/:id/acknowledge", axum::routing::post(acknowledge_alert))
        .route("/active", axum::routing::get(active_alerts))
        .with_state(state)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AlertPayload {
    pub version: String,
    pub groupKey: String,
    pub status: String, // "firing" or "resolved"
    pub alerts: Vec<AlertItem>,
    pub externalURL: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AlertItem {
    pub status: String,
    pub labels: AlertLabels,
    pub annotations: AlertAnnotations,
    #[serde(rename = "startsAt")]
    pub starts_at: String,
    #[serde(rename = "endsAt", default)]
    pub ends_at: Option<String>,
    #[serde(rename = "generatorURL", default)]
    pub generator_url: Option<String>,
    #[serde(rename = "fingerprint", default)]
    pub fingerprint: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AlertLabels {
    pub alertname: String,
    pub severity: String,
    pub category: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AlertAnnotations {
    pub summary: String,
    pub description: String,
    pub value: Option<String>,
    pub threshold: Option<String>,
}

#[derive(Debug, Serialize, FromRow, ToSchema)]
pub struct AlertRecord {
    pub id: i64,
    pub alert_name: String,
    pub severity: String,
    pub category: String,
    pub status: String,
    pub summary: String,
    pub description: String,
    pub value: Option<String>,
    pub threshold: Option<String>,
    pub starts_at: OffsetDateTime,
    pub ends_at: Option<OffsetDateTime>,
    pub acknowledged: bool,
    pub acknowledged_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AlertListResponse {
    pub total: i64,
    pub alerts: Vec<AlertRecord>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AlertCountResponse {
    pub total: i64,
    pub firing: i64,
    pub resolved: i64,
    pub critical: i64,
    pub warning: i64,
    pub info: i64,
}

/// Webhook endpoint pour recevoir les alertes de vmalert
#[utoipa::path(
    post,
    path = "/api/v1/alerts/webhook",
    request_body = AlertPayload,
    responses(
        (status = 200, description = "Alerte reçue et traitée")
    ),
    tag = "alerts"
)]
async fn handle_webhook(
    State(state): State<AppState>,
    Json(payload): Json<AlertPayload>,
) -> StatusCode {
    let pool = match state.db_pool {
        Some(ref p) => p,
        None => {
            tracing::warn!("Database pool not available, cannot store alert");
            return StatusCode::SERVICE_UNAVAILABLE;
        }
    };

    for alert in payload.alerts {
        let result = process_alert(pool, alert, &payload.status).await;
        if let Err(e) = result {
            tracing::error!("Failed to process alert: {}", e);
        }
    }

    // Broadcast aux clients WebSocket connectés
    if let Err(e) = state.broadcast_alert_update().await {
        tracing::warn!("Failed to broadcast alert update: {}", e);
    }

    StatusCode::OK
}

async fn process_alert(
    pool: &SqlitePool,
    alert: AlertItem,
    payload_status: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let status = if payload_status == "firing" {
        "firing"
    } else {
        "resolved"
    };

    // Parser les timestamps
    let starts_at = OffsetDateTime::parse(&alert.starts_at, &time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| OffsetDateTime::now_utc());
    
    let ends_at = alert.ends_at.as_ref().and_then(|s| {
        OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
    });

    // Vérifier si l'alerte existe déjà
    let existing: Option<AlertRecord> = sqlx::query_as(
        "SELECT * FROM alerts WHERE alert_name = ? AND fingerprint = ? ORDER BY created_at DESC LIMIT 1"
    )
    .bind(&alert.labels.alertname)
    .bind(&alert.fingerprint.clone().unwrap_or_default())
    .fetch_optional(pool)
    .await?;

    if let Some(record) = existing {
        // Mettre à jour l'alerte existante
        sqlx::query(
            "UPDATE alerts SET 
             status = ?, ends_at = ?, updated_at = ?
             WHERE id = ?"
        )
        .bind(status)
        .bind(ends_at)
        .bind(OffsetDateTime::now_utc())
        .bind(record.id)
        .execute(pool)
        .await?;
    } else {
        // Créer une nouvelle alerte
        sqlx::query(
            "INSERT INTO alerts 
             (alert_name, severity, category, status, summary, description, value, threshold, 
              starts_at, ends_at, acknowledged, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&alert.labels.alertname)
        .bind(&alert.labels.severity)
        .bind(&alert.labels.category)
        .bind(status)
        .bind(&alert.annotations.summary)
        .bind(&alert.annotations.description)
        .bind(&alert.annotations.value)
        .bind(&alert.annotations.threshold)
        .bind(starts_at)
        .bind(ends_at)
        .bind(false)
        .bind(OffsetDateTime::now_utc())
        .bind(OffsetDateTime::now_utc())
        .execute(pool)
        .await?;
    }

    Ok(())
}

/// Lister toutes les alertes avec pagination
#[utoipa::path(
    get,
    path = "/api/v1/alerts/list",
    params(
        ("limit" = Option<u32>, Query, description = "Nombre max d'alertes"),
        ("offset" = Option<u32>, Query, description = "Offset pour pagination"),
        ("severity" = Option<String>, Query, description = "Filtrer par sévérité"),
        ("status" = Option<String>, Query, description = "Filtrer par statut")
    ),
    responses(
        (status = 200, body = AlertListResponse)
    ),
    tag = "alerts"
)]
async fn list_alerts(
    State(state): State<AppState>,
    Query(params): Query<AlertListParams>,
) -> Result<Json<AlertListResponse>, StatusCode> {
    let pool = state.db_pool.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let where_clauses = vec![
        params.severity.map(|s| format!("severity = '{}'", s)),
        params.status.map(|s| format!("status = '{}'", s)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();

    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };

    let limit = params.limit.unwrap_or(100);
    let offset = params.offset.unwrap_or(0);

    let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM alerts {}", where_sql))
        .fetch_one(pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let alerts: Vec<AlertRecord> = sqlx::query_as(&format!(
        "SELECT * FROM alerts {} ORDER BY created_at DESC LIMIT {} OFFSET {}",
        where_sql, limit, offset
    ))
    .fetch_all(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(AlertListResponse { total, alerts }))
}

#[derive(Debug, Deserialize)]
struct AlertListParams {
    limit: Option<u32>,
    offset: Option<u32>,
    severity: Option<String>,
    status: Option<String>,
}

/// Compter les alertes par statut et sévérité
#[utoipa::path(
    get,
    path = "/api/v1/alerts/count",
    responses(
        (status = 200, body = AlertCountResponse)
    ),
    tag = "alerts"
)]
async fn count_alerts(
    State(state): State<AppState>,
) -> Result<Json<AlertCountResponse>, StatusCode> {
    let pool = state.db_pool.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM alerts")
        .fetch_one(pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let firing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM alerts WHERE status = 'firing'")
        .fetch_one(pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let resolved: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM alerts WHERE status = 'resolved'")
        .fetch_one(pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let critical: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM alerts WHERE severity = 'critical'")
        .fetch_one(pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let warning: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM alerts WHERE severity = 'warning'")
        .fetch_one(pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let info: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM alerts WHERE severity = 'info'")
        .fetch_one(pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(AlertCountResponse {
        total,
        firing,
        resolved,
        critical,
        warning,
        info,
    }))
}

/// Acknowledger une alerte
#[utoipa::path(
    post,
    path = "/api/v1/alerts/{id}/acknowledge",
    params(("id" = i64, Path, description = "ID de l'alerte")),
    responses(
        (status = 200, description = "Alerte acknowledge"),
        (status = 404, description = "Alerte non trouvée")
    ),
    tag = "alerts"
)]
async fn acknowledge_alert(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    let pool = state.db_pool.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let result = sqlx::query(
        "UPDATE alerts SET acknowledged = TRUE, acknowledged_at = ?, updated_at = ? WHERE id = ?"
    )
    .bind(OffsetDateTime::now_utc())
    .bind(OffsetDateTime::now_utc())
    .bind(id)
    .execute(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if result.rows_affected() == 0 {
        Err(StatusCode::NOT_FOUND)
    } else {
        Ok(StatusCode::OK)
    }
}

/// Obtenir les alertes actives (firing)
#[utoipa::path(
    get,
    path = "/api/v1/alerts/active",
    responses(
        (status = 200, body = Vec<AlertRecord>)
    ),
    tag = "alerts"
)]
async fn active_alerts(
    State(state): State<AppState>,
) -> Result<Json<Vec<AlertRecord>>, StatusCode> {
    let pool = state.db_pool.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let alerts: Vec<AlertRecord> = sqlx::query_as(
        "SELECT * FROM alerts WHERE status = 'firing' ORDER BY created_at DESC"
    )
    .fetch_all(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(alerts))
}
```

---

#### 4️⃣ **crates/daly-bms-server/src/state.rs** - Mise à jour AppState

```rust
// Ajouter dans AppState
pub struct AppState {
    // ... champs existants ...
    pub db_pool: Option<SqlitePool>,
    pub alert_tx: Option<tokio::sync::broadcast::Sender<AlertUpdate>>,
}

// Ajouter nouveau type pour broadcast
#[derive(Clone, Debug, Serialize)]
pub struct AlertUpdate {
    pub alert_name: String,
    pub status: String,
    pub severity: String,
    pub timestamp: OffsetDateTime,
}

// Implémenter la méthode de broadcast
impl AppState {
    pub async fn broadcast_alert_update(&self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(ref tx) = self.alert_tx {
            // Créer un message de mise à jour
            let update = AlertUpdate {
                alert_name: "update".to_string(),
                status: "refresh".to_string(),
                severity: "info".to_string(),
                timestamp: OffsetDateTime::now_utc(),
            };
            let _ = tx.send(update);
        }
        Ok(())
    }
}
```

---

#### 5️⃣ **crates/daly-bms-server/src/main.rs** - Intégration

```rust
// Ajouter dans les use
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

// Dans la fonction main, après l'initialisation de l'AppState
let db_pool = if config.database.enabled {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&config.database.url)
        .await
        .expect("Failed to create database pool");
    
    // Initialiser la table alerts
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS alerts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            alert_name TEXT NOT NULL,
            severity TEXT NOT NULL,
            category TEXT NOT NULL,
            status TEXT NOT NULL,
            summary TEXT NOT NULL,
            description TEXT NOT NULL,
            value TEXT,
            threshold TEXT,
            starts_at DATETIME NOT NULL,
            ends_at DATETIME,
            acknowledged BOOLEAN DEFAULT FALSE,
            acknowledged_at DATETIME,
            created_at DATETIME NOT NULL,
            updated_at DATETIME NOT NULL
        )"
    )
    .execute(&pool)
    .await
    .expect("Failed to create alerts table");
    
    // Index pour les requêtes fréquentes
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_alerts_status ON alerts(status)"
    )
    .execute(&pool)
    .await?;
    
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_alerts_severity ON alerts(severity)"
    )
    .execute(&pool)
    .await?;
    
    Some(pool)
} else {
    None
};

// Créer le channel pour broadcast alertes
let (alert_tx, _) = tokio::sync::broadcast::channel(100);

let app_state = AppState {
    // ... champs existants ...
    db_pool: db_pool.clone(),
    alert_tx: Some(alert_tx),
};

// Ajouter le router alerts dans api/mod.rs
// router = router.nest("/alerts", alerts::router(app_state.clone()));
```

---

#### 6️⃣ **crates/daly-bms-server/templates/alerts.html** - Dashboard Askama

```html
<!DOCTYPE html>
<html lang="fr" data-theme="dark">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Alertes - Daly BMS</title>
    <script src="https://cdn.tailwindcss.com"></script>
    <script src="https://cdn.jsdelivr.net/npm/echarts@5.4.3/dist/echarts.min.js"></script>
    <link rel="stylesheet" href="/static/css/dashboard.css">
</head>
<body class="bg-base-300 min-h-screen">
    <div class="container mx-auto px-4 py-6">
        <!-- Header -->
        <div class="flex justify-between items-center mb-6">
            <h1 class="text-3xl font-bold">Alertes</h1>
            <div class="flex gap-3">
                <button onclick="loadAlerts()" class="btn btn-primary btn-sm">
                    <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"></path>
                    </svg>
                    Rafraîchir
                </button>
                <a href="/dashboard" class="btn btn-ghost btn-sm">Retour</a>
            </div>
        </div>

        <!-- Stats -->
        <div class="stats shadow mb-6 w-full">
            <div class="stat">
                <div class="stat-figure text-error">
                    <svg class="w-8 h-8" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"></path>
                    </svg>
                </div>
                <div class="stat-title">Alertes Actives</div>
                <div class="stat-value text-error" id="active-count">0</div>
                <div class="stat-desc">Nécessitent attention</div>
            </div>
            
            <div class="stat">
                <div class="stat-figure text-warning">
                    <svg class="w-8 h-8" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path>
                    </svg>
                </div>
                <div class="stat-title">Warnings</div>
                <div class="stat-value text-warning" id="warning-count">0</div>
            </div>
            
            <div class="stat">
                <div class="stat-figure text-info">
                    <svg class="w-8 h-8" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path>
                    </svg>
                </div>
                <div class="stat-title">Info</div>
                <div class="stat-value text-info" id="info-count">0</div>
            </div>
        </div>

        <!-- Filtres -->
        <div class="card bg-base-100 shadow mb-6">
            <div class="card-body">
                <div class="flex gap-4 flex-wrap">
                    <select id="severity-filter" class="select select-bordered select-sm" onchange="loadAlerts()">
                        <option value="">Toutes sévérités</option>
                        <option value="critical">Critique</option>
                        <option value="warning">Warning</option>
                        <option value="info">Info</option>
                    </select>
                    
                    <select id="status-filter" class="select select-bordered select-sm" onchange="loadAlerts()">
                        <option value="">Tous statuts</option>
                        <option value="firing">Actives</option>
                        <option value="resolved">Résolues</option>
                    </select>
                    
                    <button onclick="showOnlyActive()" class="btn btn-primary btn-sm">Actives uniquement</button>
                </div>
            </div>
        </div>

        <!-- Liste des alertes -->
        <div class="card bg-base-100 shadow">
            <div class="card-body">
                <h2 class="card-title mb-4">Historique des Alertes</h2>
                <div id="alerts-container" class="space-y-3">
                    <!-- Les alertes seront injectées ici -->
                </div>
            </div>
        </div>
    </div>

    <script>
        let alertCounts = { active: 0, warning: 0, info: 0 };

        async function loadAlerts() {
            try {
                const severity = document.getElementById('severity-filter').value;
                const status = document.getElementById('status-filter').value;
                
                let url = '/api/v1/alerts/list?limit=50';
                if (severity) url += `&severity=${severity}`;
                if (status) url += `&status=${status}`;

                const response = await fetch(url);
                const data = await response.json();
                
                renderAlerts(data.alerts);
                updateCounts();
            } catch (error) {
                console.error('Erreur chargement alertes:', error);
            }
        }

        function renderAlerts(alerts) {
            const container = document.getElementById('alerts-container');
            
            if (alerts.length === 0) {
                container.innerHTML = `
                    <div class="alert alert-info">
                        <svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path>
                        </svg>
                        <span>Aucune alerte à afficher</span>
                    </div>
                `;
                return;
            }

            container.innerHTML = alerts.map(alert => `
                <div class="alert ${getSeverityClass(alert.severity)} ${alert.status === 'firing' ? 'animate-pulse' : ''}">
                    <svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        ${getSeverityIcon(alert.severity)}
                    </svg>
                    <div class="flex-1">
                        <div class="flex items-center gap-2">
                            <span class="font-bold">${alert.alert_name}</span>
                            <span class="badge badge-sm">${alert.severity}</span>
                            <span class="badge badge-sm">${alert.status}</span>
                            ${alert.acknowledged ? '<span class="badge badge-sm badge-success">Ack</span>' : ''}
                        </div>
                        <div class="text-sm mt-1">${alert.summary}</div>
                        <div class="text-xs opacity-70 mt-1">${alert.description}</div>
                        ${alert.value ? `<div class="text-xs mt-1">Valeur: <strong>${alert.value}</strong> (Seuil: ${alert.threshold || 'N/A'})</div>` : ''}
                        <div class="text-xs opacity-50 mt-1">
                            Début: ${new Date(alert.starts_at).toLocaleString('fr-FR')}
                            ${alert.ends_at ? ` | Fin: ${new Date(alert.ends_at).toLocaleString('fr-FR')}` : ''}
                        </div>
                    </div>
                    ${alert.status === 'firing' && !alert.acknowledged ? `
                        <button onclick="acknowledgeAlert(${alert.id})" class="btn btn-sm btn-success">
                            Ack
                        </button>
                    ` : ''}
                </div>
            `).join('');
        }

        function getSeverityClass(severity) {
            switch(severity) {
                case 'critical': return 'alert-error';
                case 'warning': return 'alert-warning';
                case 'info': return 'alert-info';
                default: return 'alert';
            }
        }

        function getSeverityIcon(severity) {
            switch(severity) {
                case 'critical':
                    return '<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"></path>';
                case 'warning':
                    return '<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path>';
                default:
                    return '<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path>';
            }
        }

        async function acknowledgeAlert(id) {
            try {
                await fetch(`/api/v1/alerts/${id}/acknowledge`, { method: 'POST' });
                loadAlerts();
            } catch (error) {
                console.error('Erreur acknowledge:', error);
            }
        }

        async function updateCounts() {
            try {
                const response = await fetch('/api/v1/alerts/count');
                const counts = await response.json();
                
                document.getElementById('active-count').textContent = counts.firing;
                document.getElementById('warning-count').textContent = counts.warning;
                document.getElementById('info-count').textContent = counts.info;
            } catch (error) {
                console.error('Erreur compteurs:', error);
            }
        }

        function showOnlyActive() {
            document.getElementById('status-filter').value = 'firing';
            loadAlerts();
        }

        // Auto-refresh toutes les 30 secondes
        setInterval(loadAlerts, 30000);

        // Chargement initial
        loadAlerts();
    </script>
</body>
</html>
```

---

#### 7️⃣ **Config.toml** - Ajout configuration database

```toml
# Ajouter à la fin de Config.toml
[database]
enabled = true
url = "sqlite:///var/lib/daly-bms/alerts.db"
```

---

#### 8️⃣ **Makefile** - Nouvelles commandes

```makefile
# Ajouter dans le Makefile
.PHONY: vmalert-rules vmalert-up vmalert-logs

vmalert-up:
	docker compose -f docker-compose.infra.yml up -d vmalert

vmalert-down:
	docker compose -f docker-compose.infra.yml stop vmalert

vmalert-logs:
	docker logs dalybms-vmalert -f

vmalert-rules-check:
	docker exec dalybms-vmalert vmalert -rule=/rules/*.yaml -dryRun

test-alerts:
	@echo "Test webhook alert..."
	curl -X POST http://localhost:8080/api/v1/alerts/webhook \
	  -H "Content-Type: application/json" \
	  -d '{
	    "version": "4",
	    "groupKey": "test",
	    "status": "firing",
	    "alerts": [{
	      "status": "firing",
	      "labels": {
	        "alertname": "TestAlert",
	        "severity": "warning",
	        "category": "test"
	      },
	      "annotations": {
	        "summary": "Test Alert",
	        "description": "This is a test alert",
	        "value": "42",
	        "threshold": "40"
	      },
	      "startsAt": "2026-05-02T12:00:00Z"
	    }]
	  }'
```

---

### 📝 Notes pour la PR

**Impact mémoire estimé:**
- vmalert: ~32-64 MB RAM
- SQLite alerts: ~5-10 MB
- Total additionnel: < 100 MB (acceptable sur Pi5 4GB)

**Tests à effectuer:**
1. ✅ Vérifier que vmalert démarre correctement
2. ✅ Tester le webhook avec `make test-alerts`
3. ✅ Valider l'affichage dans `/dashboard/alerts`
4. ✅ Tester l'acknowledge d'alerte
5. ✅ Vérifier la persistance SQLite après redémarrage

**Documentation à mettre à jour:**
- Ajouter section "Alerting" dans Readme.md
- Mettre à jour DEPLOYMENT.md avec vmalert
- Créer ALERTING.md avec guide complet

---
