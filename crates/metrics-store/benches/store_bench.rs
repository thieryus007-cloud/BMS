//! Benches Phase 0.8 — calibrer le coût d'ingestion et de range scan
//! sur redb 4.x. À comparer aux mêmes benches sur SQLite (plan v2) si
//! l'option SQLite reste sur la table.
//!
//! Exécution :
//! ```bash
//! cargo bench -p metrics-store
//! ```
//! Les rapports HTML sont générés sous `target/criterion/`.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use redb::{Builder, Database};

use metrics_store::encoding::enc_skey;
use metrics_store::reader::Reader;
use metrics_store::tables::{
    SeriesMeta, TABLE_META, TABLE_RAW, TABLE_SERIES_BY_KEY, TABLE_SERIES_META,
};

fn tmp_db_path(label: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    p.push(format!("metrics-store-bench-{label}-{}-{nanos}.redb", std::process::id()));
    p
}

fn open_empty(path: &PathBuf) -> Arc<Database> {
    let _ = std::fs::remove_file(path);
    let db = Builder::new()
        .set_cache_size(64 * 1024 * 1024)
        .create(path)
        .expect("create");
    // init tables
    let tx = db.begin_write().unwrap();
    let _ = tx.open_table(TABLE_SERIES_BY_KEY).unwrap();
    let _ = tx.open_table(TABLE_SERIES_META).unwrap();
    let _ = tx.open_table(TABLE_META).unwrap();
    let _ = tx.open_table(TABLE_RAW).unwrap();
    tx.commit().unwrap();
    Arc::new(db)
}

/// Insère `n_series * n_points_per_series` points en commits de taille
/// `batch`. Mesure le débit total.
fn bench_insert_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert_batch_raw");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));

    for &(n_series, n_points, batch) in
        &[(10_u32, 1_000_u32, 250_usize), (10, 10_000, 500), (100, 1_000, 500)]
    {
        let total = (n_series as u64) * (n_points as u64);
        group.throughput(Throughput::Elements(total));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!(
                "{n_series}series_{n_points}pts_batch{batch}"
            )),
            &(n_series, n_points, batch),
            |b, &(n_series, _n_points, batch)| {
                b.iter(|| {
                    let path = tmp_db_path("insert");
                    let db = open_empty(&path);
                    // Pré-popule séries + meta
                    {
                        let wtx = db.begin_write().unwrap();
                        {
                            let mut t_skey = wtx.open_table(TABLE_SERIES_BY_KEY).unwrap();
                            let mut t_smeta = wtx.open_table(TABLE_SERIES_META).unwrap();
                            for sid in 0..n_series {
                                let metric = format!("bench_metric_{sid}");
                                let labels_json = format!("{{\"sid\":\"{sid}\"}}");
                                let lookup_key =
                                    metrics_store::encoding::make_lookup_key(&metric, &labels_json);
                                t_skey.insert(&lookup_key[..], sid).unwrap();
                                let meta = SeriesMeta {
                                    metric,
                                    labels_json,
                                    first_seen_ms: 0,
                                    last_seen_ms: 0,
                                };
                                let bytes = bincode::serialize(&meta).unwrap();
                                t_smeta.insert(sid, &bytes[..]).unwrap();
                            }
                        }
                        wtx.commit().unwrap();
                    }

                    // Insertion en batchs
                    let mut written = 0_u64;
                    for chunk in 0..total.div_ceil(batch as u64) {
                        let wtx = db.begin_write().unwrap();
                        {
                            let mut t_raw = wtx.open_table(TABLE_RAW).unwrap();
                            for i in 0..batch as u64 {
                                let global = chunk * batch as u64 + i;
                                if global >= total {
                                    break;
                                }
                                let sid = (global % n_series as u64) as u32;
                                let ts = (global / n_series as u64) as i64 * 1_000;
                                let k = enc_skey(sid, ts);
                                t_raw.insert(&k[..], (global as f64) * 0.001).unwrap();
                                written += 1;
                            }
                        }
                        wtx.commit().unwrap();
                    }
                    assert_eq!(written, total);

                    // Cleanup hors mesure (on accepte la pollution sur les iter)
                    drop(db);
                    let _ = std::fs::remove_file(&path);
                })
            },
        );
    }
    group.finish();
}

/// Mesure un range scan sur une série dense (1 point / s sur 1 jour).
fn bench_range_scan(c: &mut Criterion) {
    let path = tmp_db_path("scan");
    let db = open_empty(&path);
    let series_id = 1_u32;
    let n_points: i64 = 86_400;
    // populate
    {
        let wtx = db.begin_write().unwrap();
        {
            let mut t_skey = wtx.open_table(TABLE_SERIES_BY_KEY).unwrap();
            let mut t_smeta = wtx.open_table(TABLE_SERIES_META).unwrap();
            let mk = metrics_store::encoding::make_lookup_key("scan", "{}");
            t_skey.insert(&mk[..], series_id).unwrap();
            let meta = SeriesMeta {
                metric: "scan".into(),
                labels_json: "{}".into(),
                first_seen_ms: 0,
                last_seen_ms: 0,
            };
            t_smeta.insert(series_id, &bincode::serialize(&meta).unwrap()[..]).unwrap();
        }
        wtx.commit().unwrap();
    }
    let chunk = 5_000_i64;
    let mut written = 0_i64;
    while written < n_points {
        let wtx = db.begin_write().unwrap();
        {
            let mut t_raw = wtx.open_table(TABLE_RAW).unwrap();
            let end = (written + chunk).min(n_points);
            for ts in written..end {
                let k = enc_skey(series_id, ts * 1_000);
                t_raw.insert(&k[..], ts as f64).unwrap();
            }
            written = end;
        }
        wtx.commit().unwrap();
    }

    // S'assure que tous les writers async sont libérés
    thread::sleep(Duration::from_millis(50));
    let _ = Instant::now();

    let reader = Reader::from_db(db.clone());

    let mut group = c.benchmark_group("range_scan_raw");
    group.sample_size(20);
    for &span_sec in &[60_i64, 3_600, 86_400] {
        group.throughput(Throughput::Elements(span_sec as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("span={span_sec}s")),
            &span_sec,
            |b, &span_sec| {
                b.iter(|| {
                    let to_ms = (n_points - 1) * 1_000;
                    let from_ms = to_ms - span_sec * 1_000;
                    let pts = reader.query_range_raw(series_id, from_ms, to_ms).unwrap();
                    criterion::black_box(pts);
                })
            },
        );
    }
    group.finish();

    drop(reader);
    drop(db);
    let _ = std::fs::remove_file(&path);
}

criterion_group!(benches, bench_insert_batch, bench_range_scan);
criterion_main!(benches);
