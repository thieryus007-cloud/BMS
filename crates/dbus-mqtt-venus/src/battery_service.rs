//! Service D-Bus `com.victronenergy.battery.{name}` pour un BMS Daly.
//!
//! ## Architecture Venus OS
//!
//! Venus OS attend que chaque batterie soit enregistrée en tant que service D-Bus
//! avec le nom `com.victronenergy.battery.{suffix}`.
//!
//! Chaque métrique est un **objet D-Bus** distinct à un chemin tel que `/Soc`,
//! `/Dc/0/Voltage`, etc. Chaque objet implémente l'interface
//! `com.victronenergy.BusItem` exposant `GetValue()`, `GetText()`, `SetValue()`.
//!
//! Le signal `ItemsChanged(a{sa{sv}})` est émis à chaque mise à jour sur l'objet
//! racine `/`, et `GetItems() → a{sa{sv}}` permet à systemcalc-py de tout lire
//! d'un coup au démarrage.

use crate::types::VenusPayload;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::{debug, info, warn};
use zbus::{connection, object_server::SignalContext, Connection};
use zvariant::{OwnedValue, Str};

// =============================================================================
// Constantes
// =============================================================================

const VICTRON_BATTERY_PREFIX: &str = "com.victronenergy.battery";

// =============================================================================
// Item D-Bus — une paire (valeur, texte) pour un path
// =============================================================================

/// Un item D-Bus Venus OS : valeur typée + représentation texte.
#[derive(Debug, Clone)]
pub struct DbusItem {
    /// Valeur serde_json (f64 / int / string)
    pub value: serde_json::Value,
    /// Représentation texte affichée dans Venus OS
    pub text: String,
}

impl DbusItem {
    pub fn f64(v: f64, unit: &str) -> Self {
        Self { value: serde_json::Value::from(v), text: format!("{:.2} {}", v, unit) }
    }
    pub fn f64_prec(v: f64, prec: usize, unit: &str) -> Self {
        Self {
            value: serde_json::Value::from(v),
            text: format!("{:.prec$} {}", v, unit, prec = prec),
        }
    }
    pub fn i32(v: i32) -> Self {
        Self { value: serde_json::Value::from(v), text: v.to_string() }
    }
    pub fn i64(v: i64) -> Self {
        Self { value: serde_json::Value::from(v), text: v.to_string() }
    }
    pub fn str(v: &str) -> Self {
        Self { value: serde_json::Value::from(v), text: v.to_string() }
    }
    pub fn u32(v: u32) -> Self {
        Self { value: serde_json::Value::from(v), text: v.to_string() }
    }
}

/// Convertit un `DbusItem` en `OwnedValue` D-Bus natif.
///
/// Mapping de types :
/// - serde_json f64 → D-Bus `double` (d)
/// - serde_json u64 → D-Bus `uint32`  (u)
/// - serde_json i64 (petit) → D-Bus `int32`  (i)
/// - serde_json i64 (grand) → D-Bus `int64`  (x)
/// - serde_json string     → D-Bus `string`  (s)
fn json_to_owned(v: &serde_json::Value) -> OwnedValue {
    match v {
        serde_json::Value::Number(n) => {
            if n.is_f64() {
                OwnedValue::from(n.as_f64().unwrap_or(0.0))
            } else if n.is_u64() {
                OwnedValue::from(n.as_u64().unwrap_or(0) as u32)
            } else {
                let i = n.as_i64().unwrap_or(0);
                if i >= i32::MIN as i64 && i <= i32::MAX as i64 {
                    OwnedValue::from(i as i32)
                } else {
                    OwnedValue::from(i)
                }
            }
        }
        serde_json::Value::String(s) => OwnedValue::from(Str::from(s.clone())),
        _ => OwnedValue::from(0i32),
    }
}

/// Construit le dict interne `{"Value": variant, "Text": variant}` pour un item.
fn item_to_inner(item: &DbusItem) -> HashMap<String, OwnedValue> {
    let mut d = HashMap::new();
    d.insert("Value".to_string(), json_to_owned(&item.value));
    d.insert("Text".to_string(), OwnedValue::from(Str::from(item.text.clone())));
    d
}

/// Type alias pour la signature D-Bus `a{sa{sv}}`.
type ItemsDict = HashMap<String, HashMap<String, OwnedValue>>;

// =============================================================================
// État partagé d'un service batterie
// =============================================================================

/// Valeurs courantes exposées sur D-Bus pour un BMS.
#[derive(Debug, Clone)]
pub struct BatteryValues {
    pub connected: i32,
    pub soc: f64,
    pub voltage: f64,
    pub current: f64,
    pub power: f64,
    pub temperature: f64,
    pub installed_capacity: f64,
    pub consumed_amphours: f64,
    pub capacity: f64,
    pub time_to_go: i64,
    pub balancing: i32,
    pub system_switch: i32,
    pub allow_to_charge: i32,
    pub allow_to_discharge: i32,
    // Alarmes
    pub alarm_low_voltage: i32,
    pub alarm_high_voltage: i32,
    pub alarm_low_soc: i32,
    pub alarm_high_temp: i32,
    pub alarm_low_temp: i32,
    pub alarm_cell_imbalance: i32,
    // System
    pub min_cell_voltage: f64,
    pub max_cell_voltage: f64,
    pub min_cell_temperature: f64,
    pub max_cell_temperature: f64,
    // Metadata
    pub product_name: String,
    pub firmware_version: String,
    pub device_instance: u32,
    /// Timestamp de la dernière mise à jour (watchdog)
    pub last_update: Instant,
}

impl BatteryValues {
    pub fn disconnected(device_instance: u32, product_name: String) -> Self {
        Self {
            connected: 0,
            soc: 0.0,
            voltage: 0.0,
            current: 0.0,
            power: 0.0,
            temperature: 25.0,
            installed_capacity: 0.0,
            consumed_amphours: 0.0,
            capacity: 0.0,
            time_to_go: 0,
            balancing: 0,
            system_switch: 1,
            allow_to_charge: 1,
            allow_to_discharge: 1,
            alarm_low_voltage: 0,
            alarm_high_voltage: 0,
            alarm_low_soc: 0,
            alarm_high_temp: 0,
            alarm_low_temp: 0,
            alarm_cell_imbalance: 0,
            min_cell_voltage: 0.0,
            max_cell_voltage: 0.0,
            min_cell_temperature: 0.0,
            max_cell_temperature: 0.0,
            product_name,
            firmware_version: "unknown".to_string(),
            device_instance,
            last_update: Instant::now(),
        }
    }

    pub fn from_payload(payload: &VenusPayload, device_instance: u32, product_name: String) -> Self {
        Self {
            connected: 1,
            soc: payload.soc,
            voltage: payload.dc.voltage,
            current: payload.dc.current,
            power: payload.dc.power,
            temperature: payload.dc.temperature,
            installed_capacity: payload.installed_capacity,
            consumed_amphours: payload.consumed_amphours,
            capacity: payload.capacity,
            time_to_go: payload.time_to_go,
            balancing: payload.balancing,
            system_switch: payload.system_switch,
            allow_to_charge: payload.io.allow_to_charge,
            allow_to_discharge: payload.io.allow_to_discharge,
            alarm_low_voltage: payload.alarms.low_voltage,
            alarm_high_voltage: payload.alarms.high_voltage,
            alarm_low_soc: payload.alarms.low_soc,
            alarm_high_temp: payload.alarms.high_temperature,
            alarm_low_temp: payload.alarms.low_temperature,
            alarm_cell_imbalance: payload.alarms.cell_imbalance,
            min_cell_voltage: payload.system.min_cell_voltage,
            max_cell_voltage: payload.system.max_cell_voltage,
            min_cell_temperature: payload.system.min_cell_temperature,
            max_cell_temperature: payload.system.max_cell_temperature,
            product_name,
            firmware_version: "Daly-RS485".to_string(),
            device_instance,
            last_update: Instant::now(),
        }
    }

    /// Construit le dictionnaire complet des items.
    ///
    /// Format : `{"/Soc" → DbusItem{value: 56.4, text: "56.4 %"}, ...}`
    pub fn to_items(&self) -> HashMap<String, DbusItem> {
        let mut m = HashMap::new();

        // Identification
        m.insert("/Mgmt/ProcessName".into(), DbusItem::str("dbus-mqtt-venus"));
        m.insert("/Mgmt/ProcessVersion".into(), DbusItem::str(env!("CARGO_PKG_VERSION")));
        m.insert("/Mgmt/Connection".into(), DbusItem::str("MQTT"));
        m.insert("/ProductId".into(), DbusItem::u32(0));
        m.insert("/ProductName".into(), DbusItem::str(&self.product_name));
        m.insert("/FirmwareVersion".into(), DbusItem::str(&self.firmware_version));
        m.insert("/DeviceInstance".into(), DbusItem::u32(self.device_instance));
        m.insert("/Connected".into(), DbusItem::i32(self.connected));

        // DC measurements
        m.insert("/Dc/0/Voltage".into(), DbusItem::f64(self.voltage, "V"));
        m.insert("/Dc/0/Current".into(), DbusItem::f64(self.current, "A"));
        m.insert("/Dc/0/Power".into(), DbusItem::f64_prec(self.power, 0, "W"));
        m.insert("/Dc/0/Temperature".into(), DbusItem::f64(self.temperature, "°C"));

        // SOC / Capacité
        m.insert("/Soc".into(), DbusItem::f64(self.soc, "%"));
        m.insert("/Capacity".into(), DbusItem::f64(self.capacity, "Ah"));
        m.insert("/InstalledCapacity".into(), DbusItem::f64(self.installed_capacity, "Ah"));
        m.insert("/ConsumedAmphours".into(), DbusItem::f64(self.consumed_amphours, "Ah"));
        m.insert("/TimeToGo".into(), DbusItem::i64(self.time_to_go));

        // Contrôle
        m.insert("/Balancing".into(), DbusItem::i32(self.balancing));
        m.insert("/SystemSwitch".into(), DbusItem::i32(self.system_switch));

        // DVCC
        m.insert("/Io/AllowToCharge".into(), DbusItem::i32(self.allow_to_charge));
        m.insert("/Io/AllowToDischarge".into(), DbusItem::i32(self.allow_to_discharge));

        // Alarmes
        m.insert("/Alarms/LowVoltage".into(), DbusItem::i32(self.alarm_low_voltage));
        m.insert("/Alarms/HighVoltage".into(), DbusItem::i32(self.alarm_high_voltage));
        m.insert("/Alarms/LowSoc".into(), DbusItem::i32(self.alarm_low_soc));
        m.insert("/Alarms/HighTemperature".into(), DbusItem::i32(self.alarm_high_temp));
        m.insert("/Alarms/LowTemperature".into(), DbusItem::i32(self.alarm_low_temp));
        m.insert("/Alarms/CellImbalance".into(), DbusItem::i32(self.alarm_cell_imbalance));

        // Info système
        m.insert("/System/MinCellVoltage".into(), DbusItem::f64(self.min_cell_voltage, "V"));
        m.insert("/System/MaxCellVoltage".into(), DbusItem::f64(self.max_cell_voltage, "V"));
        m.insert("/System/MinCellTemperature".into(), DbusItem::f64(self.min_cell_temperature, "°C"));
        m.insert("/System/MaxCellTemperature".into(), DbusItem::f64(self.max_cell_temperature, "°C"));

        m
    }
}

// =============================================================================
// Interface D-Bus — objet racine `/`
// =============================================================================

/// Objet D-Bus racine `/` : implémente `com.victronenergy.BusItem`.
///
/// Venus OS systemcalc-py appelle `GetItems()` au démarrage pour lire toutes
/// les valeurs, puis écoute le signal `ItemsChanged` pour les mises à jour.
struct BatteryRootIface {
    values: Arc<Mutex<BatteryValues>>,
}

#[zbus::interface(name = "com.victronenergy.BusItem")]
impl BatteryRootIface {
    /// Retourne tous les chemins avec leurs valeurs et textes.
    /// Type de retour D-Bus : `a{sa{sv}}`
    fn get_items(&self) -> ItemsDict {
        let guard = self.values.lock().unwrap();
        guard
            .to_items()
            .iter()
            .map(|(path, item)| (path.clone(), item_to_inner(item)))
            .collect()
    }

    /// GetValue sur l'objet racine — retourne 0 (pas une feuille).
    fn get_value(&self) -> OwnedValue {
        OwnedValue::from(0i32)
    }

    fn get_text(&self) -> String {
        String::new()
    }

    /// SetValue — lecture seule.
    fn set_value(&self, _val: zvariant::Value<'_>) -> i32 {
        1
    }

    /// Signal émis à chaque mise à jour des valeurs.
    /// Type D-Bus : `a{sa{sv}}` — dict<path, dict<"Value"|"Text", variant>>
    #[zbus(signal)]
    async fn items_changed(
        ctx: &SignalContext<'_>,
        items: ItemsDict,
    ) -> zbus::Result<()>;
}

// =============================================================================
// Interface D-Bus — objet feuille (chemin individuel)
// =============================================================================

/// Objet D-Bus pour un chemin individuel (ex: `/Soc`, `/Dc/0/Voltage`).
struct BusItemLeaf {
    path: String,
    values: Arc<Mutex<BatteryValues>>,
}

#[zbus::interface(name = "com.victronenergy.BusItem")]
impl BusItemLeaf {
    fn get_value(&self) -> OwnedValue {
        let guard = self.values.lock().unwrap();
        match guard.to_items().get(&self.path) {
            Some(item) => json_to_owned(&item.value),
            None => OwnedValue::from(0i32),
        }
    }

    fn get_text(&self) -> String {
        let guard = self.values.lock().unwrap();
        guard
            .to_items()
            .get(&self.path)
            .map(|i| i.text.clone())
            .unwrap_or_default()
    }

    fn set_value(&self, _val: zvariant::Value<'_>) -> i32 {
        1 // lecture seule
    }
}

// =============================================================================
// Handle vers un service actif
// =============================================================================

/// Référence à un service D-Bus batterie actif.
///
/// Utilisé par `BatteryManager` pour mettre à jour les valeurs.
pub struct BatteryServiceHandle {
    pub service_name: String,
    pub device_instance: u32,
    pub values: Arc<Mutex<BatteryValues>>,
    /// Connexion D-Bus — maintient le service vivant tant qu'elle existe.
    connection: Connection,
}

impl BatteryServiceHandle {
    /// Met à jour les valeurs et émet `ItemsChanged` sur D-Bus.
    pub async fn update(&self, payload: &VenusPayload, product_name: &str) -> Result<()> {
        let new_values = BatteryValues::from_payload(
            payload,
            self.device_instance,
            product_name.to_string(),
        );
        let items = new_values.to_items();

        {
            let mut guard = self.values.lock().unwrap();
            *guard = new_values;
        }

        self.emit_items_changed(&items).await?;

        debug!(
            service = %self.service_name,
            soc = %payload.soc,
            voltage = %payload.dc.voltage,
            "D-Bus ItemsChanged émis"
        );

        Ok(())
    }

    /// Marque le service comme déconnecté (timeout watchdog).
    pub async fn set_disconnected(&self) -> Result<()> {
        let items = {
            let mut guard = self.values.lock().unwrap();
            guard.connected = 0;
            guard.to_items()
        };
        warn!(service = %self.service_name, "BMS déconnecté — watchdog timeout");
        self.emit_items_changed(&items).await
    }

    /// Republication forcée depuis les valeurs courantes (keepalive Venus OS).
    pub async fn republish(&self) -> Result<()> {
        let items = {
            let guard = self.values.lock().unwrap();
            guard.to_items()
        };
        self.emit_items_changed(&items).await
    }

    /// Émet le signal `ItemsChanged(a{sa{sv}})` sur l'objet racine `/`.
    ///
    /// Format :
    /// ```text
    /// {
    ///   "/Soc":          {"Value": <56.4f64>, "Text": <"56.4 %">},
    ///   "/Dc/0/Voltage": {"Value": <48.1f64>, "Text": <"48.10 V">},
    ///   ...
    /// }
    /// ```
    async fn emit_items_changed(&self, items: &HashMap<String, DbusItem>) -> Result<()> {
        let dict: ItemsDict = items
            .iter()
            .map(|(path, item)| (path.clone(), item_to_inner(item)))
            .collect();

        let ctx = SignalContext::new(&self.connection, "/")?;

        match BatteryRootIface::items_changed(&ctx, dict).await {
            Ok(_) => {
                debug!(
                    service = %self.service_name,
                    count = items.len(),
                    "ItemsChanged(a{{sa{{sv}}}}) émis"
                );
                Ok(())
            }
            Err(e) => {
                // Non fatal — le service reste actif même si le signal échoue
                warn!(service = %self.service_name, "ItemsChanged warning : {}", e);
                Ok(())
            }
        }
    }
}

// =============================================================================
// Création du service D-Bus
// =============================================================================

/// Crée et enregistre un service D-Bus `com.victronenergy.battery.{suffix}`.
///
/// Enregistre :
/// - L'objet racine `/` avec `GetItems()` et le signal `ItemsChanged`
/// - Un objet feuille par chemin métrique (`/Soc`, `/Dc/0/Voltage`, etc.)
///
/// La connexion D-Bus (et donc le service) reste active tant que le
/// `BatteryServiceHandle` retourné est en vie.
pub async fn create_battery_service(
    dbus_bus: &str,
    service_suffix: &str,
    device_instance: u32,
    product_name: String,
) -> Result<BatteryServiceHandle> {
    let service_name = format!("{}.{}", VICTRON_BATTERY_PREFIX, service_suffix);

    info!(
        service = %service_name,
        device_instance = device_instance,
        "Enregistrement service D-Bus Venus OS"
    );

    let initial_values = Arc::new(Mutex::new(BatteryValues::disconnected(
        device_instance,
        product_name.clone(),
    )));

    // Objet racine `/`
    let root = BatteryRootIface { values: initial_values.clone() };

    // Construire la connexion D-Bus avec le nom de service et l'objet racine
    let builder = match dbus_bus {
        "session" => connection::Builder::session()?,
        _ => connection::Builder::system()?,
    };

    let conn = builder
        .name(service_name.as_str())?
        .serve_at("/", root)?
        .build()
        .await?;

    // Enregistrer un objet feuille par chemin métrique
    let leaf_paths: Vec<String> = {
        let guard = initial_values.lock().unwrap();
        guard.to_items().into_keys().collect()
    };

    for path in &leaf_paths {
        let leaf = BusItemLeaf {
            path: path.clone(),
            values: initial_values.clone(),
        };
        conn.object_server().at(path.as_str(), leaf).await?;
    }

    info!(
        service = %service_name,
        paths = leaf_paths.len(),
        "Service D-Bus enregistré ({} chemins + racine /)",
        leaf_paths.len()
    );

    Ok(BatteryServiceHandle {
        service_name,
        device_instance,
        values: initial_values,
        connection: conn,
    })
}
