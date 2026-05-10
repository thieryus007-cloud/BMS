//! Module ATS CHINT — Commutateur automatique de source (NXZB/NXZBN).
//!
//! Polling Modbus RTU sur bus RS485 dédié (parité Even, séparé du bus BMS).
//!
//! ## Architecture
//!
//! ```text
//! ATS CHINT (/dev/ttyUSB2, 9600-8E1, addr=6)
//!   ↓ Modbus RTU FC=03/FC=06 (rs485_bus SharedBus Even parity)
//! daly-bms-server — ats::run_ats_poll_loop
//!   ├── AppState::on_ats_snapshot() → AtsSnapshot
//!   ├── API REST : GET  /api/v1/ats/status
//!   │              POST /api/v1/ats/remote_on|off
//!   │              POST /api/v1/ats/force_source1|source2|double
//!   ├── MQTT    : santuario/switch/1/venus (retain=true)
//!   └── Dashboard SSR : /dashboard/ats
//!
//! santuario/switch/1/venus → broker NanoPi
//!   ↓ dbus-mqtt-venus — SwitchManager
//! com.victronenergy.switch.mqtt_1 (device_instance=60)
//!   /Position  0=AC1/Réseau  1=AC2/Onduleur
//!   /State     0=inactive  1=active  2=alerted
//! ```

pub mod types;
pub mod poll;

pub use types::{AtsCommand, AtsSnapshot};
pub use poll::{execute_ats_command, run_ats_poll_loop};
