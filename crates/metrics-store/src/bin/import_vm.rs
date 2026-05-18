//! `import-vm` — rapatrie l'historique VictoriaMetrics dans une base redb.
//!
//! Cf. `docs/plan_migration_vm_redb.md` §0.7 J2-J3.
//!
//! ## Pré-requis
//! - `daly-bms` ARRÊTÉ (sinon contention writer sur la même base redb).
//! - VM accessible.
//!
//! ## Usage
//! ```bash
//! # Mode normal — pipe la sortie d'export VM dans l'outil
//! curl -s 'http://127.0.0.1:8428/api/v1/export?match\[\]={__name__!=""}' \
//!   | import-vm --db /mnt/nvme/daly-bms/metrics.redb
//!
//! # Dry-run — parse uniquement, n'écrit rien (validation du flux)
//! curl -s 'http://127.0.0.1:8428/api/v1/export?match\[\]={__name__!=""}' \
//!   | import-vm --db /tmp/dryrun.redb --dry-run
//!
//! # Filtrer une seule métrique (utile pour tests)
//! curl -s 'http://127.0.0.1:8428/api/v1/export?match\[\]=bms_voltage' \
//!   | import-vm --db /mnt/nvme/daly-bms/metrics.redb --limit 1000
//! ```
//!
//! ## Format VM `/api/v1/export`
//! JSONL — une ligne par série, avec :
//! ```json
//! {"metric":{"__name__":"foo","bms_id":"0x01"},
//!  "values":[1.0, 2.0, ...],
//!  "timestamps":[1700000000000, 1700000060000, ...]}
//! ```
//! Les timestamps sont déjà en millisecondes (format natif de notre `Sample`).
//!
//! ## Idempotence
//! Re-lancer l'import écrase les mêmes (series_id, ts) avec les mêmes
//! valeurs. Pas de doublons en base. La sérialisation des labels est
//! canonique (`labels.rs`) donc l'ordre d'arrivée des labels VM ne crée
//! pas de séries distinctes.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use metrics_store::{MetricsStore, Options, Sample, WriterConfig};
use serde::Deserialize;
use serde_json::Value;
use smallvec::SmallVec;

#[derive(Parser, Debug)]
#[command(version, about = "Import VM → redb pour la migration Phase 4")]
struct Args {
    /// Chemin de la base redb cible
    #[arg(long)]
    db: PathBuf,
    /// Mode dry-run : parse stdin et compte, sans écrire en base
    #[arg(long)]
    dry_run: bool,
    /// Limite optionnelle du nombre total de samples à importer (debug)
    #[arg(long)]
    limit: Option<usize>,
    /// Profondeur de queue mpsc (défaut 200 000 pour absorber les pics)
    #[arg(long, default_value_t = 200_000)]
    queue_depth: usize,
    /// Taille max des batchs writer (défaut 5 000 pour throughput import)
    #[arg(long, default_value_t = 5_000)]
    batch_max: usize,
    /// Cache redb en MiB (défaut 256 — plus généreux que la prod 64)
    #[arg(long, default_value_t = 256)]
    cache_mb: usize,
}

#[derive(Deserialize)]
struct VmSeriesLine {
    metric: serde_json::Map<String, Value>,
    values: Vec<f64>,
    timestamps: Vec<i64>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    eprintln!(
        "import-vm: db={}, dry_run={}, queue={}, batch_max={}, cache={}MiB",
        args.db.display(),
        args.dry_run,
        args.queue_depth,
        args.batch_max,
        args.cache_mb,
    );

    let store = if args.dry_run {
        None
    } else {
        let opts = Options {
            cache_bytes: args.cache_mb * 1024 * 1024,
            writer_queue_depth: args.queue_depth,
            writer: WriterConfig {
                batch_max: args.batch_max,
                flush_ms: 100,
                poll_idle_ms: 1,
            },
        };
        Some(MetricsStore::open(&args.db, opts).context("ouverture base redb")?)
    };
    let writer = store.as_ref().map(|s| s.writer());

    let stdin = std::io::stdin();
    let reader = BufReader::with_capacity(64 * 1024, stdin.lock());

    let start = Instant::now();
    let mut series_count = 0_usize;
    let mut sample_count = 0_usize;
    let mut skipped = 0_usize;
    let mut last_log = Instant::now();

    for (lineno, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("lecture ligne stdin #{lineno}"))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parsed: VmSeriesLine = match serde_json::from_str(trimmed) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("WARN: parse ligne #{lineno}: {e} — ignorée");
                skipped += 1;
                continue;
            }
        };
        if parsed.values.len() != parsed.timestamps.len() {
            eprintln!(
                "WARN: ligne #{lineno} : values.len ({}) != timestamps.len ({}) — ignorée",
                parsed.values.len(),
                parsed.timestamps.len()
            );
            skipped += 1;
            continue;
        }

        // Sépare __name__ des autres labels (le shim attend __name__ hors map).
        let mut metric_name = String::new();
        let mut labels: SmallVec<[(String, String); 4]> = SmallVec::new();
        for (k, v) in &parsed.metric {
            let v_str = v.as_str().unwrap_or("").to_string();
            if k == "__name__" {
                metric_name = v_str;
            } else {
                labels.push((k.clone(), v_str));
            }
        }
        if metric_name.is_empty() {
            eprintln!("WARN: ligne #{lineno} : métrique sans __name__ — ignorée");
            skipped += 1;
            continue;
        }
        series_count += 1;

        for (ts, val) in parsed.timestamps.iter().zip(parsed.values.iter()) {
            if let Some(limit) = args.limit {
                if sample_count >= limit {
                    eprintln!("INFO: limite {limit} atteinte, arrêt");
                    return finalize(start, series_count, sample_count, skipped, store);
                }
            }
            sample_count += 1;
            if let Some(w) = &writer {
                let s = Sample {
                    metric: metric_name.clone(),
                    labels: labels.clone(),
                    ts_ms: *ts,
                    value: *val,
                };
                w.blocking_write(s).map_err(|e| {
                    anyhow::anyhow!("blocking_write a échoué (sample #{sample_count}): {e:?}")
                })?;
            }
            if last_log.elapsed() >= Duration::from_secs(5) {
                let elapsed_s = start.elapsed().as_secs_f64();
                let rate = sample_count as f64 / elapsed_s.max(0.001);
                eprintln!(
                    "PROGRESS: {} séries, {} samples ({:.0}/s, {:.0}s écoulé)",
                    series_count, sample_count, rate, elapsed_s,
                );
                last_log = Instant::now();
            }
        }
    }

    finalize(start, series_count, sample_count, skipped, store)
}

fn finalize(
    start: Instant,
    series_count: usize,
    sample_count: usize,
    skipped: usize,
    store: Option<MetricsStore>,
) -> Result<()> {
    let elapsed = start.elapsed();
    let rate = sample_count as f64 / elapsed.as_secs_f64().max(0.001);
    eprintln!(
        "LECTURE TERMINÉE: {} séries, {} samples, {} lignes ignorées en {:.1}s ({:.0} samples/s)",
        series_count,
        sample_count,
        skipped,
        elapsed.as_secs_f64(),
        rate
    );
    if let Some(s) = store {
        eprintln!("ATTENTE DRAIN: 8s pour que le writer commit le dernier batch...");
        std::thread::sleep(Duration::from_secs(8));
        drop(s);
        eprintln!("BASE REDB FERMÉE PROPREMENT");
    } else {
        eprintln!("DRY-RUN: aucune écriture effectuée");
    }
    Ok(())
}
