//! Agent de monitoring système pour energy-manager.
//!
//! Collecte toutes les 60 secondes :
//! - CPU, RAM, swap, disque, load average, température CPU
//! - I/O réseau (octets/s)
//! - Top processus (par CPU%)
//! - Métriques runtime tokio (polls, latences)
//!
//! Exporte vers VictoriaMetrics via POST /api/v1/import/prometheus.

use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::interval;
use tracing::warn;

use crate::bus::AppBus;
use crate::types::MqttOutgoing;

// =============================================================================
// Point d'entrée
// =============================================================================

/// Spawne l'agent de monitoring en arrière-plan.
pub fn spawn(vm_url: String, bus: AppBus) {
    use tokio_metrics::TaskMonitor;

    let task_mon = TaskMonitor::new();
    let vm_url_mon = vm_url.clone();
    let bus_mon    = bus.clone();
    tokio::spawn(task_mon.instrument(async move {
        run_monitoring_loop(vm_url_mon, bus_mon).await;
    }));

    // Exporteur métriques tokio → VM (toutes les 60s)
    tokio::spawn(async move {
        let mut ivals  = task_mon.intervals();
        let mut ticker = interval(Duration::from_secs(60));
        let client     = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        loop {
            ticker.tick().await;
            let Some(m) = ivals.next() else { continue };
            let ts_ms = chrono::Utc::now().timestamp_millis();
            let lines = vec![
                format!("em_tokio_task_polls_total{{task=\"energy_manager_monitor\"}} {} {}", m.total_poll_count, ts_ms),
                format!("em_tokio_task_mean_poll_us{{task=\"energy_manager_monitor\"}} {} {}", m.mean_poll_duration().as_micros(), ts_ms),
                format!("em_tokio_task_mean_scheduled_us{{task=\"energy_manager_monitor\"}} {} {}", m.mean_scheduled_duration().as_micros(), ts_ms),
            ];
            write_to_vm(&client, &vm_url, &lines.join("\n")).await;
        }
    });
}

// =============================================================================
// Boucle principale
// =============================================================================

async fn run_monitoring_loop(vm_url: String, bus: AppBus) {
    tracing::info!("energy-manager monitoring démarré (intervalle: 60s)");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    let mut ticker      = interval(Duration::from_secs(60));
    let mut prev_net    = read_net_bytes().await;
    let mut prev_net_ts = Instant::now();

    loop {
        ticker.tick().await;

        let curr_net   = read_net_bytes().await;
        let elapsed_s  = prev_net_ts.elapsed().as_secs_f64().max(1.0);
        let net_rx_bps = ((curr_net.0.saturating_sub(prev_net.0)) as f64 / elapsed_s) as u64;
        let net_tx_bps = ((curr_net.1.saturating_sub(prev_net.1)) as f64 / elapsed_s) as u64;
        prev_net       = curr_net;
        prev_net_ts    = Instant::now();

        let ts_ms                           = chrono::Utc::now().timestamp_millis();
        let cpu_percent                     = read_cpu_percent().await;
        let (mem_total, mem_used, _)        = read_memory().await;
        let (swap_total, swap_used)         = read_swap().await;
        let disk_percent                    = read_disk_percent().await;
        let load_avg                        = read_load_avg().await;
        let cpu_temp                        = read_cpu_temp().await;
        let processes                       = collect_processes().await;
        let mem_percent = if mem_total > 0 { (mem_used as f64 / mem_total as f64) * 100.0 } else { 0.0 };

        let mut lines = vec![
            format!("em_cpu_percent {} {}", cpu_percent, ts_ms),
            format!("em_memory_percent {} {}", mem_percent, ts_ms),
            format!("em_mem_used_mb {} {}", mem_used, ts_ms),
            format!("em_swap_used_mb {} {}", swap_used, ts_ms),
            format!("em_disk_percent {} {}", disk_percent, ts_ms),
            format!("em_net_rx_bps {} {}", net_rx_bps, ts_ms),
            format!("em_net_tx_bps {} {}", net_tx_bps, ts_ms),
            format!("em_load_avg{{window=\"1m\"}} {} {}", load_avg[0], ts_ms),
            format!("em_load_avg{{window=\"5m\"}} {} {}", load_avg[1], ts_ms),
            format!("em_load_avg{{window=\"15m\"}} {} {}", load_avg[2], ts_ms),
        ];
        let _ = (mem_total, swap_total); // calculés mais non exportés directement

        if let Some(temp) = cpu_temp {
            lines.push(format!("em_cpu_temp_c {} {}", temp, ts_ms));
        }
        // Agrégation par nom de processus pour éviter la cardinalité éphémère du
        // `pid` (chaque compilation Rust créait des séries fantômes survivant 5 ans
        // dans la rétention VM). Tuple : (cpu_percent, mem_mb, name, pid).
        let mut by_name: std::collections::HashMap<&str, (f32, f32)> = std::collections::HashMap::new();
        for proc in &processes {
            if proc.0 > 0.1 {
                let entry = by_name.entry(proc.2.as_str()).or_insert((0.0, 0.0));
                entry.0 += proc.0;
                entry.1 += proc.1;
            }
        }
        for (name, (cpu, mem)) in by_name {
            lines.push(format!(
                "em_process_cpu_percent{{process=\"{}\"}} {} {}",
                name, cpu, ts_ms
            ));
            lines.push(format!(
                "em_process_mem_mb{{process=\"{}\"}} {} {}",
                name, mem, ts_ms
            ));
        }

        write_to_vm(&client, &vm_url, &lines.join("\n")).await;

        // Publication MQTT vers `santuario/em/metrics` (consommée par daly-bms-server).
        let payload = serde_json::json!({
            "cpu_percent":    cpu_percent,
            "memory_percent": mem_percent,
            "mem_used_mb":    mem_used,
            "swap_used_mb":   swap_used,
            "disk_percent":   disk_percent,
            "load_avg_1m":    load_avg[0],
            "load_avg_5m":    load_avg[1],
            "load_avg_15m":   load_avg[2],
            "net_rx_bps":     net_rx_bps,
            "net_tx_bps":     net_tx_bps,
            "cpu_temp_c":     cpu_temp,
        });
        bus.publish(MqttOutgoing::transient("santuario/em/metrics", payload)).await;
    }
}

// =============================================================================
// Écriture VM
// =============================================================================

async fn write_to_vm(client: &reqwest::Client, vm_url: &str, body: &str) {
    let url = format!("{}/api/v1/import/prometheus", vm_url.trim_end_matches('/'));
    if let Err(e) = client
        .post(&url)
        .header("Content-Type", "text/plain")
        .body(body.to_string())
        .send()
        .await
    {
        warn!("energy-manager monitoring VM write: {}", e);
    }
}

// =============================================================================
// Lectures système (identiques à daly-bms-server/monitor.rs — /proc)
// =============================================================================

/// (rx_total, tx_total) pour toutes les interfaces non-loopback
async fn read_net_bytes() -> (u64, u64) {
    let Ok(content) = tokio::fs::read_to_string("/proc/net/dev").await else { return (0, 0) };
    let mut rx = 0u64;
    let mut tx = 0u64;
    for line in content.lines() {
        let line = line.trim();
        let Some(colon) = line.find(':') else { continue };
        if line[..colon].trim() == "lo" { continue }
        let mut f = line[colon + 1..].split_whitespace();
        if let Some(r) = f.next().and_then(|v| v.parse::<u64>().ok()) { rx += r; }
        for _ in 0..7 { f.next(); }
        if let Some(t) = f.next().and_then(|v| v.parse::<u64>().ok()) { tx += t; }
    }
    (rx, tx)
}

/// (total_mb, used_mb, avail_mb)
async fn read_memory() -> (u64, u64, u64) {
    let Ok(s) = tokio::fs::read_to_string("/proc/meminfo").await else { return (0, 0, 0) };
    let mut total = 0u64;
    let mut avail = 0u64;
    for line in s.lines() {
        if line.starts_with("MemTotal:")     { total = parse_kb(line); }
        else if line.starts_with("MemAvailable:") { avail = parse_kb(line); }
    }
    (total / 1024, total.saturating_sub(avail) / 1024, avail / 1024)
}

async fn read_swap() -> (u64, u64) {
    let Ok(s) = tokio::fs::read_to_string("/proc/meminfo").await else { return (0, 0) };
    let mut total = 0u64;
    let mut free  = 0u64;
    for line in s.lines() {
        if line.starts_with("SwapTotal:") { total = parse_kb(line); }
        else if line.starts_with("SwapFree:") { free = parse_kb(line); }
    }
    (total / 1024, total.saturating_sub(free) / 1024)
}

fn parse_kb(line: &str) -> u64 {
    line.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0)
}

async fn read_cpu_percent() -> f64 {
    let read_stat = || async {
        tokio::fs::read_to_string("/proc/stat").await.ok().and_then(|s| {
            let line = s.lines().next()?;
            let mut p = line.split_whitespace().skip(1);
            let user:    u64 = p.next()?.parse().ok()?;
            let nice:    u64 = p.next()?.parse().ok()?;
            let system:  u64 = p.next()?.parse().ok()?;
            let idle:    u64 = p.next()?.parse().ok()?;
            let iowait:  u64 = p.next()?.parse().ok()?;
            let irq:     u64 = p.next()?.parse().ok()?;
            let softirq: u64 = p.next()?.parse().ok()?;
            let total = user + nice + system + idle + iowait + irq + softirq;
            Some((total, idle + iowait))
        })
    };
    let before = read_stat().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let after  = read_stat().await;
    if let (Some((t1, i1)), Some((t2, i2))) = (before, after) {
        let dt = (t2 - t1) as f64;
        let di = (i2 - i1) as f64;
        if dt > 0.0 { (1.0 - di / dt) * 100.0 } else { 0.0 }
    } else { 0.0 }
}

async fn read_disk_percent() -> f64 {
    match Command::new("df").args(["-h", "--output=pcent", "/"]).output().await {
        Ok(out) => String::from_utf8_lossy(&out.stdout)
            .lines().nth(1)
            .and_then(|l| l.trim().trim_end_matches('%').parse::<f64>().ok())
            .unwrap_or(0.0),
        Err(_) => 0.0,
    }
}

async fn read_load_avg() -> [f64; 3] {
    tokio::fs::read_to_string("/proc/loadavg").await.ok()
        .and_then(|s| {
            let mut p = s.split_whitespace();
            let a: f64 = p.next()?.parse().ok()?;
            let b: f64 = p.next()?.parse().ok()?;
            let c: f64 = p.next()?.parse().ok()?;
            Some([a, b, c])
        })
        .unwrap_or([0.0, 0.0, 0.0])
}

async fn read_cpu_temp() -> Option<f64> {
    for path in ["/sys/class/thermal/thermal_zone0/temp", "/sys/class/hwmon/hwmon0/temp1_input"] {
        if let Ok(s) = tokio::fs::read_to_string(path).await {
            if let Ok(v) = s.trim().parse::<f64>() {
                return Some(v / 1000.0);
            }
        }
    }
    None
}

/// Retourne (cpu%, mem_mb, name, pid) pour les top 10 processus
async fn collect_processes() -> Vec<(f32, f32, String, String)> {
    let out = Command::new("ps")
        .args(["--no-headers", "-eo", "pid,%cpu,rss,comm", "--sort=-%cpu"])
        .output()
        .await;
    let Ok(out) = out else { return vec![] };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .take(10)
        .filter_map(|line| {
            let mut p = line.split_whitespace();
            let pid:    String = p.next()?.to_string();
            let cpu:    f32    = p.next()?.parse().ok()?;
            let rss_kb: f32    = p.next()?.parse().ok()?;
            let name:   String = p.next()?.to_string();
            Some((cpu, rss_kb / 1024.0, name, pid))
        })
        .collect()
}
