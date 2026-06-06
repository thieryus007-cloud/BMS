//! Agent de monitoring autonome — surveille l'ensemble du système Pi5.
//!
//! Vérifie toutes les 30 secondes :
//! - Services systemd daly-bms + energy-manager (via systemctl)
//! - Services réseau via sonde TCP : mosquitto, energy-manager, venus MQTT
//! - Port série RS485 (/dev/ttyUSB0)
//! - CPU, RAM, disque, charge système, uptime
//! - Liste des processus (top 20 par CPU%)
//! - I/O réseau (octets/s)
//! - Température CPU

use crate::state::{AppState, MonitorSnapshot, ProcessInfo, ServiceStatus};
use chrono::Utc;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::time::interval;
use tracing::{info, warn};

/// Services réseau à sonder : (label, host, port, service_systemd_à_redémarrer).
const TCP_SERVICES: &[(&str, &str, u16, Option<&str>)] = &[
    ("mosquitto",      "127.0.0.1",     1883, Some("mosquitto-broker")),
    ("energy-manager", "127.0.0.1",     8081, None),
    ("venus-mqtt",     "192.168.1.120", 1883, None),
];

/// Port série RS485.
const RS485_PORT: &str = "/dev/ttyUSB0";

// =============================================================================
// Agent principal
// =============================================================================

/// Démarre l'agent de monitoring en arrière-plan.
pub async fn run_monitor_agent(state: AppState) {
    info!("Agent de monitoring Pi5 démarré (intervalle: 30s)");
    let mut ticker   = interval(Duration::from_secs(30));
    let mut prev_net = read_net_bytes().await;
    let mut prev_net_ts = Instant::now();

    loop {
        ticker.tick().await;

        // Heartbeat watchdog systemd — doit arriver avant WatchdogSec=60
        let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Watchdog]);

        let curr_net    = read_net_bytes().await;
        let elapsed_s   = prev_net_ts.elapsed().as_secs_f64().max(1.0);
        let net_rx_bps  = ((curr_net.0.saturating_sub(prev_net.0)) as f64 / elapsed_s) as u64;
        let net_tx_bps  = ((curr_net.1.saturating_sub(prev_net.1)) as f64 / elapsed_s) as u64;
        prev_net        = curr_net;
        prev_net_ts     = Instant::now();

        let snap = collect_snapshot(&state, net_rx_bps, net_tx_bps).await;
        // Les métriques système sont écrites dans metrics-store (redb) via
        // on_monitor_snapshot (hook d'écriture côté state.rs).
        state.on_monitor_snapshot(snap).await;
    }
}

// =============================================================================
// Collecte complète
// =============================================================================

async fn collect_snapshot(_state: &AppState, net_rx_bps: u64, net_tx_bps: u64) -> MonitorSnapshot {
    let mut services         = Vec::new();
    let mut network_services = Vec::new();
    let mut auto_actions     = Vec::new();

    // ── Services systemd ──────────────────────────────────────────────────────
    let daly_status = check_systemd_service("daly-bms").await;
    services.push(ServiceStatus {
        name:   "daly-bms".to_string(),
        active: true,
        status: if daly_status.is_empty() { "active".to_string() } else { daly_status },
    });

    let em_status  = check_systemd_service("energy-manager").await;
    let em_systemd = em_status == "active";
    let em_tcp_ok  = if em_systemd { true } else { tcp_probe("127.0.0.1", 8081).await };
    let em_active  = em_systemd || em_tcp_ok;
    services.push(ServiceStatus {
        name:   "energy-manager".to_string(),
        active: em_active,
        status: if em_active {
            if em_systemd { em_status } else { "active (port 8081)".to_string() }
        } else if em_status.is_empty() { "unknown".to_string() } else { em_status },
    });

    let mosquitto_systemd = check_systemd_service("mosquitto-broker").await == "active";
    services.push(ServiceStatus {
        name:   "mosquitto-broker".to_string(),
        active: mosquitto_systemd || tcp_probe("127.0.0.1", 1883).await,
        status: if mosquitto_systemd { "active".to_string() } else { "inactive".to_string() },
    });

    // ── Sondes TCP ───────────────────────────────────────────────────────────
    for &(name, host, port, svc_to_restart) in TCP_SERVICES {
        let reachable = tcp_probe(host, port).await;

        if !reachable {
            if let Some(svc) = svc_to_restart {
                if restart_systemd_service(svc).await {
                    let msg = format!("Redémarré service systemd: {}", svc);
                    info!("{}", msg);
                    auto_actions.push(msg);
                    network_services.push(ServiceStatus {
                        name: name.to_string(), active: false, status: "restarted".to_string(),
                    });
                } else {
                    warn!("Échec redémarrage service systemd: {}", svc);
                    network_services.push(ServiceStatus {
                        name: name.to_string(), active: false, status: "down".to_string(),
                    });
                }
            } else {
                network_services.push(ServiceStatus {
                    name: name.to_string(), active: false, status: "unreachable".to_string(),
                });
            }
        } else {
            network_services.push(ServiceStatus {
                name: name.to_string(), active: true, status: format!("{}:{}", host, port),
            });
        }
    }

    // ── Port série RS485 ─────────────────────────────────────────────────────
    let serial_port_ok = tokio::fs::metadata(RS485_PORT).await.is_ok();

    // ── Métriques système ────────────────────────────────────────────────────
    let load_avg       = read_load_avg().await;
    let cpu_percent    = read_cpu_percent().await;
    let disk_percent   = read_disk_percent().await;
    let uptime_secs    = read_uptime_secs().await;
    let (mem_total_mb, mem_used_mb, mem_avail_mb) = read_memory_detailed().await;
    let (swap_total_mb, swap_used_mb) = read_swap().await;
    let memory_percent = if mem_total_mb > 0 {
        (mem_used_mb as f32 / mem_total_mb as f32) * 100.0
    } else { 0.0 };
    let _ = mem_avail_mb; // used only to compute mem_used_mb above

    // ── Température CPU ───────────────────────────────────────────────────────
    let cpu_temp_c = read_cpu_temp().await;

    // ── Processus ─────────────────────────────────────────────────────────────
    let processes = collect_processes().await;

    MonitorSnapshot {
        timestamp: Utc::now(),
        services,
        network_services,
        serial_port_ok,
        load_avg,
        cpu_percent,
        memory_percent,
        disk_percent,
        uptime_secs,
        auto_actions,
        processes,
        mem_total_mb,
        mem_used_mb,
        swap_total_mb,
        swap_used_mb,
        net_rx_bps,
        net_tx_bps,
        cpu_temp_c,
    }
}

// =============================================================================
// Processus — ps aux
// =============================================================================

async fn collect_processes() -> Vec<ProcessInfo> {
    let out = Command::new("ps")
        .args(["--no-headers", "-eo", "pid,%cpu,%mem,rss,stat,comm", "--sort=-%cpu"])
        .output()
        .await;
    let Ok(out) = out else { return vec![] };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .take(20)
        .filter_map(|line| {
            let mut p = line.split_whitespace();
            let pid: u32       = p.next()?.parse().ok()?;
            let cpu: f32       = p.next()?.parse().ok()?;
            let mem: f32       = p.next()?.parse().ok()?;
            let rss_kb: u64    = p.next()?.parse().ok()?;
            let state          = p.next().unwrap_or("?").to_string();
            let name           = p.next().unwrap_or("?").to_string();
            Some(ProcessInfo {
                pid,
                name,
                cpu_percent: cpu,
                mem_percent: mem,
                mem_rss_mb: rss_kb as f32 / 1024.0,
                state,
            })
        })
        .collect()
}

// =============================================================================
// Réseau I/O — /proc/net/dev
// =============================================================================

/// Retourne (rx_bytes_total, tx_bytes_total) pour toutes les ifaces non-loopback.
async fn read_net_bytes() -> (u64, u64) {
    let Ok(content) = tokio::fs::read_to_string("/proc/net/dev").await else {
        return (0, 0);
    };
    let mut rx_total = 0u64;
    let mut tx_total = 0u64;
    for line in content.lines() {
        let line = line.trim();
        // Skip header lines and loopback
        let Some(colon_pos) = line.find(':') else { continue };
        let iface = line[..colon_pos].trim();
        if iface == "lo" { continue; }
        let data = &line[colon_pos + 1..];
        let mut fields = data.split_whitespace();
        if let Some(rx) = fields.next().and_then(|v| v.parse::<u64>().ok()) {
            rx_total += rx;
        }
        // Skip 7 receive fields (packets, errs, drop, fifo, frame, compressed, multicast)
        for _ in 0..7 { fields.next(); }
        if let Some(tx) = fields.next().and_then(|v| v.parse::<u64>().ok()) {
            tx_total += tx;
        }
    }
    (rx_total, tx_total)
}

// =============================================================================
// Température CPU
// =============================================================================

async fn read_cpu_temp() -> Option<f32> {
    // Raspberry Pi 5 / generic Linux thermal zone
    let paths = [
        "/sys/class/thermal/thermal_zone0/temp",
        "/sys/class/hwmon/hwmon0/temp1_input",
    ];
    for path in &paths {
        if let Ok(s) = tokio::fs::read_to_string(path).await {
            if let Ok(v) = s.trim().parse::<f32>() {
                return Some(v / 1000.0);
            }
        }
    }
    None
}

// =============================================================================
// Mémoire détaillée
// =============================================================================

/// Retourne (total_mb, used_mb, available_mb)
async fn read_memory_detailed() -> (u64, u64, u64) {
    let Ok(content) = tokio::fs::read_to_string("/proc/meminfo").await else {
        return (0, 0, 0);
    };
    let mut total: u64 = 0;
    let mut available: u64 = 0;
    for line in content.lines() {
        if line.starts_with("MemTotal:") {
            total = line.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        } else if line.starts_with("MemAvailable:") {
            available = line.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        }
    }
    let used = total.saturating_sub(available);
    (total / 1024, used / 1024, available / 1024)
}

async fn read_swap() -> (u64, u64) {
    let Ok(content) = tokio::fs::read_to_string("/proc/meminfo").await else {
        return (0, 0);
    };
    let mut total: u64 = 0;
    let mut free: u64 = 0;
    for line in content.lines() {
        if line.starts_with("SwapTotal:") {
            total = line.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        } else if line.starts_with("SwapFree:") {
            free = line.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        }
    }
    (total / 1024, total.saturating_sub(free) / 1024)
}

// =============================================================================
// Helpers système (inchangés)
// =============================================================================

async fn tcp_probe(host: &str, port: u16) -> bool {
    let addr = format!("{}:{}", host, port);
    tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(addr))
        .await.map(|r| r.is_ok()).unwrap_or(false)
}

async fn check_systemd_service(name: &str) -> String {
    match Command::new("systemctl").args(["is-active", name]).output().await {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => "unknown".to_string(),
    }
}

async fn read_load_avg() -> [f32; 3] {
    tokio::fs::read_to_string("/proc/loadavg").await.ok()
        .and_then(|s| {
            let mut p = s.split_whitespace();
            let a: f32 = p.next()?.parse().ok()?;
            let b: f32 = p.next()?.parse().ok()?;
            let c: f32 = p.next()?.parse().ok()?;
            Some([a, b, c])
        })
        .unwrap_or([0.0, 0.0, 0.0])
}

async fn read_cpu_percent() -> f32 {
    let read_stat = || async {
        tokio::fs::read_to_string("/proc/stat").await.ok().and_then(|s| {
            let line = s.lines().next()?;
            let mut parts = line.split_whitespace().skip(1);
            let user:    u64 = parts.next()?.parse().ok()?;
            let nice:    u64 = parts.next()?.parse().ok()?;
            let system:  u64 = parts.next()?.parse().ok()?;
            let idle:    u64 = parts.next()?.parse().ok()?;
            let iowait:  u64 = parts.next()?.parse().ok()?;
            let irq:     u64 = parts.next()?.parse().ok()?;
            let softirq: u64 = parts.next()?.parse().ok()?;
            let total = user + nice + system + idle + iowait + irq + softirq;
            Some((total, idle + iowait))
        })
    };
    let before = read_stat().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let after  = read_stat().await;
    if let (Some((t1, i1)), Some((t2, i2))) = (before, after) {
        let dt = (t2 - t1) as f32;
        let di = (i2 - i1) as f32;
        if dt > 0.0 { (1.0 - di / dt) * 100.0 } else { 0.0 }
    } else { 0.0 }
}

async fn read_disk_percent() -> f32 {
    match Command::new("df").args(["-h", "--output=pcent", "/"]).output().await {
        Ok(out) => {
            let s = String::from_utf8_lossy(&out.stdout);
            s.lines().nth(1)
                .and_then(|l| l.trim().trim_end_matches('%').parse::<f32>().ok())
                .unwrap_or(0.0)
        }
        Err(_) => 0.0,
    }
}

async fn read_uptime_secs() -> u64 {
    tokio::fs::read_to_string("/proc/uptime").await.ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .map(|v| v as u64)
        .unwrap_or(0)
}

// =============================================================================
// Watchdog
// =============================================================================

const WATCHDOG_SERVICES: &[(&str, &str, u16, Option<&str>)] = &[
    // (label, host, port, systemd_unit_à_restart_si_injoignable)
    // L'unit énergy-manager.service doit être restartable par dalybms
    // via la règle polkit `contrib/polkit/50-dalybms-systemd-restart.rules`.
    ("energy-manager", "127.0.0.1", 8081, Some("energy-manager")),
];

const WATCHDOG_INTERVAL:      Duration = Duration::from_secs(15);
const WATCHDOG_CONFIRM_DELAY: Duration = Duration::from_secs(5);
const WATCHDOG_COOLDOWN:      Duration = Duration::from_secs(120);

pub async fn run_watchdog_agent(_state: AppState) {
    info!("Watchdog démarré (intervalle: {}s, cooldown: {}s)",
        WATCHDOG_INTERVAL.as_secs(), WATCHDOG_COOLDOWN.as_secs());
    let mut ticker       = interval(WATCHDOG_INTERVAL);
    let mut last_restart: Vec<Option<Instant>> = vec![None; WATCHDOG_SERVICES.len()];

    loop {
        ticker.tick().await;
        for (idx, &(name, host, port, systemd_unit)) in WATCHDOG_SERVICES.iter().enumerate() {
            if !tcp_probe(host, port).await {
                tokio::time::sleep(WATCHDOG_CONFIRM_DELAY).await;
                if tcp_probe(host, port).await { continue; }

                let cooldown_ok = last_restart[idx]
                    .map(|t| t.elapsed() >= WATCHDOG_COOLDOWN)
                    .unwrap_or(true);
                if !cooldown_ok {
                    warn!("Watchdog: {} injoignable mais cooldown actif", name);
                    continue;
                }

                warn!("Watchdog: {} injoignable — tentative de redémarrage", name);
                if let Some(unit) = systemd_unit {
                    if restart_systemd_service(unit).await {
                        info!("Watchdog: {} redémarré avec succès", name);
                        last_restart[idx] = Some(Instant::now());
                    } else {
                        warn!("Watchdog: échec redémarrage de {}", name);
                    }
                }
            }
        }
    }
}

// =============================================================================
// Métriques système → metrics-store (redb)
// =============================================================================

// Phase 5 cleanup : la fonction build_monitor_vm_rows() qui sérialisait les
// métriques système (pi5_cpu_percent, pi5_memory_percent, pi5_disk_percent,
// pi5_load_avg, pi5_cpu_temp_c, pi5_process_*) au format VmRow a été retirée
// avec le client VM. Pour réintroduire l'export de ces séries vers
// metrics-store, ajouter un hook dans state.rs::on_monitor_snapshot qui
// pousse directement dans le Writer redb (TODO si besoin réel d'observabilité
// système en prod).

// =============================================================================
// Point d'entrée unique — remplace les deux tokio::spawn dans main.rs
// =============================================================================

/// Démarre monitor_agent et watchdog_agent en arrière-plan.
/// Appeler depuis main.rs à la place des deux tokio::spawn séparés.
///
/// Note : l'instrumentation `tokio_metrics::TaskMonitor` a été retirée
/// après Phase 5 (l'exporteur VM qui consommait ces métriques a été retiré
/// avec le client VM, donc le TaskMonitor était orphelin). Le TaskMonitor
/// retenait un `Arc<RawMetrics>` par task instrumentée pour la durée de vie
/// du programme — supprimé pour éliminer cette rétention (audit fuite
/// mémoire phase 2.3, mai 2026).
pub fn spawn_all(state: AppState) {
    // Supervision (audit robustesse §2/§4) : si un agent se termine (retour) —
    // ce qui ne doit jamais arriver pour une boucle de monitoring/watchdog —
    // `spawn_critical` arrête le process pour un redémarrage propre par systemd,
    // au lieu de laisser le heartbeat manquant déclencher WatchdogSec (60 s). Le
    // heartbeat systemd vit dans monitor_agent : sa mort silencieuse donnait
    // auparavant une fausse impression de santé. Panics déjà fatals (panic=abort).
    let s_mon = state.clone();
    crate::spawn_critical(async move {
        run_monitor_agent(s_mon).await;
        tracing::error!("monitor_agent terminé de façon inattendue");
    });
    let s_wd = state.clone();
    crate::spawn_critical(async move {
        run_watchdog_agent(s_wd).await;
        tracing::error!("watchdog_agent terminé de façon inattendue");
    });
}

/// Redémarre une unité systemd. Nécessite une règle polkit autorisant
/// l'utilisateur `dalybms` à gérer l'unité concernée (voir
/// `contrib/polkit/50-dalybms-systemd-restart.rules`).
async fn restart_systemd_service(name: &str) -> bool {
    match Command::new("systemctl").args(["restart", name]).output().await {
        Ok(out) => {
            if !out.status.success() {
                warn!("systemctl restart {} → stderr: {}",
                    name, String::from_utf8_lossy(&out.stderr).trim());
            }
            out.status.success()
        }
        Err(e) => { warn!("systemctl restart {} → erreur: {}", name, e); false }
    }
}
