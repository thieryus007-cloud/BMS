error[E0599]: no method named `with_worker_threads` found for struct `AsyncStorageBuilder` in the current scope
  --> crates/daly-bms-server/src/tsink_db.rs:60:14
   |
53 |           let storage = AsyncStorageBuilder::new()
   |  _______________________-
54 | |             .with_data_path(&config.data_path)
55 | |             .with_timestamp_precision(TimestampPrecision::Milliseconds)
56 | |             .with_retention(Duration::from_secs(config.retention_days * 24 * 3600))
...  |
60 | |             .with_worker_threads(2)                     // Limite les threads internes
   | |             -^^^^^^^^^^^^^^^^^^^ method not found in `AsyncStorageBuilder`
   | |_____________|
   |

For more information about this error, try `rustc --explain E0599`.
error: could not compile `daly-bms-server` (bin "daly-bms-server") due to 1 previous error
make: *** [Makefile:69: build-arm] Error 101
[!!] make build-arm a échoué
