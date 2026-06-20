/// DEYE relay control via Shelly Pro 2PM (MQTT RPC).
/// State machine: On → PendingCut (15s) → Lockout (120s) → Off → PendingRestore (45s) → On
/// State transitions are decided by rust-rule-engine (rules/deye_command.grl).
/// State is persisted to santuario/persist/deye_state (retained MQTT) to survive restarts.
mod rules;

use chrono::{DateTime, Utc};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tokio::time::{interval, sleep, Duration};
use tracing::{info, warn};

use crate::bus::AppBus;
use crate::config::{DeyeConfig, VictronConfig};
use crate::mqtt::topics::publish;
use crate::rules_loader::RulesLoader;
use crate::types::{EnergyState, MqttOutgoing};

#[derive(Debug, Clone, Copy, PartialEq)]
enum DeyeState {
    On,
    PendingCut(DateTime<Utc>),
    Off,
    PendingRestore(DateTime<Utc>),
    Lockout(DateTime<Utc>),
}

fn state_name(s: &DeyeState) -> &'static str {
    match s {
        DeyeState::On                => "On",
        DeyeState::PendingCut(_)     => "PendingCut",
        DeyeState::Off               => "Off",
        DeyeState::PendingRestore(_) => "PendingRestore",
        DeyeState::Lockout(_)        => "Lockout",
    }
}

fn time_in_state_secs(s: &DeyeState, now: DateTime<Utc>) -> u64 {
    match s {
        DeyeState::PendingCut(since) | DeyeState::PendingRestore(since) => {
            (now - *since).num_seconds().max(0) as u64
        }
        _ => 0,
    }
}

fn lockout_expired(s: &DeyeState, now: DateTime<Utc>) -> bool {
    if let DeyeState::Lockout(until) = s {
        now >= *until
    } else {
        false
    }
}

pub async fn spawn(
    vic: Arc<VictronConfig>,
    cfg: DeyeConfig,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
    loader: Arc<RulesLoader>,
) {
    crate::supervise::spawn_critical(run(vic, cfg, bus, state, loader));
}

async fn run(
    vic: Arc<VictronConfig>,
    cfg: DeyeConfig,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
    loader: Arc<RulesLoader>,
) {
    let pid = &vic.portal_id;
    let vb  = vic.vebus_instance;

    let t_freq      = format!("N/{pid}/vebus/{vb}/Ac/Out/L1/F");
    let t_connected = format!("N/{pid}/vebus/{vb}/Ac/ActiveIn/Connected");

    let shelly_id = &vic.shelly_deye_id;
    // One channel per DEYE. Prefer the multi-channel list; fall back to the
    // legacy single-channel field for backward compatibility.
    let channels: Vec<u8> = if vic.shelly_deye_channels.is_empty() {
        vec![vic.shelly_deye_channel]
    } else {
        vic.shelly_deye_channels.clone()
    };

    if shelly_id.is_empty() {
        info!("DEYE control disabled — shelly_deye_id not configured");
        return;
    }
    info!("DEYE control: shelly={shelly_id} channels={channels:?}");

    let src = loader.load("deye_command");
    let mut rule_engine = match rules::DeyeRuleEngine::with_source(&src) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Failed to init DEYE rule engine: {e}");
            return;
        }
    };

    // Restore persisted state — wait up to 3s for retained MQTT message.
    // This prevents spurious relay-on when restarting while DEYE is cut.
    let initial_state = {
        sleep(Duration::from_secs(3)).await;
        let s = state.read().await;
        match s.deye_persisted_state.as_deref() {
            Some("Off") | Some("Lockout") | Some("PendingRestore") => {
                info!("DEYE: restoring state=Off from retained MQTT");
                DeyeState::Off
            }
            Some(other) => {
                info!("DEYE: starting with state=On (persisted={other})");
                DeyeState::On
            }
            None => {
                info!("DEYE: no persisted state — starting with On");
                DeyeState::On
            }
        }
    };

    let mut deye_sm   = initial_state;
    let mut last_freq: f64 = 50.0;
    // Timestamp since which the MPPT-full condition has held continuously (debounce).
    let mut mppt_full_since: Option<DateTime<Utc>> = None;
    let mut rx        = bus.subscribe_mqtt();
    let mut reload_rx = bus.subscribe_rule_reload();
    let mut ticker    = interval(Duration::from_secs(1));
    // Periodic idempotent re-assert of the relay state on every channel, so the
    // physical Shelly reconverges after a missed command or a reboot. The first
    // tick fires immediately, asserting the restored state at startup.
    let mut resync    = interval(Duration::from_secs(cfg.relay_resync_secs.max(1)));

    loop {
        tokio::select! {
            result = rx.recv() => {
                let msg = match result {
                    Ok(m) => m,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("deye_command MQTT subscriber lagged, dropped {n} message(s)");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                let t = &msg.topic;

                if *t == t_freq {
                    if let Some(freq) = msg.victron_value::<f64>() {
                        last_freq = freq;

                        let now = Utc::now();
                        // Réaction immédiate : `freq` du message est la valeur la plus fraîche
                        // pour ce chemin (le ticker, lui, recale sur `ac_frequency_hz`) → pas de
                        // garde de staleness fréquence ici. La garde MPPT s'applique néanmoins :
                        // un état MPPT figé « plein » ne doit pas bloquer la restauration.
                        let inp = read_deye_inputs(&state, &cfg, now).await;
                        let mppt_full = effective_mppt_full(inp.mppt_full, inp.mppt_stale);
                        // Keep the MPPT-cut debounce in sync with the ticker so a nominal
                        // frequency update cannot cancel an active MPPT-driven cut.
                        mppt_full_since = if mppt_full { Some(mppt_full_since.unwrap_or(now)) } else { None };
                        let mppt_cut = mppt_full_since
                            .map(|t| (now - t).num_seconds().max(0) as u64 >= cfg.mppt_cut_delay_secs)
                            .unwrap_or(false);

                        let new_state = apply_decision(
                            rule_engine.evaluate(
                                state_name(&deye_sm),
                                last_freq,
                                time_in_state_secs(&deye_sm, now),
                                cfg.freq_high_hz,
                                cfg.freq_hard_hz,
                                cfg.cut_delay_secs,
                                cfg.reenable_delay_secs,
                                lockout_expired(&deye_sm, now),
                                mppt_full,
                                mppt_cut,
                            ),
                            deye_sm,
                            now,
                            &cfg,
                            &bus,
                            shelly_id,
                            &channels,
                        ).await;
                        if new_state != deye_sm {
                            deye_sm = new_state;
                            persist_deye_state(&bus, &deye_sm).await;
                            update_deye_state(&state, &deye_sm).await;
                        }
                    }

                } else if *t == t_connected {
                    // Grid status no longer takes part in the DEYE decision (Fréquence + MPPT
                    // only). Still tracked so the dashboard "Réseau" row stays meaningful.
                    if let Some(v) = msg.victron_value::<i64>() {
                        state.write().await.ac_connected = Some(v);
                    }
                }
            }

            _ = ticker.tick() => {
                let now = Utc::now();
                // Source de vérité UNIQUE + GARDES DE FRAÎCHEUR (anti-blocage relais).
                // - fréquence : préférer la valeur partagée `ac_frequency_hz` (= widget) à la
                //   `last_freq` locale (qui peut rester figée haut si l'abonnement deye se tarit) ;
                //   si la télémétrie est périmée, on la traite comme nominale (restauration permise).
                // - MPPT : un état figé « plein » (topic muet) ne doit plus bloquer la restauration.
                let inp = read_deye_inputs(&state, &cfg, now).await;
                last_freq = decision_freq(inp.shared_freq, last_freq);
                let eff_freq = effective_freq(last_freq, inp.freq_stale);
                let mppt_full = effective_mppt_full(inp.mppt_full, inp.mppt_stale);
                // Debounce the MPPT-full signal before it is allowed to cut the DEYE.
                mppt_full_since = if mppt_full { Some(mppt_full_since.unwrap_or(now)) } else { None };
                let mppt_cut = mppt_full_since
                    .map(|t| (now - t).num_seconds().max(0) as u64 >= cfg.mppt_cut_delay_secs)
                    .unwrap_or(false);
                // Refresh observability fields (1 Hz) for /api/rules-status.
                // deye_on is synced here too (not only on transition) so it can never
                // diverge from the state machine — e.g. at startup before any transition.
                // restore_blocked == effective mppt_full (battery-full per the MPPT stage, only
                // when fresh, is the only restore gate; Bulk/charging/stale unblocks it).
                {
                    let mut s = state.write().await;
                    s.deye_on              = matches!(deye_sm, DeyeState::On | DeyeState::PendingCut(_));
                    s.deye_state           = Some(state_name(&deye_sm).to_string());
                    s.deye_restore_blocked = mppt_full;
                    s.deye_mppt_full       = mppt_full;
                    s.deye_freq_stale      = inp.freq_stale;
                    s.deye_mppt_stale      = inp.mppt_stale;
                }
                let new_state = apply_decision(
                    rule_engine.evaluate(
                        state_name(&deye_sm),
                        eff_freq,
                        time_in_state_secs(&deye_sm, now),
                        cfg.freq_high_hz,
                        cfg.freq_hard_hz,
                        cfg.cut_delay_secs,
                        cfg.reenable_delay_secs,
                        lockout_expired(&deye_sm, now),
                        mppt_full,
                        mppt_cut,
                    ),
                    deye_sm,
                    now,
                    &cfg,
                    &bus,
                    shelly_id,
                    &channels,
                ).await;
                if new_state != deye_sm {
                    deye_sm = new_state;
                    persist_deye_state(&bus, &deye_sm).await;
                    update_deye_state(&state, &deye_sm).await;
                }
            }

            _ = resync.tick() => {
                // Re-assert the current logical relay state on every channel (idempotent).
                let desired_on = matches!(deye_sm, DeyeState::On | DeyeState::PendingCut(_));
                send_shelly(&bus, shelly_id, &channels, desired_on).await;
            }

            Ok(name) = reload_rx.recv() => {
                if name == "deye_command" || name == "*" {
                    let src = loader.load("deye_command");
                    match rules::DeyeRuleEngine::with_source(&src) {
                        Ok(e) => { rule_engine = e; info!("deye_command rule engine reloaded"); }
                        Err(e) => tracing::warn!("deye_command reload failed (keeping old engine): {e}"),
                    }
                }
            }
        }
    }
}

/// Publishes DEYE state as retained MQTT for persistence across restarts.
/// Only stable states are persisted: "On" and "Off".
async fn persist_deye_state(bus: &AppBus, state: &DeyeState) {
    let persisted = match state {
        DeyeState::On | DeyeState::PendingCut(_) => "On",
        DeyeState::Off | DeyeState::Lockout(_) | DeyeState::PendingRestore(_) => "Off",
    };
    bus.publish(MqttOutgoing::raw(
        publish::DEYE_STATE, persisted, true,
    )).await;
}

/// Updates EnergyState with DEYE relay info for the REST /api/rules-status endpoint.
async fn update_deye_state(state: &Arc<RwLock<EnergyState>>, deye: &DeyeState) {
    let mut s = state.write().await;
    s.deye_on            = matches!(deye, DeyeState::On | DeyeState::PendingCut(_));
    s.deye_state         = Some(state_name(deye).to_string());
    s.deye_lockout_until = match deye { DeyeState::Lockout(until) => Some(*until), _ => None };
    s.deye_last_change   = Some(Utc::now());
}

#[allow(clippy::too_many_arguments)]
async fn apply_decision(
    decision: anyhow::Result<rules::DeyeDecision>,
    current: DeyeState,
    now: DateTime<Utc>,
    cfg: &DeyeConfig,
    bus: &AppBus,
    shelly_id: &str,
    channels: &[u8],
) -> DeyeState {
    let d = match decision {
        Ok(d)  => d,
        Err(e) => {
            tracing::error!("DEYE rule engine error: {e}");
            return current;
        }
    };

    if d.relay_off {
        send_shelly(bus, shelly_id, channels, false).await;
    }
    if d.relay_on {
        send_shelly(bus, shelly_id, channels, true).await;
    }

    let Some(next_name) = d.next_state else {
        return current;
    };

    match next_name.as_str() {
        "On" => {
            info!("DEYE: → On");
            DeyeState::On
        }
        "Off" => {
            info!("DEYE: → Off");
            DeyeState::Off
        }
        "PendingCut" => {
            info!("DEYE: freq high — starting cut timer");
            DeyeState::PendingCut(now)
        }
        "PendingRestore" => {
            info!("DEYE: freq low — starting restore timer");
            DeyeState::PendingRestore(now)
        }
        "Lockout" => {
            let until = now + chrono::Duration::seconds(cfg.lockout_secs as i64);
            info!("DEYE: relay cut — lockout until {until}");
            DeyeState::Lockout(until)
        }
        other => {
            tracing::warn!("DEYE rule engine returned unknown state: {other}");
            current
        }
    }
}

/// Sends a `Switch.Set` to every DEYE channel (idempotent on the Shelly side).
/// Logged at DEBUG because it also runs on each periodic re-assert; real state
/// transitions are logged at INFO by `apply_decision`.
async fn send_shelly(bus: &AppBus, shelly_id: &str, channels: &[u8], on: bool) {
    let topic = publish::shelly_rpc(shelly_id);
    for &channel in channels {
        let payload = json!({
            "id":     1,
            "src":    "energy-manager",
            "method": "Switch.Set",
            "params": { "id": channel, "on": on }
        });
        bus.publish(MqttOutgoing::transient(topic.clone(), &payload)).await;
    }
    tracing::debug!("DEYE Shelly: channels {channels:?} = {}", if on { "ON" } else { "OFF" });
}

/// DEYE decision inputs read from shared state, with freshness flags.
struct DeyeInputs {
    /// Shared AC-Out frequency (`ac_frequency_hz`, same source as the widget).
    shared_freq: Option<f64>,
    /// AC-Out frequency telemetry is stale (topic silent > `input_max_age_secs`).
    freq_stale: bool,
    /// Battery topping/full per the MPPT charge stage (raw, before freshness guard).
    mppt_full: bool,
    /// MPPT State telemetry is stale (both chargers silent > `input_max_age_secs`).
    mppt_stale: bool,
}

/// Reads the DEYE decision inputs from shared state in a single lock, including the
/// freshness of each input (frequency + MPPT State). A frozen telemetry value (topic gone
/// silent while the loop stays alive) must never strand the relay — `apply_freshness`
/// turns these flags into a safe effective decision.
async fn read_deye_inputs(state: &Arc<RwLock<EnergyState>>, cfg: &DeyeConfig, now: DateTime<Utc>) -> DeyeInputs {
    let s = state.read().await;
    let max = cfg.input_max_age_secs as i64;
    // MPPT is fresh if at least one charger reported its State recently.
    let mppt_stale = [s.mppt_273.state_last_ts, s.mppt_289.state_last_ts]
        .into_iter()
        .flatten()
        .map(|t| (now - t).num_seconds())
        .min()
        .map(|age| age > max)
        .unwrap_or(true);
    DeyeInputs {
        shared_freq: s.ac_frequency_hz,
        freq_stale: !is_fresh(s.ac_frequency_last_ts, now, max),
        mppt_full: mppt_battery_full(&s, cfg),
        mppt_stale,
    }
}

/// Whether a timestamp is within `max_age` seconds of `now` (absent → not fresh).
fn is_fresh(ts: Option<DateTime<Utc>>, now: DateTime<Utc>, max_age: i64) -> bool {
    ts.map(|t| (now - t).num_seconds() <= max_age).unwrap_or(false)
}

/// Frequency the DEYE decision must use: prefer the shared, inverter-maintained
/// `ac_frequency_hz` (identical to the widget, always fresh) over the deye-local last
/// value, so the decision can never diverge from the display nor stay latched on a stale
/// high reading. Falls back to the local value when the shared one is not yet available.
fn decision_freq(shared: Option<f64>, local: f64) -> f64 {
    shared.unwrap_or(local)
}

/// Nominal AC frequency used when the real one is stale (well below any cut threshold).
const NOMINAL_FREQ_HZ: f64 = 50.0;

/// Effective frequency for the decision: when the telemetry is stale, treat it as nominal
/// (50 Hz) so the relay is NOT latched off on a frozen high reading — restore is allowed and
/// no frequency-based cut fires. Per ops policy, the DEYE 51.5 Hz hardware auto-trip is the
/// safety net while AC-frequency telemetry is unavailable.
fn effective_freq(freq_hz: f64, freq_stale: bool) -> f64 {
    if freq_stale { NOMINAL_FREQ_HZ } else { freq_hz }
}

/// Effective MPPT-full for the decision: a stale MPPT State is treated as NOT full, so a
/// frozen "battery full" reading can never strand the relay off (the bug class this guards).
fn effective_mppt_full(mppt_full: bool, mppt_stale: bool) -> bool {
    mppt_full && !mppt_stale
}

/// Battery topping/full according to the MPPT charge stage (Absorption/Float/Storage on
/// any solar charger). Pure solar-charger telemetry — works **without DVCC**. Bulk (3) is
/// NOT a full stage, so a charger in Bulk reports `false` → restore is allowed. Returns
/// false when the feature is disabled.
fn mppt_battery_full(s: &EnergyState, cfg: &DeyeConfig) -> bool {
    if !cfg.mppt_cut_enabled {
        return false;
    }
    [s.mppt_273.state, s.mppt_289.state]
        .into_iter()
        .flatten()
        .any(|st| cfg.mppt_full_states.contains(&st))
}

/// Whether the Victron is grid-tied (NOT islanded). **Informational only** — no longer part
/// of the DEYE decision (Fréquence + MPPT only); kept for the dashboard "Réseau" row.
pub(crate) fn is_grid_connected(ac_ignore: Option<i64>, ac_connected: Option<i64>) -> bool {
    ac_ignore != Some(1) && ac_connected != Some(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_tied_when_connected_and_not_ignored() {
        assert!(is_grid_connected(Some(0), Some(1)));
    }

    #[test]
    fn islanded_on_deliberate_ignore() {
        // ESS running off-battery while physically still connected → frequency shifting active.
        assert!(!is_grid_connected(Some(1), Some(1)));
    }

    #[test]
    fn islanded_on_real_outage_even_if_ignore_stays_zero() {
        // Real grid loss: ActiveIn/Connected drops to 0 while IgnoreAcIn1 may remain 0.
        assert!(!is_grid_connected(Some(0), Some(0)));
    }

    #[test]
    fn unknown_signals_assume_connected() {
        // Graceful degradation: no data → previous ac_ignore-only "assume connected" default.
        assert!(is_grid_connected(None, None));
        assert!(is_grid_connected(Some(0), None));
        assert!(!is_grid_connected(Some(1), None));
    }

    // --- mppt_battery_full : seul signal non-fréquentiel de la décision DEYE ----

    fn cfg_mppt_on() -> DeyeConfig {
        DeyeConfig { mppt_cut_enabled: true, ..Default::default() }
    }

    fn state_with_mppt(s273: Option<i64>, s289: Option<i64>) -> EnergyState {
        let mut s = EnergyState::default();
        s.mppt_273.state = s273;
        s.mppt_289.state = s289;
        s
    }

    #[test]
    fn both_mppt_bulk_not_full_restore_allowed() {
        // ← le cas signalé : les deux MPPT en Bulk (3) → batterie PAS pleine → restore autorisé.
        let s = state_with_mppt(Some(3), Some(3));
        assert!(!mppt_battery_full(&s, &cfg_mppt_on()));
    }

    #[test]
    fn any_mppt_full_blocks_restore() {
        // Un seul chargeur en Absorption/Float/Storage suffit à considérer la batterie pleine.
        assert!(mppt_battery_full(&state_with_mppt(Some(4), Some(3)), &cfg_mppt_on())); // Absorption
        assert!(mppt_battery_full(&state_with_mppt(Some(3), Some(5)), &cfg_mppt_on())); // Float
        assert!(mppt_battery_full(&state_with_mppt(Some(6), None),    &cfg_mppt_on())); // Storage
    }

    #[test]
    fn no_mppt_data_not_full() {
        assert!(!mppt_battery_full(&EnergyState::default(), &cfg_mppt_on()));
    }

    // --- decision_freq : source de vérité unique (anti-divergence widget/décision) ----

    #[test]
    fn decision_freq_prefers_fresh_shared_value() {
        // Bug terrain : la `last_freq` locale est restée figée HAUT (≥ 51) après le pic de
        // midi, tandis que la fréquence partagée (widget) est redescendue à 49,95 Hz.
        // La décision DOIT utiliser la valeur partagée fraîche → restauration possible.
        assert_eq!(decision_freq(Some(49.95), 51.4), 49.95);
    }

    #[test]
    fn decision_freq_falls_back_to_local_when_shared_absent() {
        // Pas encore de valeur partagée (démarrage) → repli sur la locale.
        assert_eq!(decision_freq(None, 50.0), 50.0);
    }

    // --- gardes de fraîcheur (anti-blocage relais sur télémétrie figée) ----------

    #[test]
    fn is_fresh_within_and_beyond_window() {
        let now = Utc::now();
        assert!(is_fresh(Some(now - chrono::Duration::seconds(30)), now, 90));   // récent
        assert!(!is_fresh(Some(now - chrono::Duration::seconds(120)), now, 90)); // périmé
        assert!(!is_fresh(None, now, 90));                                        // jamais vu
    }

    #[test]
    fn stale_freq_treated_as_nominal_allows_restore() {
        // Fréquence figée HAUT mais périmée → traitée comme nominale (50 Hz) → côté restauration,
        // aucune coupure fréquence. Le filet = auto-trip DEYE 51,5 Hz (politique ops).
        assert_eq!(effective_freq(51.4, true), NOMINAL_FREQ_HZ);
        // Fraîche → valeur réelle conservée.
        assert_eq!(effective_freq(51.4, false), 51.4);
    }

    #[test]
    fn stale_mppt_full_does_not_block_restore() {
        // État MPPT figé « plein » mais périmé → traité comme NON plein → ne bloque pas la
        // restauration (c'est exactement la classe de bug que la garde protège).
        assert!(!effective_mppt_full(true, true));
        // Frais + plein → bloque bien.
        assert!(effective_mppt_full(true, false));
        // Frais + pas plein → ne bloque pas.
        assert!(!effective_mppt_full(false, false));
    }

    #[test]
    fn feature_disabled_never_full() {
        // mppt_cut_enabled == false → étage MPPT ignoré (repli fréquence seule).
        let cfg = DeyeConfig { mppt_cut_enabled: false, ..Default::default() };
        assert!(!mppt_battery_full(&state_with_mppt(Some(4), Some(4)), &cfg));
    }
}
