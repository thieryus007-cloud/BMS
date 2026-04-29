Compiling daly-bms-server v0.1.0 (/home/pi5compute/Daly-BMS-Rust/crates/daly-bms-server)
error[E0599]: no method named `with_flush_interval` found for struct `AsyncStorageBuilder` in the current scope
  --> crates/daly-bms-server/src/tsink_db.rs:61:14
   |
54 |           let storage = AsyncStorageBuilder::new()
   |  _______________________-
55 | |             .with_data_path(&config.data_path)
56 | |             .with_timestamp_precision(TimestampPrecision::Milliseconds)
57 | |             .with_retention(Duration::from_secs(config.retention_days * 24 * 3600))
...  |
61 | |             .with_flush_interval(Duration::from_secs(10))      // Flush périodique (10s)
   | |             -^^^^^^^^^^^^^^^^^^^ method not found in `AsyncStorageBuilder`
   | |_____________|
   |

For more information about this error, try `rustc --explain E0599`.
error: could not compile `daly-bms-server` (bin "daly-bms-server") due to 1 previous error
make: *** [Makefile:69: build-arm] Error 101
[!!] make build-arm a échoué
