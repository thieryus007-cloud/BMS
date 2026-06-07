//! Router Axum — API REST + WebSocket
//!
//! Toutes les routes sont définies ici et réparties dans les sous-modules.

pub mod system;
pub mod bms;
pub mod ats;
pub mod alerts;
pub mod console;
pub mod dashboards;
pub mod et112;
pub mod tasmota;
pub mod shelly;
pub mod chart;
pub mod history;
pub mod promql;
pub mod redb;
pub mod health;
pub mod prometheus_import;

use crate::dashboard;
use crate::state::AppState;
use axum::{
    Router,
    routing::{get, post},
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

/// Construit le router principal de l'application.
pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // ── Dashboard HTML ────────────────────────────────────────────────────
        .merge(dashboard::build_dashboard_router())

        // ── Système ─────────────────────────────────────────────────────────
        .route("/api/v1/system/status",  get(system::get_status))
        .route("/api/v1/system/logs",    get(system::get_logs))
        .route("/api/v1/system/totals",  get(system::get_system_totals))
        .route("/api/v1/config",         get(system::get_config))
        .route("/api/v1/discover",       get(system::discover))
        .route("/api/v1/irradiance/status", get(system::get_irradiance_status))
        .route("/api/v1/solar/mppt-yield",  post(system::set_mppt_yield))

        // ── Venus OS D-Bus ──────────────────────────────────────────────────
        .route("/api/v1/venus/mppt",        get(system::get_venus_mppt))
        .route("/api/v1/venus/smartshunt",  get(system::get_venus_smartshunt))
        .route("/api/v1/venus/inverter",    get(system::get_venus_inverter))
        .route("/api/v1/venus/temperatures",get(system::get_venus_temperatures))
        .route("/api/v1/venus/heatpumps",   get(system::get_venus_heatpumps))

        // ── Monitoring système Pi5 ────────────────────────────────────────────
        .route("/api/v1/monitor/status",        get(system::get_monitor_status))
        .route("/api/v1/monitor/rs485-health",  get(system::get_rs485_health))
        .route("/api/v1/monitor/logs",          get(system::get_monitor_logs))
        .route("/api/v1/monitor/logs/content",  get(system::get_monitor_log_content))

        // ── BMS — Lecture ────────────────────────────────────────────────────
        .route("/api/v1/bms/{id}/status",      get(bms::get_bms_status))
        .route("/api/v1/bms/{id}/cells",       get(bms::get_cells))
        .route("/api/v1/bms/{id}/temperatures",get(bms::get_temperatures))
        .route("/api/v1/bms/{id}/alarms",      get(bms::get_alarms))
        .route("/api/v1/bms/{id}/mos",         get(bms::get_mos))
        .route("/api/v1/bms/{id}/history",     get(bms::get_history))
        .route("/api/v1/bms/{id}/history/summary", get(bms::get_history_summary))
        .route("/api/v1/bms/{id}/export/csv",  get(bms::export_csv))
        .route("/api/v1/bms/compare",         get(bms::compare_all))

        // ── BMS — Paramètres (lecture à la demande) ───────────────────────────
        .route("/api/v1/bms/{id}/settings",                         get(bms::get_settings))

        // ── BMS — Écriture ────────────────────────────────────────────────────
        .route("/api/v1/bms/{id}/mos",                              post(bms::set_mos))
        .route("/api/v1/bms/{id}/soc",                              post(bms::set_soc))
        .route("/api/v1/bms/{id}/soc/full",                         post(bms::set_soc_full))
        .route("/api/v1/bms/{id}/soc/empty",                        post(bms::set_soc_empty))
        .route("/api/v1/bms/{id}/reset",                            post(bms::reset_bms))
        .route("/api/v1/bms/{id}/settings/cell-voltage-alarms",     post(bms::set_cell_volt_alarms))
        .route("/api/v1/bms/{id}/settings/pack-voltage-alarms",     post(bms::set_pack_volt_alarms))
        .route("/api/v1/bms/{id}/settings/current-alarms",          post(bms::set_current_alarms))
        .route("/api/v1/bms/{id}/settings/delta-alarms",            post(bms::set_delta_alarms))
        .route("/api/v1/bms/{id}/settings/balancing",               post(bms::set_balancing))

        // ── ATS CHINT ────────────────────────────────────────────────────────
        .route("/api/v1/ats/status",        get(ats::get_ats_status))
        .route("/api/v1/ats/remote_on",     post(ats::ats_remote_on))
        .route("/api/v1/ats/remote_off",    post(ats::ats_remote_off))
        .route("/api/v1/ats/force_source1", post(ats::ats_force_source1))
        .route("/api/v1/ats/force_source2", post(ats::ats_force_source2))
        .route("/api/v1/ats/force_double",  post(ats::ats_force_double))
        .route("/api/v1/ats/send_raw",      post(ats::ats_send_raw))
        .route("/api/v1/ats/debug_on",      get(ats::ats_debug_on))
        .route("/api/v1/ats/debug_off",     get(ats::ats_debug_off))

        // ── ET112 ────────────────────────────────────────────────────────────
        .route("/api/v1/et112",                   get(et112::list_et112))
        .route("/api/v1/et112/{addr}/status",      get(et112::get_et112_status))
        .route("/api/v1/et112/{addr}/history",     get(et112::get_et112_history))

        // ── Chart historique (Tsink) ──────────────────────────────────────────
        .route("/api/v1/chart/history",           get(chart::get_chart_history))
        .route("/api/v1/chart/edge-history",      get(chart::get_edge_history))
        .route("/api/v1/history/energy",          get(history::get_energy_history))

        // ── Dashboards (catalogue panels Grafana + data + layouts) ────────────
        .route("/api/v1/dashboards/catalog",         get(dashboards::get_catalog))
        .route("/api/v1/dashboards/panel/{id}/data",  get(dashboards::get_panel_data))
        .route(
            "/api/v1/dashboards/layout",
            get(dashboards::get_layout)
                .post(dashboards::set_layout)
                .delete(dashboards::delete_layout),
        )

        // ── Tasmota ──────────────────────────────────────────────────────────
        .route("/api/v1/tasmota",                 get(tasmota::list_tasmota))
        .route("/api/v1/tasmota/{id}/status",      get(tasmota::get_tasmota_status))
        .route("/api/v1/tasmota/{id}/history",     get(tasmota::get_tasmota_history))
        .route("/api/v1/tasmota/{id}/control",     post(tasmota::control_tasmota))
        // ── Shelly ───────────────────────────────────────────────────────────
        .route("/api/v1/shelly",                             get(shelly::list_shelly))
        .route("/api/v1/shelly/{id}/status",                  get(shelly::get_shelly_status))
        .route("/api/v1/shelly/{id}/channel/{ch}/control",     post(shelly::control_shelly_channel))
        // ── PromQL (historique Tsink) ─────────────────────────────────────────
        // ── PromQL Grafana-facing — dispatch vm/redb selon
        //    `[metrics_store].default_backend` (cf. api/promql.rs).
        //    GET + POST tous les deux (Grafana `httpMethod: POST` par défaut)
        .route("/api/v1/query",
               get(promql::query_instant).post(promql::query_instant_post))
        .route("/api/v1/query_range",
               get(promql::query_range).post(promql::query_range_post))
        .route("/api/v1/labels",         get(promql::list_metrics))

        // ── Import Prometheus text format → redb (remplace VM /api/v1/import/prometheus)
        .route("/api/v1/import/prometheus",
               post(prometheus_import::import_prometheus))

        // ── Endpoints Prometheus complémentaires §8.2 (toujours redb) ─────
        .route("/api/v1/series",                      get(redb::list_series))
        .route("/api/v1/label/{name}/values",          get(redb::label_values))
        .route("/-/healthy",                          get(redb::healthy))

        // ── Endpoints redb explicites (debug, parité côte-à-côte) ──────────
        .route("/api/v1/redb/query",
               get(redb::query_instant).post(redb::query_instant_post))
        .route("/api/v1/redb/query_range",
               get(redb::query_range).post(redb::query_range_post))
        .route("/api/v1/redb/series",                 get(redb::list_series))
        .route("/api/v1/redb/labels",                 get(redb::list_labels))
        .route("/api/v1/redb/label/{name}/values",     get(redb::label_values))
        .route("/api/v1/redb/healthy",                get(redb::healthy))

        // ── Alertes (journal SQLite) ──────────────────────────────────────────
        .route("/api/v1/alerts/list",              get(alerts::list_alerts))
        .route("/api/v1/alerts/stats",             get(alerts::get_stats))
        .route("/api/v1/alerts/{id}/acknowledge",   post(alerts::acknowledge_alert))

        // ── Health check ──────────────────────────────────────────────────────
        .route("/health",                get(health::health_check))

        // ── WebSocket ─────────────────────────────────────────────────────────
        .route("/ws/bms/stream",         get(bms::ws_all))
        .route("/ws/bms/{id}/stream",     get(bms::ws_single))
        .route("/ws/venus/stream",       get(bms::ws_venus))
        .route("/ws/console",            get(console::ws_console))

        // ── Middlewares ───────────────────────────────────────────────────────
        // tower-http upgrade 0.5 → 0.6 a refactoré le pattern BoxCloneService
        // qui causait ~80 % des allocations linéaires par requête HTTP en 0.5
        // (cf. docs/diagnostic-depannage.md §13). Avec 0.6, CorsLayer +
        // TraceLayer ne fuient plus → on peut tout réactiver.
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
