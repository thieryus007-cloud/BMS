//crates/energy-manager/src/logic/water_heater/mod.rs

mod rules;
use chrono::{DateTime, Utc};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, sleep, Duration};
use tracing::{error, info, warn};
use crate::bus::AppBus;
use crate::config::WaterHeaterConfig;
use crate::http_clients::lg_thinq::LgThinqClient;
use crate::mqtt::topics::publish;
use crate::rules_loader::RulesLoader;
use crate::types::{EnergyState, LiveEvent, MqttOutgoing, WaterHeaterMode};

pub async fn spawn(
    wh_cfg: Arc<RwLock<WaterHeaterConfig>>,
    lg: Option<Arc<LgThinqClient>>,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
    loader: Arc<RulesLoader>,
) {
    // keepalive_secs est lu une seule fois (l'intervalle du ticker est fixé au
    // démarrage ; seuls les seuils métier sont rechargeables à chaud).
    let keepalive_secs = wh_cfg.read().await.keepalive_secs;
    crate::supervise::spawn_critical(keepalive_task(keepalive_secs, bus.clone(), state.clone()));
    if let Some(lg_client) = lg {
        crate::supervise::spawn_critical(control_task(wh_cfg, lg_client, bus, state, loader));
    } else {
        info!("Water heater auto-control disabled (no LG ThinQ client)");
    }
}

async fn keepalive_task(interval_secs: u64, bus: AppBus, state: Arc<RwLock<EnergyState>>) {
    let mut ticker = interval(Duration::from_secs(interval_secs));
    loop {
        ticker.tick().await;
        publish_to_venus(&bus, &state).await;
    }
}

async fn publish_to_venus(bus: &AppBus, state: &Arc<RwLock<EnergyState>>) {
    let s = state.read().await;
    let payload = json!({
        "State": s.water_heater_mode.to_venus_state(),
        "Temperature": s.water_heater_temp_c,
        "TargetTemperature": s.water_heater_target_c,
        "Position": 0,
    });
    drop(s);
    bus.publish(MqttOutgoing::retained(publish::HEATPUMP_VENUS, &payload)).await;
    bus.emit_live(LiveEvent::new("water_heater_venus", &payload));
}

/// Ancre (ou réarme) le chrono « cuve à température cible » en fonction de la
/// dernière température lue, stockée au préalable dans `water_heater_temp_c`.
///
/// Appelée à **chaque** lecture de température — par le poller LG ThinQ ET par
/// le `control_task` — afin que le chrono démarre dès l'instant où la cuve passe
/// ≥ `temp_max_c`, sans attendre le prochain tick de 5 min du control_task.
///
/// - Temp ≥ seuil : on date le premier franchissement (`get_or_insert`).
/// - Temp < seuil : la cible n'est plus tenue → on réarme.
/// - Temp inconnue (erreur API/timeout transitoire) : on PRÉSERVE le chrono en
///   cours pour ne pas perdre le temps accumulé.
pub(crate) fn anchor_temp_max(s: &mut EnergyState, now: DateTime<Utc>, temp_max_c: f64) {
    match s.water_heater_temp_c {
        Some(t) if t >= temp_max_c => {
            s.water_heater_temp_max_since.get_or_insert(now);
        }
        Some(_) => s.water_heater_temp_max_since = None,
        None => {}
    }
}

/// Met à jour le suivi « température cible atteinte » dans l'état partagé et
/// renvoie `true` si la température de la cuve est restée ≥ `temp_max_c`
/// pendant au moins `hold_secs`.
///
/// L'ancrage du chrono (`water_heater_temp_max_since`) est délégué à
/// [`anchor_temp_max`] ; cette fonction ne fait que mesurer la durée écoulée
/// depuis cet ancrage pour décider du passage en VACATION.
fn update_temp_max_tracking(
    s: &mut EnergyState,
    now: DateTime<Utc>,
    temp_max_c: f64,
    hold_secs: u64,
) -> bool {
    anchor_temp_max(s, now, temp_max_c);
    match (s.water_heater_temp_c, s.water_heater_temp_max_since) {
        (Some(t), Some(since)) if t >= temp_max_c => {
            (now - since).num_seconds().max(0) as u64 >= hold_secs
        }
        _ => false,
    }
}

async fn write_wh_metrics(vm_url: &str, mode: WaterHeaterMode, temp: Option<f64>, target: Option<f64>) {
    let ts_ms = Utc::now().timestamp_millis();
    let mut lines = Vec::new();
    lines.push(format!("wh_mode{{}} {} {}", mode.to_venus_state(), ts_ms));
    if let Some(t) = temp {
        lines.push(format!("wh_current_temp_c{{}} {} {}", t, ts_ms));
    }
    if let Some(t) = target {
        lines.push(format!("wh_target_temp_c{{}} {} {}", t, ts_ms));
    }
    let body = lines.join("\n");
    let url = format!("{}/api/v1/import/prometheus", vm_url);
    if let Err(e) = crate::http_clients::shared_client()
        .post(&url)
        .body(body)
        .send()
        .await
    {
        warn!("Water heater VM write error: {e}");
    }
}

async fn control_task(
    wh_cfg: Arc<RwLock<WaterHeaterConfig>>,
    lg: Arc<LgThinqClient>,
    bus: AppBus,
    state: Arc<RwLock<EnergyState>>,
    loader: Arc<RulesLoader>,
) {
    let mut last_change: Option<DateTime<Utc>> = None;
    let mut consecutive_fails: u32 = 0;
    let mut ticker    = interval(Duration::from_secs(300));
    let mut reload_rx = bus.subscribe_rule_reload();

    let mut rule_engine = match rules::WaterHeaterRuleEngine::with_source(&loader.load("water_heater")) {
        Ok(e) => e,
        Err(e) => {
            error!("Failed to init water heater rule engine: {e}");
            return;
        }
    };

    info!("Water heater control task started (5min interval)");

    loop {
        tokio::select! {
        _ = ticker.tick() => {
        let now = Utc::now();

        // Snapshot des seuils courants (rechargeable à chaud via reload_rx) :
        // un clone par tick (toutes les 5 min) → coût négligeable, et tous les
        // paramètres chauffe-eau deviennent modifiables sans redémarrage.
        let cfg = wh_cfg.read().await.clone();

        // Read actual state from LG ThinQ before deciding
        info!("Water heater: calling lg.get_state()...");
        let lg_snapshot = match lg.get_state().await {
            Ok(snap) => {
                info!(
                    "Water heater: LG get_state OK → mode={:?}, temp={:?}°C, target={:?}°C",
                    snap.mode, snap.current_temp_c, snap.target_temp_c
                );
                {
                    let mut s = state.write().await;
                    s.water_heater_mode     = snap.mode;
                    s.water_heater_temp_c   = snap.current_temp_c;
                    s.water_heater_target_c = snap.target_temp_c;
                    s.water_heater_last_read = Some(now);
                }
                write_wh_metrics(
                    &cfg.vm_url,
                    snap.mode,
                    snap.current_temp_c,
                    snap.target_temp_c,
                ).await;
                Some(snap)
            }
            Err(e) => {
                error!("LG ThinQ get_state FAILED: {e} — will skip mode comparison this tick");
                None
            }
        };

        // Suivi « cuve à température cible » : si la température (lue à l'instant
        // ci-dessus, sinon valeur en cache rafraîchie par le poller LG) reste
        // ≥ temp_max_c pendant temp_max_hold_secs → on forcera VACATION.
        let (temp_max_reached, temp_max_since) = {
            let mut s = state.write().await;
            let reached = update_temp_max_tracking(&mut s, now, cfg.temp_max_c, cfg.temp_max_hold_secs);
            (reached, s.water_heater_temp_max_since)
        };
        if temp_max_reached {
            info!(
                "Water heater: température ≥ {:.1}°C tenue ≥ {}s (depuis {:?}) → cible atteinte, VACATION forcé",
                cfg.temp_max_c, cfg.temp_max_hold_secs, temp_max_since
            );
        }

        // Read energy conditions — skip evaluation if MQTT data not yet available
        let (ac_ignore_opt, soc_opt, irradiance) = {
            let s = state.read().await;
            (s.ac_ignore, s.soc_pct, s.irradiance_wm2)
        };

        info!(
            "Water heater tick — ac_ignore={:?}, soc={:?}, irradiance={:?} W/m² (min={})",
            ac_ignore_opt, soc_opt, irradiance, cfg.irradiance_min_wm2
        );

        if ac_ignore_opt.is_none() || soc_opt.is_none() {
            warn!(
                "Water heater: MQTT data missing (ac_ignore={:?}, soc={:?}) — skipping evaluation. \
                 Check that energy-manager subscriptions are received from broker.",
                ac_ignore_opt, soc_opt
            );
            continue;
        }

        let ac_ignore = ac_ignore_opt.unwrap_or(0);
        let soc       = soc_opt.unwrap_or(0.0);
        let irradiance_low = match irradiance {
            Some(w) => {
                let low = w < cfg.irradiance_min_wm2;
                info!("Water heater: irradiance={:.1} W/m², min={}, irradiance_low={}", w, cfg.irradiance_min_wm2, low);
                low
            }
            None => {
                warn!(
                    "Water heater: irradiance_wm2=None (topic 'santuario/irradiance/raw' not received yet) \
                     — treating as irradiance_low=true → target will be VACATION"
                );
                true
            }
        };
        let grid_connected = ac_ignore == 0;
        let soc_low = soc < cfg.soc_min_pct;
        info!(
            "Water heater: ac_ignore={}, grid_connected={}, soc={:.1}% (min={}%, soc_low={}), irradiance_low={}",
            ac_ignore, grid_connected, soc, cfg.soc_min_pct, soc_low, irradiance_low
        );

        let target_mode_str = match rule_engine.evaluate(grid_connected, soc_low, irradiance_low, temp_max_reached) {
            Ok(m) => {
                info!("Water heater: rule engine → target_mode={m} (grid_connected={}, soc={:.1}%, irradiance_low={}, temp_max_reached={})",
                    grid_connected, soc, irradiance_low, temp_max_reached);
                m
            }
            Err(e) => {
                error!("Rule engine error: {e} — fallback VACATION");
                "VACATION".to_string()
            }
        };

        let target_mode = match target_mode_str.as_str() {
            "HEAT_PUMP" => WaterHeaterMode::HeatPump,
            _ => WaterHeaterMode::Vacation,
        };

        // Use actual LG state (from readback) as the reference for comparison
        let actual_mode = lg_snapshot
            .as_ref()
            .map(|s| s.mode)
            .unwrap_or_else(|| {
                warn!("Water heater: LG readback failed, using cached state for comparison");
                state.try_read().map(|s| s.water_heater_mode).unwrap_or(WaterHeaterMode::Vacation)
            });

        info!(
            "Water heater: target={:?}, actual (LG)={:?}, should_send={}",
            target_mode, actual_mode, actual_mode != target_mode
        );

        let should_send = actual_mode != target_mode;

        if !should_send {
            info!(
                "Water heater: target={:?} matches actual={:?}, no command needed",
                target_mode, actual_mode
            );
            consecutive_fails = 0;
            continue;
        }

        let can_change = last_change
            .map(|t| (now - t).num_seconds() as u64 >= cfg.mode_change_min_secs)
            .unwrap_or(true);

        if !can_change {
            let secs_left = cfg.mode_change_min_secs.saturating_sub(
                last_change.map(|t| (now - t).num_seconds() as u64).unwrap_or(0)
            );
            info!("Water heater: cooldown active, {}s remaining — target={:?}", secs_left, target_mode);
            continue;
        }

        info!(
            "Water heater: SEND {:?} (actual={:?}, soc={:.1}%, grid={}, irradiance_low={})",
            target_mode, actual_mode, soc, grid_connected, irradiance_low
        );

        if let Err(e) = lg.set_mode(target_mode).await {
            error!("LG set_mode error: {e}");
            consecutive_fails += 1;
            if consecutive_fails >= 3 {
                warn!(
                    "Water heater: {} consecutive send failures — LG ThinQ may be unreachable!",
                    consecutive_fails
                );
            }
            continue;
        }

        consecutive_fails = 0;
        last_change = Some(now);

        {
            let mut s = state.write().await;
            s.water_heater_mode = target_mode;
            s.water_heater_last_change = Some(now);
            s.water_heater_send_count += 1;
        }
        publish_to_venus(&bus, &state).await;

        let delay_secs = cfg.temp_set_delay_secs;
        let target_temp = match target_mode {
            WaterHeaterMode::HeatPump => cfg.heat_pump_target_c,
            _ => cfg.vacation_target_c,
        };

        let lg2 = lg.clone();
        let bus2 = bus.clone();
        let state2 = state.clone();
        tokio::spawn(async move {
            sleep(Duration::from_secs(delay_secs)).await;
            if let Err(e) = lg2.set_target_temp(target_temp).await {
                error!("LG set_target_temp error: {e}");
                return;
            }
            {
                let mut s = state2.write().await;
                s.water_heater_target_c = Some(target_temp);
            }
            publish_to_venus(&bus2, &state2).await;
        });
        }   // close _ = ticker.tick() arm

        Ok(name) = reload_rx.recv() => {
            if name == "water_heater" || name == "*" {
                let src = loader.load("water_heater");
                match rules::WaterHeaterRuleEngine::with_source(&src) {
                    Ok(e) => { rule_engine = e; info!("water_heater rule engine reloaded"); }
                    Err(e) => tracing::warn!("water_heater reload failed (keeping old engine): {e}"),
                }
                // Recharge les seuils depuis Config.toml (temp_max_c, hold, etc.)
                // → modifiables à chaud, sans recompiler ni redémarrer le service.
                // Le serveur HTTP partage le même Arc<RwLock> → /api/rules-status
                // reflète immédiatement les nouvelles valeurs.
                // load() fait de l'I/O fichier bloquante → l'isoler du pool async
                // (spawn_blocking) pour ne pas bloquer l'exécuteur Tokio.
                match tokio::task::spawn_blocking(crate::config::load).await {
                    Ok(Ok(c)) => {
                        let new = c.water_heater;
                        info!(
                            "water_heater config reloaded (temp_max_c={}, temp_max_hold_secs={}, \
                             irradiance_min_wm2={}, soc_min_pct={})",
                            new.temp_max_c, new.temp_max_hold_secs,
                            new.irradiance_min_wm2, new.soc_min_pct,
                        );
                        *wh_cfg.write().await = new;
                    }
                    Ok(Err(e)) => tracing::warn!("water_heater config reload failed (keeping old): {e}"),
                    Err(e) => tracing::warn!("water_heater config reload task panicked: {e}"),
                }
            }
        }
        }   // close select!
    }       // close loop
}
