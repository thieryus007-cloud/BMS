//! DEYE relay control via Shelly Pro 2PM (MQTT RPC).
//!
//! The decision is **pure Rust** (`rules::decide`, like every other module), and it is
//! **stateless**: there is NO latching state machine (no "Lockout"/"Pending*" that can get
//! stuck). This controller carries only the relay state and two debounce timers, pre-computes
//! the boolean flags, and `rules::decide` maps them to the desired relay state, re-evaluated
//! every second. The relay simply tracks the inputs — as long as the 1 Hz loop ticks it always
//! converges, so it can never be stranded.
//!
//! The 4 things the decision depends on:
//!   1. AC-Out frequency   → cut at/above `freq_high_hz`, restore below.
//!   2. MPPT charge stage  → cut while any charger is full (Float/Absorption/Storage).
//!   3. the current relay state.
//!   4. time (two debounce timers: `cut_delay_secs`, `reenable_delay_secs`).
//!
//! Relay state (On/Off) is persisted to `santuario/persist/deye_state` (retained MQTT) so a
//! restart doesn't toggle the relay spuriously.

mod rules;

use chrono::{DateTime, Utc};
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tokio::time::{interval, sleep, Duration};
use tracing::{info, warn};

use crate::bus::AppBus;
use crate::config::{DeyeConfig, VictronConfig};
use crate::mqtt::topics::publish;
use crate::types::{EnergyState, MqttOutgoing};

/// Nominal AC frequency used when the real one is stale (well below any cut threshold)
/// → restore allowed, no frequency cut. The DEYE 51.5 Hz hardware auto-trip is the net.
const NOMINAL_FREQ_HZ: f64 = 50.0;

// ---------------------------------------------------------------------------
// Controller — carries the relay state + debounce timers, pre-computes the flags
// fed to the (stateless) `rules::decide`. Nothing here can latch and get stuck.
// ---------------------------------------------------------------------------

struct DeyeController {
    relay_on: bool,
    /// Continuously in the "should cut" condition since this instant (None otherwise).
    cut_since: Option<DateTime<Utc>>,
    /// Continuously in the "should restore" condition since this instant (None otherwise).
    restore_since: Option<DateTime<Utc>>,
}

/// Pre-computed boolean flags handed to `rules::decide`.
struct DeyeFlags {
    hard_cut: bool,
    cut_debounced: bool,
    restore_debounced: bool,
}

impl DeyeController {
    fn new(relay_on: bool) -> Self {
        Self { relay_on, cut_since: None, restore_since: None }
    }

    /// Updates the debounce anchors from the current inputs and returns the flags for the rules.
    /// - cut condition  = `freq >= freq_high_hz` OR `mppt_full`
    /// - clear condition = the negation
    fn flags(&mut self, freq_hz: f64, mppt_full: bool, now: DateTime<Utc>, cfg: &DeyeConfig) -> DeyeFlags {
        let should_cut = freq_hz >= cfg.freq_high_hz || mppt_full;
        let hard_cut   = freq_hz >= cfg.freq_hard_hz;

        if should_cut {
            self.cut_since.get_or_insert(now);
            self.restore_since = None;
        } else {
            self.restore_since.get_or_insert(now);
            self.cut_since = None;
        }

        let held = |since: Option<DateTime<Utc>>, secs: u64| {
            since.map(|t| (now - t).num_seconds().max(0) as u64 >= secs).unwrap_or(false)
        };

        DeyeFlags {
            hard_cut,
            cut_debounced: held(self.cut_since, cfg.cut_delay_secs),
            restore_debounced: held(self.restore_since, cfg.reenable_delay_secs),
        }
    }

    /// Applies the decision. Returns `Some(new_on)` iff the relay changed.
    fn set(&mut self, desired_on: bool) -> Option<bool> {
        if desired_on != self.relay_on {
            self.relay_on = desired_on;
            Some(desired_on)
        } else {
            None
        }
    }
}

/// Inputs read from shared state for one evaluation, with their freshness.
struct Inputs {
    freq_hz: f64,
    freq_stale: bool,
    mppt_full: bool,
    mppt_stale: bool,
}

/// Reads freq + MPPT from shared state, applying freshness guards so a frozen telemetry
/// value can never strand the relay:
/// - stale frequency → treated as nominal (50 Hz) → restore allowed, no frequency cut;
/// - stale MPPT state → treated as NOT full → does not hold the relay off.
async fn read_inputs(state: &Arc<RwLock<EnergyState>>, cfg: &DeyeConfig, now: DateTime<Utc>) -> Inputs {
    let s = state.read().await;
    let max = cfg.input_max_age_secs as i64;

    let freq_stale = s.ac_frequency_last_ts.map(|t| (now - t).num_seconds() > max).unwrap_or(true);
    let freq_hz = if freq_stale { NOMINAL_FREQ_HZ } else { s.ac_frequency_hz.unwrap_or(NOMINAL_FREQ_HZ) };

    // MPPT is fresh if at least one charger reported its State recently.
    let mppt_stale = [s.mppt_273.state_last_ts, s.mppt_289.state_last_ts]
        .into_iter()
        .flatten()
        .map(|t| (now - t).num_seconds())
        .min()
        .map(|age| age > max)
        .unwrap_or(true);
    let mppt_full = cfg.mppt_cut_enabled
        && !mppt_stale
        && [s.mppt_273.state, s.mppt_289.state]
            .into_iter()
            .flatten()
            .any(|st| cfg.mppt_full_states.contains(&st));

    Inputs { freq_hz, freq_stale, mppt_full, mppt_stale }
}

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

pub async fn spawn(
    vic: Arc<VictronConfig>,
    cfg: DeyeConfig,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
) {
    crate::supervise::spawn_critical(run(vic, cfg, bus, state));
}

async fn run(
    vic: Arc<VictronConfig>,
    cfg: DeyeConfig,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
) {
    let pid = &vic.portal_id;
    let vb  = vic.vebus_instance;
    let t_connected = format!("N/{pid}/vebus/{vb}/Ac/ActiveIn/Connected");

    let shelly_id = &vic.shelly_deye_id;
    // One channel per DEYE. Prefer the multi-channel list; fall back to the legacy single field.
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

    // Restore persisted relay state — wait up to 3s for the retained MQTT message,
    // so restarting while DEYE is cut doesn't spuriously power it back on.
    let initial_on = {
        sleep(Duration::from_secs(3)).await;
        let s = state.read().await;
        match s.deye_persisted_state.as_deref() {
            Some("Off") => { info!("DEYE: restoring relay=Off from retained MQTT"); false }
            Some(other) => { info!("DEYE: starting relay=On (persisted={other:?})"); true }
            None        => { info!("DEYE: no persisted state — starting relay=On"); true }
        }
    };

    let mut controller = DeyeController::new(initial_on);
    let mut rx        = bus.subscribe_mqtt();
    // 1 Hz authoritative driver: re-derives the relay state every second from the inputs,
    // so the relay always converges regardless of MQTT traffic. The first tick fires now.
    let mut ticker = interval(Duration::from_secs(1));
    // Periodic idempotent re-assert so the physical Shelly reconverges after a missed command.
    let mut resync = interval(Duration::from_secs(cfg.relay_resync_secs.max(1)));

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let now = Utc::now();
                let inputs = read_inputs(&state, &cfg, now).await;
                apply(&mut controller, &inputs, now, &cfg, &bus, shelly_id, &channels, &state).await;
            }

            result = rx.recv() => {
                let msg = match result {
                    Ok(m) => m,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("deye_command MQTT subscriber lagged, dropped {n} message(s)");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                // Grid status is informational only (not part of the decision); track it for the widget.
                if msg.topic == t_connected {
                    if let Some(v) = msg.victron_value::<i64>() {
                        state.write().await.ac_connected = Some(v);
                    }
                }
            }

            _ = resync.tick() => {
                send_shelly(&bus, shelly_id, &channels, controller.relay_on).await;
            }
        }
    }
}

/// Pre-computes the flags, derives the desired relay state (`rules::decide`), applies any
/// change (relay command + persistence), then refreshes the `/api/rules-status` fields.
#[allow(clippy::too_many_arguments)]
async fn apply(
    controller: &mut DeyeController,
    inputs: &Inputs,
    now: DateTime<Utc>,
    cfg: &DeyeConfig,
    bus: &AppBus,
    shelly_id: &str,
    channels: &[u8],
    state: &Arc<RwLock<EnergyState>>,
) {
    let flags = controller.flags(inputs.freq_hz, inputs.mppt_full, now, cfg);
    let desired_on = rules::decide(
        controller.relay_on, flags.hard_cut, flags.cut_debounced, flags.restore_debounced,
    );
    let changed = controller.set(desired_on);

    if let Some(on) = changed {
        send_shelly(bus, shelly_id, channels, on).await;
        persist_relay(bus, on).await;
        info!(
            "DEYE: relais → {} (freq={:.2} Hz, mppt_full={})",
            if on { "ON" } else { "OFF" }, inputs.freq_hz, inputs.mppt_full
        );
    }

    let mut s = state.write().await;
    s.deye_on              = controller.relay_on;
    s.deye_state           = Some(if controller.relay_on { "On" } else { "Off" }.to_string());
    // Restore is "held off" only when the relay is OFF because the battery is full per MPPT.
    s.deye_restore_blocked = !controller.relay_on && inputs.mppt_full;
    s.deye_mppt_full       = inputs.mppt_full;
    s.deye_freq_stale      = inputs.freq_stale;
    s.deye_mppt_stale      = inputs.mppt_stale;
    s.deye_lockout_until   = None; // no latching lockout state anymore
    if changed.is_some() {
        s.deye_last_change = Some(now);
    }
}

/// Publishes the relay state as retained MQTT for persistence across restarts.
async fn persist_relay(bus: &AppBus, on: bool) {
    bus.publish(MqttOutgoing::raw(
        publish::DEYE_STATE, if on { "On" } else { "Off" }, true,
    )).await;
}

/// Sends a `Switch.Set` to every DEYE channel (idempotent on the Shelly side).
async fn send_shelly(bus: &AppBus, shelly_id: &str, channels: &[u8], on: bool) {
    let topic = publish::shelly_rpc(shelly_id);
    for &channel in channels {
        let payload = serde_json::json!({
            "id":     1,
            "src":    "energy-manager",
            "method": "Switch.Set",
            "params": { "id": channel, "on": on }
        });
        bus.publish(MqttOutgoing::transient(topic.clone(), &payload)).await;
    }
    tracing::debug!("DEYE Shelly: channels {channels:?} = {}", if on { "ON" } else { "OFF" });
}

/// Whether the Victron is grid-tied (NOT islanded). **Informational only** — no longer part
/// of the DEYE decision (Fréquence + MPPT only); kept for the dashboard "Réseau" row.
pub(crate) fn is_grid_connected(ac_ignore: Option<i64>, ac_connected: Option<i64>) -> bool {
    ac_ignore != Some(1) && ac_connected != Some(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> DeyeConfig {
        // freq cut 51.0 / hard 51.3, cut debounce 3s, restore debounce 45s, full = 4/5/6.
        DeyeConfig::default()
    }

    fn t(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    /// Drives the controller for one tick and returns the relay change, if any.
    fn step(c: &mut DeyeController, freq: f64, mppt_full: bool, now: DateTime<Utc>, cfg: &DeyeConfig) -> Option<bool> {
        let f = c.flags(freq, mppt_full, now, cfg);
        let desired = rules::decide(c.relay_on, f.hard_cut, f.cut_debounced, f.restore_debounced);
        c.set(desired)
    }

    #[test]
    fn grid_tied_helper() {
        assert!(is_grid_connected(Some(0), Some(1)));
        assert!(!is_grid_connected(Some(1), Some(1))); // deliberate islanding
        assert!(!is_grid_connected(Some(0), Some(0))); // grid outage
    }

    #[test]
    fn hard_freq_cuts_immediately() {
        let mut c = DeyeController::new(true);
        assert_eq!(step(&mut c, 51.5, false, t(0), &cfg()), Some(false));
        assert!(!c.relay_on);
    }

    #[test]
    fn soft_high_freq_cuts_after_debounce() {
        let mut c = DeyeController::new(true);
        let cfg = cfg();
        assert_eq!(step(&mut c, 51.1, false, t(0), &cfg), None);   // debouncing
        assert_eq!(step(&mut c, 51.1, false, t(2), &cfg), None);   // < cut_delay
        assert_eq!(step(&mut c, 51.1, false, t(3), &cfg), Some(false)); // 3s → cut
    }

    #[test]
    fn mppt_full_cuts_even_at_low_freq() {
        let mut c = DeyeController::new(true);
        let cfg = cfg();
        assert_eq!(step(&mut c, 50.0, true, t(0), &cfg), None);
        assert_eq!(step(&mut c, 50.0, true, t(3), &cfg), Some(false));
    }

    #[test]
    fn restores_after_sustained_clear() {
        let mut c = DeyeController::new(false);
        let cfg = cfg();
        assert_eq!(step(&mut c, 50.0, false, t(0), &cfg), None);
        assert_eq!(step(&mut c, 50.0, false, t(44), &cfg), None);  // < reenable_delay
        assert_eq!(step(&mut c, 50.0, false, t(45), &cfg), Some(true)); // 45s → restore
    }

    #[test]
    fn restore_wait_resets_if_condition_breaks() {
        let mut c = DeyeController::new(false);
        let cfg = cfg();
        assert_eq!(step(&mut c, 50.0, false, t(0), &cfg), None);
        assert_eq!(step(&mut c, 51.2, false, t(10), &cfg), None); // freq climbs → reset
        assert_eq!(step(&mut c, 50.0, false, t(20), &cfg), None);
        assert_eq!(step(&mut c, 50.0, false, t(64), &cfg), None); // only 44s from t=20
        assert_eq!(step(&mut c, 50.0, false, t(65), &cfg), Some(true));
    }

    /// THE regression this guards: once conditions are clearly restorable (freq low, MPPT not
    /// full), the relay ALWAYS restores — it can never stay stranded OFF the way the old
    /// latching "Lockout" state could.
    #[test]
    fn never_stuck_off_when_conditions_allow_restore() {
        let mut c = DeyeController::new(false);
        let cfg = cfg();
        let mut restored = false;
        for s in 0..300 {
            if let Some(true) = step(&mut c, 50.08, false, t(s), &cfg) {
                restored = true;
                break;
            }
        }
        assert!(restored, "relay must restore once conditions are clear — never stranded");
    }

    #[test]
    fn full_cycle_cut_then_restore() {
        let mut c = DeyeController::new(true);
        let cfg = cfg();
        assert_eq!(step(&mut c, 51.5, false, t(0), &cfg), Some(false)); // spike → cut
        assert_eq!(step(&mut c, 50.0, false, t(10), &cfg), None);
        assert_eq!(step(&mut c, 50.0, false, t(55), &cfg), Some(true)); // restore
    }
}
