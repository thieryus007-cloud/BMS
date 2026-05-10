# AlertEngine — Moteur d'alertes natif Rust

> **Statut : déployé** — Remplace entièrement vmalert. Aucune dépendance externe.
> Dernière mise à jour : 2026-05-02

---

## 1. Vue d'ensemble

`AlertEngine` est le moteur d'alertes intégré à `daly-bms-server`. Il évalue des règles métier sur chaque snapshot BMS reçu via le bus broadcast interne, journalise les événements dans SQLite et envoie des notifications Telegram.

### Fonctionnalités clés

| Fonction | Détail |
|----------|--------|
| **9 règles actives** | Cellules, pack, SOC, température, courant |
| **`for:` duration** | Condition doit rester vraie X secondes avant déclenchement |
| **Hysteresis** | Seuil de réarmement distinct du seuil de déclenchement |
| **Cooldown par règle** | Évite le spam de notifications |
| **Journal SQLite** | Tous les événements `triggered`/`cleared` persistés |
| **Acquittement** | Via dashboard ou API REST `POST /api/v1/alerts/:id/acknowledge` |
| **Telegram** | Notification push à chaque déclenchement/effacement |
| **Source SmartShunt** | Courant Victron prioritaire sur courant BMS interne |

---

## 2. Architecture

```
BmsSnapshot broadcast (AppBus)
        │
        ▼
run_alert_engine()          ← tâche Tokio background
        │  lit shunt_current_a depuis AppState.venus_smartshunt
        │
        ▼
AlertEngine::evaluate()     ← évalue chaque règle sur chaque BMS
        │
        ├─ RuleState (en mémoire, HashMap keyed sur (bms_address, rule_id))
        │     ├─ active: bool
        │     ├─ last_notified: Option<Instant>   ← cooldown
        │     └─ pending_since: Option<Instant>   ← timer "for:"
        │
        ├─ SQLite (alert_events)                  ← journal persisté
        │
        └─ Telegram (optionnel)
```

`Arc<AlertEngine>` est partagé entre :
- la tâche background `run_alert_engine` (évaluation)
- `AppState.alert_engine` (accès API REST via `spawn_blocking`)

---

## 3. Les 9 règles actives

### 3.1 Règles cellule BMS

| ID | Description | Sévérité | Source | Seuil trigger | Seuil clear | Durée min | Cooldown |
|----|-------------|----------|--------|---------------|-------------|-----------|----------|
| `cell_ovp` | Sur-tension cellule | Critical | `max_cell_voltage` | > 3.60 V | < 3.55 V | immédiat | 5 min |
| `cell_uvp` | Sous-tension cellule | Critical | `min_cell_voltage` | < 2.90 V | > 2.95 V | immédiat | 5 min |
| `cell_imbalance` | Déséquilibre cellules | Warning | `cell_delta_mv()` | > 100 mV | < 90 mV | immédiat | 10 min |

### 3.2 Règles ESS niveau pack

| ID | Description | Sévérité | Source | Seuil trigger | Seuil clear | Durée min | Cooldown |
|----|-------------|----------|--------|---------------|-------------|-----------|----------|
| `soc_low` | SOC bas | Warning | `snap.soc` | < 20 % | > 25 % | 5 min | 15 min |
| `soc_critical` | SOC critique | Critical | `snap.soc` | < 15 % | > 17 % | 3 min | 5 min |
| `temp_high` | Sur-température | Critical | `max_cell_temperature` | > 45 °C | < 43 °C | 3 min | 5 min |
| `high_current` | Sur-courant décharge | Warning | SmartShunt¹ ou BMS | > 100 A | < 95 A | 2 min | 1 min |
| `pack_ovp` | Tension pack trop haute | Critical | `snap.dc.voltage` | > 57.0 V | < 56.5 V | 1 min | 5 min |
| `pack_uvp` | Tension pack trop basse | Critical | `snap.dc.voltage` | < 44.0 V | > 44.5 V | 1 min | 5 min |

¹ `high_current` utilise `|shunt_current_a|` du Victron SmartShunt en priorité ; si non disponible, fallback sur `|snap.dc.current|`.

### 3.3 Diagramme de séquence d'une règle avec `for:`

```
Temps →
Condition vraie :  ───────────────────────────────────────────
                   t0          t0+for          t0+for+ε
                               │
pending_since = t0             │ elapsed ≥ for → déclenche
                               │ state.active = true
                               │ log_event("triggered")
                               │ Telegram ← notif

Condition fausse :
                   pending_since = None (timer réinitialisé)
```

---

## 4. Configuration `Config.toml`

```toml
[alerts]
# Chemin de la base SQLite. Laisser vide pour désactiver.
db_path = "/var/lib/daly-bms/alerts.db"

# Durées minimales "for:" avant déclenchement (secondes). 0 = immédiat.
soc_critical_for_secs = 180   # 3 minutes
soc_low_for_secs      = 300   # 5 minutes
temp_high_for_secs    = 180   # 3 minutes
current_high_for_secs = 120   # 2 minutes
voltage_for_secs      = 60    # 1 minute

# Telegram (optionnel)
telegram_token   = ""
telegram_chat_id = ""

[alerts.thresholds]
cell_ovp_v           = 3.60   # V — sur-tension cellule
cell_uvp_v           = 2.90   # V — sous-tension cellule
cell_delta_mv        = 100    # mV — déséquilibre
soc_low_percent      = 20.0   # % — SOC warning
soc_critical_percent = 15.0   # % — SOC critique
temp_high_c          = 45.0   # °C — sur-température
current_high_a       = 100.0  # A — sur-courant (SmartShunt prioritaire)
pack_ovp_v           = 57.0   # V — tension pack trop haute
pack_uvp_v           = 44.0   # V — tension pack trop basse
```

### Valeurs par défaut (si section absente)

| Champ | Défaut | Unité |
|-------|--------|-------|
| `cell_ovp_v` | 3.60 | V |
| `cell_uvp_v` | 2.90 | V |
| `cell_delta_mv` | 100 | mV |
| `soc_low_percent` | 20.0 | % |
| `soc_critical_percent` | 10.0 | % |
| `temp_high_c` | 45.0 | °C |
| `current_high_a` | 80.0 | A |
| `pack_ovp_v` | 57.0 | V |
| `pack_uvp_v` | 44.0 | V |
| `*_for_secs` | 0 | s (immédiat) |

---

## 5. Base de données SQLite

### Schéma `alert_events`

```sql
CREATE TABLE alert_events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    bms_address     INTEGER NOT NULL,
    rule_id         TEXT    NOT NULL,
    description     TEXT    NOT NULL DEFAULT '',
    severity        TEXT    NOT NULL DEFAULT 'warning',
    event           TEXT    NOT NULL,          -- 'triggered' | 'cleared'
    value           REAL    NOT NULL,
    timestamp       TEXT    NOT NULL,          -- datetime('now') UTC
    acknowledged    INTEGER NOT NULL DEFAULT 0,
    acknowledged_at TEXT                       -- NULL si non acquittée
);

-- Index
CREATE INDEX idx_alert_ts       ON alert_events(timestamp);
CREATE INDEX idx_alert_bms_rule ON alert_events(bms_address, rule_id);
CREATE INDEX idx_alert_event    ON alert_events(event);
CREATE INDEX idx_alert_ack      ON alert_events(acknowledged);
```

### Migration backward-compatible

Au démarrage, `init_db()` tente silencieusement d'ajouter les colonnes absentes :
```sql
ALTER TABLE alert_events ADD COLUMN description     TEXT    NOT NULL DEFAULT '';
ALTER TABLE alert_events ADD COLUMN severity        TEXT    NOT NULL DEFAULT 'warning';
ALTER TABLE alert_events ADD COLUMN acknowledged    INTEGER NOT NULL DEFAULT 0;
ALTER TABLE alert_events ADD COLUMN acknowledged_at TEXT;
```
Les erreurs `duplicate column` sont ignorées — une base existante est migrée sans perte de données.

---

## 6. API REST

Toutes les routes nécessitent `db_path` configuré (sinon `503 Service Unavailable`).

### `GET /api/v1/alerts/list`

Paramètres de requête :

| Param | Type | Défaut | Description |
|-------|------|--------|-------------|
| `limit` | usize | 100 | Max 500 |
| `offset` | usize | 0 | Pagination |
| `severity` | string | — | `critical` ou `warning` |
| `active` | bool | false | `true` = événements `triggered` seulement |

Réponse :
```json
{
  "total": 42,
  "events": [
    {
      "id": 17,
      "bms_address": 1,
      "rule_id": "soc_critical",
      "description": "SOC critique",
      "severity": "critical",
      "event": "triggered",
      "value": 13.5,
      "timestamp": "2026-05-01 14:32:11",
      "acknowledged": false,
      "acknowledged_at": null
    }
  ]
}
```

### `GET /api/v1/alerts/stats`

Réponse :
```json
{
  "total": 142,
  "triggered": 3,
  "cleared": 139,
  "critical": 78,
  "warning": 64,
  "unacknowledged": 2
}
```

### `POST /api/v1/alerts/:id/acknowledge`

Acquitte l'alerte `id`. Retourne `200 OK` ou `404 Not Found`.

---

## 7. Dashboard web

URL : `/dashboard/alerts`

Lien dans la sidebar : **🔔 Alertes** (section Système).

Fonctionnalités :
- Stats bar : Actives / Non acquittées / Critiques total / Total journalisés
- Filtres : Sévérité (Toutes / Critique / Warning) + Statut (Tous / Actives seulement)
- Liste paginée (50 événements/page) avec tri anti-chronologique
- Bouton **Acquitter** par événement (si `triggered` + non acquittée)
- Auto-refresh toutes les 30 secondes

Indicateur de statut (badge topbar) :
- 🔴 rouge : N alerte(s) active(s)
- 🟢 vert : Aucune alerte active

---

## 8. Notifications Telegram

Format du message envoyé :

```
🔴 Alerte DÉCLENCHÉE — BMS 0x01
Règle : SOC critique
Valeur : 13.50
Sévérité : critical
```

```
✅ Alerte EFFACÉE — BMS 0x01
Règle : SOC critique
Valeur : 0.00
Sévérité : critical
```

Pour activer : renseigner `telegram_token` et `telegram_chat_id` dans `Config.toml`.

---

## 9. Intégration dans le code

### `main.rs` — création et démarrage

```rust
let alert_engine = if !config.alerts.db_path.is_empty() {
    std::fs::create_dir_all(parent)?;
    match alerts::AlertEngine::new(config.alerts.clone()) {
        Ok(e)  => Some(e),
        Err(e) => { warn!("AlertEngine: {}", e); None }
    }
} else { None };

let state = AppState::new(config.clone(), log_buffer, vm_handle, alert_engine.clone());

if let Some(ref engine) = alert_engine {
    tokio::spawn(alerts::run_alert_engine(state.clone(), engine.clone()));
}
```

### `state.rs` — champ AppState

```rust
pub alert_engine: Option<Arc<AlertEngine>>,
```

### API — pattern `spawn_blocking` (rusqlite est synchrone)

```rust
let engine = state.alert_engine.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?.clone();
let result = spawn_blocking(move || engine.query_events(limit, offset, only_active, severity))
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??;
```

---

## 10. Ajouter ou modifier une règle

Toutes les règles sont dans `build_rules()` dans `bridges/alerts.rs`.

Structure d'une `AlertRule` :

```rust
AlertRule {
    id:           "ma_regle",          // identifiant unique, snake_case
    description:  "Description lisible",
    severity:     Severity::Warning,   // ou Severity::Critical
    cooldown:     Duration::from_secs(300),
    min_duration: dur_opt(cfg.xxx_for_secs), // None = immédiat
    trigger: Box::new(|ctx| {
        // Retourner Some(valeur) si la règle doit se déclencher
        let v = ctx.snap.dc.voltage;
        if v > ctx.cfg.thresholds.ma_valeur { Some(v) } else { None }
    }),
    clear: Box::new(|ctx| {
        // Retourner true si la règle doit s'effacer (avec hysteresis)
        ctx.snap.dc.voltage < ctx.cfg.thresholds.ma_valeur - 0.5
    }),
}
```

Contexte disponible dans les closures :

| Champ | Type | Description |
|-------|------|-------------|
| `ctx.snap` | `&BmsSnapshot` | Snapshot complet du BMS |
| `ctx.snap.soc` | `f32` | SOC en % |
| `ctx.snap.dc.voltage` | `f32` | Tension pack (V) |
| `ctx.snap.dc.current` | `f32` | Courant BMS (A, + = charge) |
| `ctx.snap.system.max_cell_voltage` | `f32` | Tension max cellule (V) |
| `ctx.snap.system.min_cell_voltage` | `f32` | Tension min cellule (V) |
| `ctx.snap.system.max_cell_temperature` | `f32` | Température max (°C) |
| `ctx.snap.system.cell_delta_mv()` | `f32` | Delta cellules (mV) |
| `ctx.cfg.thresholds.*` | `f32` | Seuils depuis Config.toml |
| `ctx.shunt_current_a` | `Option<f32>` | Courant SmartShunt (A) |

Pour ajouter un nouveau seuil configurable : ajouter le champ dans `AlertThresholds` (config.rs), le déclarer dans `Config.toml`, puis l'utiliser dans la closure.

---

## 11. Fichiers concernés

| Fichier | Rôle |
|---------|------|
| `crates/daly-bms-server/src/bridges/alerts.rs` | Moteur complet : règles, évaluation, SQLite, Telegram |
| `crates/daly-bms-server/src/config.rs` | `AlertsConfig`, `AlertThresholds` |
| `crates/daly-bms-server/src/state.rs` | `AppState.alert_engine: Option<Arc<AlertEngine>>` |
| `crates/daly-bms-server/src/main.rs` | Création AlertEngine + spawn tâche |
| `crates/daly-bms-server/src/api/alerts.rs` | Handlers REST list / stats / acknowledge |
| `crates/daly-bms-server/src/api/mod.rs` | Routes `/api/v1/alerts/*` |
| `crates/daly-bms-server/src/dashboard/mod.rs` | Route `/dashboard/alerts` |
| `crates/daly-bms-server/templates/alerts.html` | Template Askama du dashboard |
| `Config.toml` | Section `[alerts]` + `[alerts.thresholds]` |
