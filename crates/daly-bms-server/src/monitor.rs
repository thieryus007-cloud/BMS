//! Agent de monitoring autonome — surveille l'ensemble du système Pi5.
//!
//! Vérifie toutes les 30 secondes :
//! - Services systemd daly-bms + energy-manager (via systemctl)
//! - Services réseau via sonde TCP : mosquitto, influxdb, energy-manager, venus MQTT
//! - Port série RS485 (/dev/ttyUSB0)
//! - CPU, RAM, disque, charge système, uptime
//!
//! Action automatique : si un conteneur Docker est injoignable → `docker restart`

use crate::state::{AppState, MonitorSnapshot, ServiceStatus};
use chrono::Utc;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::time::interval;
use tracing::{info, warn};

/// Services réseau à sonder : (label, host, port, conteneur_docker_pour_restart).
const TCP_SERVICES: &[(&str, &str, u16, Option<&str>)] = &[
    ("mosquitto",      "127.0.0.1",     1883, Some("mosquitto")),
    ("influxdb",       "127.0.0.1",     8086, Some("influxdb")),
    ("energy-manager", "127.0.0.1",     8081, None),
    ("venus-mqtt",     "192.168.1.120", 1883, None),
];

/// Port série RS485.
const RS485_PORT: &str = "/dev/ttyUSB0";

/// Démarre l'agent de monitoring en arrière-plan.
pub async fn run_monitor_agent(state: AppState) {
    info!("Agent de monitoring Pi5 démarré (intervalle: 30s)");
    let mut ticker = interval(Duration::from_secs(30));

    loop {
        ticker.tick().await;
        let snap = collect_snapshot(&state).await;
        state.on_monitor_snapshot(snap).await;
    }
}

/// Collecte un snapshot complet de l'état du système.
async fn collect_snapshot(_state: &AppState) -> MonitorSnapshot {
    let mut services = Vec::new();
    let mut network_services = Vec::new();
    let mut auto_actions = Vec::new();

    // ── Services systemd ──────────────────────────────────────────────────────
    // daly-bms : nous sommes le processus en cours — on force active:true.
    let daly_status = check_systemd_service("daly-bms").await;
    services.push(ServiceStatus {
        name: "daly-bms".to_string(),
        active: true,
        status: if daly_status.is_empty() { "active".to_string() } else { daly_status },
    });

    // energy-manager : vérification via systemctl + fallback sonde TCP :8081.
    let em_status  = check_systemd_service("energy-manager").await;
    let em_systemd = em_status == "active";
    // Si systemd dit inactive, vérifier quand même via TCP (service démarré manuellement).
    let em_tcp_ok  = if em_systemd { true } else { tcp_probe("127.0.0.1", 8081).await };
    let em_active  = em_systemd || em_tcp_ok;
    services.push(ServiceStatus {
        name: "energy-manager".to_string(),
        active: em_active,
        status: if em_active {
            if em_systemd { em_status } else { "active (port 8081)".to_string() }
        } else {
            if em_status.is_empty() { "unknown".to_string() } else { em_status }
        },
    });

    // ── Sondes TCP ───────────────────────────────────────────────────────────
    for &(name, host, port, docker_name) in TCP_SERVICES {
        let reachable = tcp_probe(host, port).await;

        if !reachable {
            if let Some(cname) = docker_name {
                if restart_docker_container(cname).await {
                    let msg = format!("Redémarré conteneur Docker: {}", cname);
                    info!("{}", msg);
                    auto_actions.push(msg);
                    network_services.push(ServiceStatus {
                        name: name.to_string(),
                        active: false,
                        status: "restarted".to_string(),
                    });
                } else {
                    warn!("Échec redémarrage conteneur Docker: {}", cname);
                    network_services.push(ServiceStatus {
                        name: name.to_string(),
                        active: false,
                        status: "down".to_string(),
                    });
                }
            } else {
                network_services.push(ServiceStatus {
                    name: name.to_string(),
                    active: false,
                    status: "unreachable".to_string(),
                });
            }
        } else {
            network_services.push(ServiceStatus {
                name: name.to_string(),
                active: true,
                status: format!("{}:{}", host, port),
            });
        }
    }

    // ── Port série RS485 ─────────────────────────────────────────────────────
    let serial_port_ok = tokio::fs::metadata(RS485_PORT).await.is_ok();

    // ── Métriques système ────────────────────────────────────────────────────
    let load_avg       = read_load_avg().await;
    let cpu_percent    = read_cpu_percent().await;
    let memory_percent = read_memory_percent().await;
    let disk_percent   = read_disk_percent().await;
    let uptime_secs    = read_uptime_secs().await;

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
    }
}

/// Sonde TCP avec timeout 2 secondes.
async fn tcp_probe(host: &str, port: u16) -> bool {
    let addr = format!("{}:{}", host, port);
    tokio::time::timeout(
        Duration::from_secs(2),
        TcpStream::connect(addr),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

/// Vérifie l'état d'un service systemd.
async fn check_systemd_service(name: &str) -> String {
    match Command::new("systemctl")
        .args(["is-active", name])
        .output()
        .await
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => "unknown".to_string(),
    }
}

/// Tente de redémarrer un conteneur Docker.
async fn restart_docker_container(name: &str) -> bool {
    match Command::new("docker")
        .args(["restart", name])
        .output()
        .await
    {
        Ok(out) => out.status.success(),
        Err(_)  => false,
    }
}

/// Lit la charge système depuis `/proc/loadavg` → [1min, 5min, 15min].
async fn read_load_avg() -> [f32; 3] {
    tokio::fs::read_to_string("/proc/loadavg")
        .await
        .ok()
        .and_then(|s| {
            let mut p = s.split_whitespace();
            let a: f32 = p.next()?.parse().ok()?;
            let b: f32 = p.next()?.parse().ok()?;
            let c: f32 = p.next()?.parse().ok()?;
            Some([a, b, c])
        })
        .unwrap_or([0.0, 0.0, 0.0])
}

/// Lit l'utilisation CPU depuis `/proc/stat` (deux lectures avec 200ms d'écart).
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
    } else {
        0.0
    }
}

/// Lit l'utilisation mémoire depuis `/proc/meminfo`.
async fn read_memory_percent() -> f32 {
    let Ok(content) = tokio::fs::read_to_string("/proc/meminfo").await else { return 0.0; };
    let mut total = 0u64;
    let mut available = 0u64;
    for line in content.lines() {
        if line.starts_with("MemTotal:") {
            total = line.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        } else if line.starts_with("MemAvailable:") {
            available = line.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        }
    }
    if total > 0 { ((total - available) as f32 / total as f32) * 100.0 } else { 0.0 }
}

/// Lit l'utilisation disque via `df /`.
async fn read_disk_percent() -> f32 {
    match Command::new("df")
        .args(["-h", "--output=pcent", "/"])
        .output()
        .await
    {
        Ok(out) => {
            let s = String::from_utf8_lossy(&out.stdout);
            s.lines()
                .nth(1)
                .and_then(|l| l.trim().trim_end_matches('%').parse::<f32>().ok())
                .unwrap_or(0.0)
        }
        Err(_) => 0.0,
    }
}

/// Lit l'uptime depuis `/proc/uptime`.
async fn read_uptime_secs() -> u64 {
    tokio::fs::read_to_string("/proc/uptime")
        .await
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .map(|v| v as u64)
        .unwrap_or(0)
}

// ─────────────────────────────────────────────────────────────────────────────
// Watchdog — redémarre les services critiques en cas de crash
// ─────────────────────────────────────────────────────────────────────────────

/// Services à surveiller : (label, host, port, commande_systemd).
/// Si la commande est None le service n'est pas redémarré automatiquement.
const WATCHDOG_SERVICES: &[(&str, &str, u16, Option<&str>)] = &[
    // None = pas de restart automatique : NoNewPrivileges=true empêche sudo,
    // systemd gère le restart via Restart=on-failure dans energy-manager.service.
    ("energy-manager", "127.0.0.1", 8081, None),
];

/// Intervalle de vérification du watchdog.
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(15);
/// Délai de confirmation avant redémarrage (évite les faux-positifs transitoires).
const WATCHDOG_CONFIRM_DELAY: Duration = Duration::from_secs(5);
/// Cooldown minimum entre deux redémarrages du même service.
const WATCHDOG_COOLDOWN: Duration = Duration::from_secs(120);

/// Démarre le watchdog en arrière-plan.
/// Vérifie tous les `WATCHDOG_INTERVAL` que chaque service répond.
/// Si un service est injoignable après confirmation, il est redémarré via systemctl.
pub async fn run_watchdog_agent(_state: AppState) {
    info!("Watchdog démarré (intervalle: {}s, cooldown: {}s)",
        WATCHDOG_INTERVAL.as_secs(), WATCHDOG_COOLDOWN.as_secs());

    let mut ticker = interval(WATCHDOG_INTERVAL);
    // Dernière tentative de redémarrage par service (même index que WATCHDOG_SERVICES).
    let mut last_restart: Vec<Option<Instant>> = vec![None; WATCHDOG_SERVICES.len()];

    loop {
        ticker.tick().await;

        for (idx, &(name, host, port, systemd_unit)) in WATCHDOG_SERVICES.iter().enumerate() {
            if !tcp_probe(host, port).await {
                // Sonde de confirmation après délai court.
                tokio::time::sleep(WATCHDOG_CONFIRM_DELAY).await;
                if tcp_probe(host, port).await {
                    continue; // faux positif — service de nouveau joignable
                }

                let cooldown_ok = last_restart[idx]
                    .map(|t| t.elapsed() >= WATCHDOG_COOLDOWN)
                    .unwrap_or(true);

                if !cooldown_ok {
                    warn!("Watchdog: {} injoignable mais cooldown actif ({:.0}s restantes)",
                        name,
                        WATCHDOG_COOLDOWN.as_secs_f32()
                            - last_restart[idx].unwrap().elapsed().as_secs_f32());
                    continue;
                }

                warn!("Watchdog: {} injoignable — tentative de redémarrage", name);

                if let Some(unit) = systemd_unit {
                    if restart_systemd_service(unit).await {
                        info!("Watchdog: {} redémarré avec succès via systemctl", name);
                        last_restart[idx] = Some(Instant::now());
                    } else {
                        warn!("Watchdog: échec du redémarrage systemctl de {}", name);
                    }
                }
            }
        }
    }
}

/// Redémarre un service systemd via `sudo systemctl restart <name>`.
async fn restart_systemd_service(name: &str) -> bool {
    match Command::new("sudo")
        .args(["systemctl", "restart", name])
        .output()
        .await
    {
        Ok(out) => {
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                warn!("systemctl restart {} → stderr: {}", name, stderr.trim());
            }
            out.status.success()
        }
        Err(e) => {
            warn!("systemctl restart {} → erreur: {}", name, e);
            false
        }
    }
}
