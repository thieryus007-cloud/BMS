//! Moteur d'alertes avec hysteresis, durée minimale ("for:"), journal SQLite,
//! notifications Telegram/SMTP.
//!
//! ## Architecture
//!
//! - [`AlertEngine`] évalue chaque snapshot contre les règles configurées.
//! - Chaque règle a un seuil de déclenchement et un seuil d'effacement (hysteresis).
//! - `min_duration` : la condition doit rester vraie pendant X secondes avant de déclencher.
//! - Les événements sont journalisés dans SQLite (table `alert_events`).
//! - Les notifications sont envoyées par Telegram avec cooldown.
//! - Les méthodes `query_events`, `count_stats`, `acknowledge` permettent l'accès API.

use crate::config::AlertsConfig;
use crate::state::AppState;
use daly_bms_core::types::BmsSnapshot;
use rusqlite::{Connection, params};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

// =============================================================================
// Types publics pour l'API REST
// =============================================================================

/// Un événement d'alerte journalisé dans SQLite.
#[derive(Debug, Clone, Serialize)]
pub struct AlertEventRow {
    pub id:              i64,
    pub bms_address:     u8,
    pub rule_id:         String,
    pub description:     String,
    pub severity:        String,
    pub event:           String,
    pub value:           f64,
    pub timestamp:       String,
    pub acknowledged:    bool,
    pub acknowledged_at: Option<String>,
}

/// Compteurs agrégés pour le dashboard.
#[derive(Debug, Clone, Serialize)]
pub struct AlertStats {
    pub total:           i64,
    pub triggered:       i64,
    pub cleared:         i64,
    pub critical:        i64,
    pub warning:         i64,
    pub unacknowledged:  i64,
}

// =============================================================================
// Règles d'alerte
// =============================================================================

/// Sévérité d'une alerte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Critical,
}

impl Severity {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Warning  => "warning",
            Self::Critical => "critical",
        }
    }
}

/// État d'une règle pour une clé (adresse BMS, rule_id).
#[derive(Debug, Clone)]
struct RuleState {
    /// Règle actuellement déclenchée (notification envoyée).
    active:         bool,
    /// Dernière notification envoyée (pour cooldown).
    last_notified:  Option<Instant>,
    /// Instant où la condition a commencé à être vraie (pour `min_duration`).
    pending_since:  Option<Instant>,
}

impl Default for RuleState {
    fn default() -> Self {
        Self {
            active:        false,
            last_notified: None,
            pending_since: None,
        }
    }
}

/// Contexte d'évaluation passé à chaque règle.
pub struct AlertContext<'a> {
    pub snap:             &'a BmsSnapshot,
    pub cfg:              &'a AlertsConfig,
    /// Courant SmartShunt (A) — positif = charge, négatif = décharge.
    pub shunt_current_a:  Option<f32>,
}

/// Définition d'une règle d'alerte.
pub struct AlertRule {
    pub id:           &'static str,
    pub description:  &'static str,
    pub severity:     Severity,
    pub cooldown:     Duration,
    /// Durée minimale pendant laquelle la condition doit rester vraie avant de déclencher.
    /// `None` = déclencher immédiatement (comportement legacy).
    pub min_duration: Option<Duration>,
    pub trigger:      Box<dyn Fn(&AlertContext) -> Option<f32> + Send + Sync>,
    pub clear:        Box<dyn Fn(&AlertContext) -> bool + Send + Sync>,
}

// =============================================================================
// AlertEngine
// =============================================================================

pub struct AlertEngine {
    rules:  Vec<AlertRule>,
    states: Mutex<HashMap<(u8, &'static str), RuleState>>,
    db:     Mutex<Connection>,
    pub cfg: AlertsConfig,
}

impl AlertEngine {
    /// Crée le moteur d'alertes et initialise la base SQLite.
    pub fn new(cfg: AlertsConfig) -> anyhow::Result<Arc<Self>> {
        let db = Connection::open(&cfg.db_path)?;
        init_db(&db)?;

        let engine = Arc::new(Self {
            rules:  build_rules(&cfg),
            states: Mutex::new(HashMap::new()),
            db:     Mutex::new(db),
            cfg,
        });
        Ok(engine)
    }

    /// Évalue toutes les règles sur un snapshot et envoie les notifications.
    ///
    /// `shunt_current_a` : courant SmartShunt courant (None si non disponible).
    pub async fn evaluate(&self, snap: &BmsSnapshot, shunt_current_a: Option<f32>) {
        let ctx = AlertContext { snap, cfg: &self.cfg, shunt_current_a };

        // Collecter les actions (sans tenir le Mutex pendant les await)
        let mut notifications: Vec<(u8, &'static str, &'static str, &'static str, f32, bool)> = Vec::new();

        {
            let mut states = self.states.lock().unwrap();
            for rule in &self.rules {
                let key   = (snap.address, rule.id);
                let state = states.entry(key).or_default();

                if !state.active {
                    if let Some(value) = (rule.trigger)(&ctx) {
                        if let Some(min_dur) = rule.min_duration {
                            // Condition vraie — démarrer ou continuer le timer pending
                            let ps = state.pending_since.get_or_insert_with(Instant::now);
                            if ps.elapsed() >= min_dur {
                                // Durée atteinte → déclencher
                                state.active        = true;
                                state.pending_since = None;
                                let should_notify = state.last_notified
                                    .map(|t| t.elapsed() >= rule.cooldown)
                                    .unwrap_or(true);
                                if should_notify {
                                    state.last_notified = Some(Instant::now());
                                    notifications.push((snap.address, rule.id, rule.description, rule.severity.as_str(), value, true));
                                }
                            }
                            // Sinon : toujours en attente, pas encore déclenché
                        } else {
                            // Pas de durée minimale → déclencher immédiatement
                            state.active        = true;
                            state.pending_since = None;
                            let should_notify = state.last_notified
                                .map(|t| t.elapsed() >= rule.cooldown)
                                .unwrap_or(true);
                            if should_notify {
                                state.last_notified = Some(Instant::now());
                                notifications.push((snap.address, rule.id, rule.description, rule.severity.as_str(), value, true));
                            }
                        }
                    } else {
                        // Condition non vraie → réinitialiser le timer pending
                        state.pending_since = None;
                    }
                } else if (rule.clear)(&ctx) {
                    state.active        = false;
                    state.pending_since = None;
                    notifications.push((snap.address, rule.id, rule.description, rule.severity.as_str(), 0.0, false));
                }
            }
        } // Mutex relâché ici

        // Envoyer notifications hors du lock
        for (addr, rule_id, description, severity, value, triggered) in notifications {
            let event = if triggered { "triggered" } else { "cleared" };
            self.log_event(addr, rule_id, description, severity, event, value);

            let icon   = if triggered { "🔴" } else { "✅" };
            let action = if triggered { "DÉCLENCHÉE" } else { "EFFACÉE" };
            let msg = format!(
                "{} Alerte {} — BMS {:#04x}\nRègle : {}\nValeur : {:.2}\nSévérité : {}",
                icon, action, addr, description, value, severity
            );
            info!("{}", msg);
            if !self.cfg.telegram_token.is_empty() && !self.cfg.telegram_chat_id.is_empty() {
                if let Err(e) = send_telegram(&self.cfg.telegram_token, &self.cfg.telegram_chat_id, &msg).await {
                    error!("Telegram erreur : {:?}", e);
                }
            }
        }
    }

    fn log_event(&self, addr: u8, rule_id: &str, description: &str, severity: &str, event: &str, value: f32) {
        if let Ok(db) = self.db.lock() {
            let _ = db.execute(
                "INSERT INTO alert_events \
                 (bms_address, rule_id, description, severity, event, value, timestamp) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))",
                params![addr, rule_id, description, severity, event, value],
            );
        }
    }

    // -------------------------------------------------------------------------
    // Méthodes publiques pour l'API REST
    // -------------------------------------------------------------------------

    /// Liste les événements avec pagination et filtres optionnels.
    /// Retourne (total_count, events).
    pub fn query_events(
        &self,
        limit:         usize,
        offset:        usize,
        only_active:   bool,
        severity_filter: Option<String>,
    ) -> rusqlite::Result<(i64, Vec<AlertEventRow>)> {
        let db = self.db.lock().unwrap();

        let mut where_parts: Vec<String> = Vec::new();
        if only_active {
            where_parts.push("event = 'triggered'".to_string());
        }
        if let Some(ref sev) = severity_filter {
            where_parts.push(format!("severity = '{}'", sev.replace('\'', "''")));
        }
        let where_clause = if where_parts.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_parts.join(" AND "))
        };

        let count: i64 = db.query_row(
            &format!("SELECT COUNT(*) FROM alert_events {}", where_clause),
            [],
            |r| r.get(0),
        )?;

        let mut stmt = db.prepare(&format!(
            "SELECT id, bms_address, rule_id, description, severity, event, value, \
                    timestamp, acknowledged, acknowledged_at \
             FROM alert_events {} \
             ORDER BY timestamp DESC \
             LIMIT {} OFFSET {}",
            where_clause, limit, offset
        ))?;

        let rows = stmt.query_map([], |row| {
            Ok(AlertEventRow {
                id:              row.get(0)?,
                bms_address:     row.get::<_, i64>(1)? as u8,
                rule_id:         row.get(2)?,
                description:     row.get(3)?,
                severity:        row.get(4)?,
                event:           row.get(5)?,
                value:           row.get(6)?,
                timestamp:       row.get(7)?,
                acknowledged:    row.get::<_, i64>(8)? != 0,
                acknowledged_at: row.get(9)?,
            })
        })?.collect::<rusqlite::Result<Vec<_>>>()?;

        Ok((count, rows))
    }

    /// Compteurs agrégés pour le dashboard.
    pub fn count_stats(&self) -> rusqlite::Result<AlertStats> {
        let db = self.db.lock().unwrap();

        let q = |sql: &str| -> rusqlite::Result<i64> {
            db.query_row(sql, [], |r| r.get(0))
        };

        Ok(AlertStats {
            total:          q("SELECT COUNT(*) FROM alert_events")?,
            triggered:      q("SELECT COUNT(*) FROM alert_events WHERE event = 'triggered'")?,
            cleared:        q("SELECT COUNT(*) FROM alert_events WHERE event = 'cleared'")?,
            critical:       q("SELECT COUNT(*) FROM alert_events WHERE severity = 'critical'")?,
            warning:        q("SELECT COUNT(*) FROM alert_events WHERE severity = 'warning'")?,
            unacknowledged: q("SELECT COUNT(*) FROM alert_events WHERE acknowledged = 0 AND event = 'triggered'")?,
        })
    }

    /// Marque une alerte comme acquittée. Retourne `true` si l'enregistrement existe.
    pub fn acknowledge(&self, id: i64) -> rusqlite::Result<bool> {
        let db = self.db.lock().unwrap();
        let n = db.execute(
            "UPDATE alert_events \
             SET acknowledged = 1, acknowledged_at = datetime('now') \
             WHERE id = ?1",
            [id],
        )?;
        Ok(n > 0)
    }
}

// =============================================================================
// Règles par défaut
// =============================================================================

fn dur_opt(secs: u64) -> Option<Duration> {
    if secs > 0 { Some(Duration::from_secs(secs)) } else { None }
}

fn build_rules(cfg: &AlertsConfig) -> Vec<AlertRule> {
    let t = cfg.thresholds.clone();

    vec![
        // ── Règles cellule BMS ────────────────────────────────────────────────
        AlertRule {
            id:           "cell_ovp",
            description:  "Sur-tension cellule",
            severity:     Severity::Critical,
            cooldown:     Duration::from_secs(300),
            min_duration: None,
            trigger: Box::new(move |ctx| {
                let v = ctx.snap.system.max_cell_voltage;
                if v > ctx.cfg.thresholds.cell_ovp_v { Some(v) } else { None }
            }),
            clear: Box::new(move |ctx| {
                ctx.snap.system.max_cell_voltage < t.cell_ovp_v - 0.05
            }),
        },
        AlertRule {
            id:           "cell_uvp",
            description:  "Sous-tension cellule",
            severity:     Severity::Critical,
            cooldown:     Duration::from_secs(300),
            min_duration: None,
            trigger: Box::new(|ctx| {
                let v = ctx.snap.system.min_cell_voltage;
                if v < ctx.cfg.thresholds.cell_uvp_v { Some(v) } else { None }
            }),
            clear: Box::new(|ctx| {
                ctx.snap.system.min_cell_voltage > ctx.cfg.thresholds.cell_uvp_v + 0.05
            }),
        },
        AlertRule {
            id:           "cell_imbalance",
            description:  "Déséquilibre cellules",
            severity:     Severity::Warning,
            cooldown:     Duration::from_secs(600),
            min_duration: None,
            trigger: Box::new(|ctx| {
                let d = ctx.snap.system.cell_delta_mv();
                if d > ctx.cfg.thresholds.cell_delta_mv { Some(d) } else { None }
            }),
            clear: Box::new(|ctx| {
                ctx.snap.system.cell_delta_mv() < ctx.cfg.thresholds.cell_delta_mv - 10.0
            }),
        },

        // ── Règles ESS niveau pack ────────────────────────────────────────────
        AlertRule {
            id:           "soc_low",
            description:  "SOC bas",
            severity:     Severity::Warning,
            cooldown:     Duration::from_secs(900),
            min_duration: dur_opt(cfg.soc_low_for_secs),
            trigger: Box::new(|ctx| {
                let s = ctx.snap.soc;
                if s < ctx.cfg.thresholds.soc_low_percent { Some(s) } else { None }
            }),
            clear: Box::new(|ctx| {
                ctx.snap.soc > ctx.cfg.thresholds.soc_low_percent + 5.0
            }),
        },
        AlertRule {
            id:           "soc_critical",
            description:  "SOC critique",
            severity:     Severity::Critical,
            cooldown:     Duration::from_secs(300),
            min_duration: dur_opt(cfg.soc_critical_for_secs),
            trigger: Box::new(|ctx| {
                let s = ctx.snap.soc;
                if s < ctx.cfg.thresholds.soc_critical_percent { Some(s) } else { None }
            }),
            clear: Box::new(|ctx| {
                ctx.snap.soc > ctx.cfg.thresholds.soc_critical_percent + 2.0
            }),
        },
        AlertRule {
            id:           "temp_high",
            description:  "Sur-température batterie",
            severity:     Severity::Critical,
            cooldown:     Duration::from_secs(300),
            min_duration: dur_opt(cfg.temp_high_for_secs),
            trigger: Box::new(|ctx| {
                let t = ctx.snap.system.max_cell_temperature;
                if t > ctx.cfg.thresholds.temp_high_c { Some(t) } else { None }
            }),
            clear: Box::new(|ctx| {
                ctx.snap.system.max_cell_temperature < ctx.cfg.thresholds.temp_high_c - 2.0
            }),
        },
        AlertRule {
            id:           "high_current",
            description:  "Sur-courant décharge",
            severity:     Severity::Warning,
            cooldown:     Duration::from_secs(60),
            min_duration: dur_opt(cfg.current_high_for_secs),
            trigger: Box::new(|ctx| {
                // Priorité SmartShunt ; fallback courant BMS
                let c = ctx.shunt_current_a
                    .map(|v| v.abs())
                    .unwrap_or_else(|| ctx.snap.dc.current.abs());
                if c > ctx.cfg.thresholds.current_high_a { Some(c) } else { None }
            }),
            clear: Box::new(|ctx| {
                let c = ctx.shunt_current_a
                    .map(|v| v.abs())
                    .unwrap_or_else(|| ctx.snap.dc.current.abs());
                c < ctx.cfg.thresholds.current_high_a - 5.0
            }),
        },
        AlertRule {
            id:           "pack_ovp",
            description:  "Tension pack trop haute",
            severity:     Severity::Critical,
            cooldown:     Duration::from_secs(300),
            min_duration: dur_opt(cfg.voltage_for_secs),
            trigger: Box::new(|ctx| {
                let v = ctx.snap.dc.voltage;
                if v > ctx.cfg.thresholds.pack_ovp_v { Some(v) } else { None }
            }),
            clear: Box::new(|ctx| {
                ctx.snap.dc.voltage < ctx.cfg.thresholds.pack_ovp_v - 0.5
            }),
        },
        AlertRule {
            id:           "pack_uvp",
            description:  "Tension pack trop basse",
            severity:     Severity::Critical,
            cooldown:     Duration::from_secs(300),
            min_duration: dur_opt(cfg.voltage_for_secs),
            trigger: Box::new(|ctx| {
                let v = ctx.snap.dc.voltage;
                if v < ctx.cfg.thresholds.pack_uvp_v { Some(v) } else { None }
            }),
            clear: Box::new(|ctx| {
                ctx.snap.dc.voltage > ctx.cfg.thresholds.pack_uvp_v + 0.5
            }),
        },
    ]
}

// =============================================================================
// Notifications
// =============================================================================

async fn send_telegram(token: &str, chat_id: &str, message: &str) -> anyhow::Result<()> {
    let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
    let client = reqwest::Client::new();
    client
        .post(&url)
        .json(&serde_json::json!({
            "chat_id":    chat_id,
            "text":       message,
            "parse_mode": "HTML",
        }))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

// =============================================================================
// Initialisation SQLite
// =============================================================================

fn init_db(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS alert_events (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            bms_address     INTEGER NOT NULL,
            rule_id         TEXT    NOT NULL,
            description     TEXT    NOT NULL DEFAULT '',
            severity        TEXT    NOT NULL DEFAULT 'warning',
            event           TEXT    NOT NULL,
            value           REAL    NOT NULL,
            timestamp       TEXT    NOT NULL,
            acknowledged    INTEGER NOT NULL DEFAULT 0,
            acknowledged_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_alert_ts
            ON alert_events(timestamp);
        CREATE INDEX IF NOT EXISTS idx_alert_bms_rule
            ON alert_events(bms_address, rule_id);
        CREATE INDEX IF NOT EXISTS idx_alert_event
            ON alert_events(event);
        CREATE INDEX IF NOT EXISTS idx_alert_ack
            ON alert_events(acknowledged);
    ")?;

    // Migration pour les bases existantes (ADD COLUMN échoue silencieusement si déjà là)
    let _ = conn.execute("ALTER TABLE alert_events ADD COLUMN description TEXT NOT NULL DEFAULT ''", []);
    let _ = conn.execute("ALTER TABLE alert_events ADD COLUMN severity TEXT NOT NULL DEFAULT 'warning'", []);
    let _ = conn.execute("ALTER TABLE alert_events ADD COLUMN acknowledged INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE alert_events ADD COLUMN acknowledged_at TEXT", []);

    Ok(())
}

// =============================================================================
// Tâche principale
// =============================================================================

/// Démarre la boucle d'évaluation en arrière-plan.
/// Accepte un `Arc<AlertEngine>` déjà créé (partagé avec l'API REST via AppState).
pub async fn run_alert_engine(state: AppState, engine: Arc<AlertEngine>) {
    info!(db = %engine.cfg.db_path, "AlertEngine démarré");

    let mut rx = state.subscribe_ws();
    loop {
        match rx.recv().await {
            Ok(snaps) => {
                // Lire le courant SmartShunt une seule fois pour tous les BMS
                let shunt_current_a = state.venus_smartshunt_get().await
                    .and_then(|s| s.current_a);

                for snap in snaps.iter() {
                    engine.evaluate(snap, shunt_current_a).await;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                warn!("AlertEngine : {} snapshots manqués", n);
            }
            Err(_) => break,
        }
    }
}
