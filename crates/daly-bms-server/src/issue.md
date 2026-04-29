error[E0308]: mismatched types
   --> crates/daly-bms-server/src/main.rs:285:21
    |
279 | /             tasmota::run_tasmota_mqtt_loop(
280 | |                 devs_ta,
281 | |                 mqtt_ta,
282 | |                 move |snap| {
...   |
285 | |                     async move { s.on_tasmota_snapshot(snap).await }
    | |                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ expected `()`, found `async` block
...   |
288 | |             .await;
    | |                  -- help: consider using a semicolon here
    | |__________________|
    |                    expected this to be `()`
    |
    = note:  expected unit type `()`
            found `async` block `{async block@crates/daly-bms-server/src/main.rs:285:21: 285:31}`

error[E0308]: mismatched types
   --> crates/daly-bms-server/src/main.rs:310:21
    |
303 | /             shelly::run_shelly_mqtt_loop(
304 | |                 devs_sh,
305 | |                 mqtt_sh,
306 | |                 client_sh,
...   |
310 | |                     async move { s.on_shelly_snapshot(snap).await }
    | |                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ expected `()`, found `async` block
...   |
313 | |             .await;
    | |                  -- help: consider using a semicolon here
    | |__________________|
    |                    expected this to be `()`
    |
    = note:  expected unit type `()`
            found `async` block `{async block@crates/daly-bms-server/src/main.rs:310:21: 310:31}`

error[E0308]: mismatched types
   --> crates/daly-bms-server/src/main.rs:395:37
    |
388 | / ...                   et112::run_et112_poll_loop(
389 | | ...                       bus_et,
390 | | ...                       et112_cfg.devices,
391 | | ...                       std::time::Duration::from_millis(et112_cfg.poll_interval_ms),
...   |
395 | | ...                           async move { s.on_et112_snapshot(snap).await }
    | |                               ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ expected `()`, found `async` block
...   |
409 | | ...                   .await;
    | |                            -- help: consider using a semicolon here
    | |____________________________|
    |                              expected this to be `()`
    |
    = note:  expected unit type `()`
            found `async` block `{async block@crates/daly-bms-server/src/main.rs:395:37: 395:47}`

error[E0308]: mismatched types
   --> crates/daly-bms-server/src/main.rs:401:37
    |
388 | /  ...                   et112::run_et112_poll_loop(
389 | |  ...                       bus_et,
390 | |  ...                       et112_cfg.devices,
391 | |  ...                       std::time::Duration::from_millis(et112_cfg.poll_interval_ms),
...   |
401 | |/ ...                           async move {
402 | || ...                               match res {
403 | || ...                                   Ok(()) => s.record_rs485_success(addr, "ET112", &name).await,
404 | || ...                                   Err(msg) => s.record_rs485_error(addr, "ET112", &name, &msg).await,
405 | || ...                               }
406 | || ...                           }
    | ||_______________________________^ expected `()`, found `async` block
...   |
409 | |  ...                   .await;
    | |                             -- help: consider using a semicolon here
    | |_____________________________|
    |                               expected this to be `()`
    |
    = note:  expected unit type `()`
            found `async` block `{async block@crates/daly-bms-server/src/main.rs:401:37: 401:47}`

error[E0308]: mismatched types
   --> crates/daly-bms-server/src/main.rs:429:37
    |
423 | / ...                   irradiance::run_irradiance_poll_loop(
424 | | ...                       bus_irrad,
425 | | ...                       irrad_cfg,
426 | | ...                       move |snap| {
...   |
429 | | ...                           async move { s.on_irradiance_snapshot(snap).await }
    | |                               ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ expected `()`, found `async` block
...   |
443 | | ...                   .await;
    | |                            -- help: consider using a semicolon here
    | |____________________________|
    |                              expected this to be `()`
    |
    = note:  expected unit type `()`
            found `async` block `{async block@crates/daly-bms-server/src/main.rs:429:37: 429:47}`

error[E0308]: mismatched types
   --> crates/daly-bms-server/src/main.rs:435:37
    |
423 | /  ...                   irradiance::run_irradiance_poll_loop(
424 | |  ...                       bus_irrad,
425 | |  ...                       irrad_cfg,
426 | |  ...                       move |snap| {
...   |
435 | |/ ...                           async move {
436 | || ...                               match res {
437 | || ...                                   Ok(()) => s.record_rs485_success(addr, "PRALRAN", &name).await,
438 | || ...                                   Err(msg) => s.record_rs485_error(addr, "PRALRAN", &name, &msg).await,
439 | || ...                               }
440 | || ...                           }
    | ||_______________________________^ expected `()`, found `async` block
...   |
443 | |  ...                   .await;
    | |                             -- help: consider using a semicolon here
    | |_____________________________|
    |                               expected this to be `()`
    |
    = note:  expected unit type `()`
            found `async` block `{async block@crates/daly-bms-server/src/main.rs:435:37: 435:47}`

error[E0308]: mismatched types
   --> crates/daly-bms-server/src/main.rs:466:41
    |
460 | / ...                   ats::run_ats_poll_loop(
461 | | ...                       bus_ats,
462 | | ...                       ats_cfg,
463 | | ...                       move |snap| {
...   |
466 | | ...                           async move { s.on_ats_snapshot(snap).await }
    | |                               ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ expected `()`, found `async` block
...   |
480 | | ...                   .await;
    | |                            -- help: consider using a semicolon here
    | |____________________________|
    |                              expected this to be `()`
    |
    = note:  expected unit type `()`
            found `async` block `{async block@crates/daly-bms-server/src/main.rs:466:41: 466:51}`

error[E0308]: mismatched types
   --> crates/daly-bms-server/src/main.rs:472:41
    |
460 | /  ...                   ats::run_ats_poll_loop(
461 | |  ...                       bus_ats,
462 | |  ...                       ats_cfg,
463 | |  ...                       move |snap| {
...   |
472 | |/ ...                           async move {
473 | || ...                               match res {
474 | || ...                                   Ok(()) => s.record_rs485_success(addr, "ATS", &name).await,
475 | || ...                                   Err(msg) => s.record_rs485_error(addr, "ATS", &name, &msg).await,
476 | || ...                               }
477 | || ...                           }
    | ||_______________________________^ expected `()`, found `async` block
...   |
480 | |  ...                   .await;
    | |                             -- help: consider using a semicolon here
    | |_____________________________|
    |                               expected this to be `()`
    |
    = note:  expected unit type `()`
            found `async` block `{async block@crates/daly-bms-server/src/main.rs:472:41: 472:51}`

error[E0308]: mismatched types
   --> crates/daly-bms-server/src/main.rs:544:37
    |
536 | /  ...                   poll_loop(
537 | |  ...                       manager,
538 | |  ...                       poll_cfg,
539 | |  ...                       move |snap| {
...   |
544 | |/ ...                           async move {
545 | || ...                               s.record_rs485_success(addr, "BMS", &name).await;
546 | || ...                               s.on_snapshot(snap).await;
547 | || ...                           }
    | ||_______________________________^ expected `()`, found `async` block
...   |
568 | |  ...                   .await;
    | |                             -- help: consider using a semicolon here
    | |_____________________________|
    |                               expected this to be `()`
    |
    = note:  expected unit type `()`
            found `async` block `{async block@crates/daly-bms-server/src/main.rs:544:37: 544:47}`

error[E0308]: mismatched types
   --> crates/daly-bms-server/src/main.rs:563:37
    |
536 | /  ...                   poll_loop(
537 | |  ...                       manager,
538 | |  ...                       poll_cfg,
539 | |  ...                       move |snap| {
...   |
563 | |/ ...                           async move {
564 | || ...                               s.record_rs485_error(addr, "BMS", &name, &err_msg).await;
565 | || ...                           }
    | ||_______________________________^ expected `()`, found `async` block
...   |
568 | |  ...                   .await;
    | |                             -- help: consider using a semicolon here
    | |_____________________________|
    |                               expected this to be `()`
    |
    = note:  expected unit type `()`
            found `async` block `{async block@crates/daly-bms-server/src/main.rs:563:37: 563:47}`

For more information about this error, try `rustc --explain E0308`.
error: could not compile `daly-bms-server` (bin "daly-bms-server") due to 10 previous errors
make: *** [Makefile:69: build-arm] Error 101
[!!] make build-arm a échoué
