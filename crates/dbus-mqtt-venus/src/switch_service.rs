//! Service D-Bus `com.victronenergy.switch.{name}` pour commutateurs.
//!
//! Conforme au wiki Victron Venus OS — section Switch :
//! <https://github.com/victronenergy/venus/wiki/dbus#switch>
//!
//! ## Chemins D-Bus exposés (tous les switches)
//!
//! ```text
//! /Position          — 0=AC1 (réseau), 1=AC2 (générateur/onduleur)
//! /State             — 0x100=Connected (état module global)
//! /Connected         — 0 ou 1
//! /ProductName
//! /ProductId
//! /DeviceInstance
//! /Mgmt/ProcessName
//! /Mgmt/ProcessVersion
//! /Mgmt/Connection
//! ```
//!
//! ## Chemins SwitchableOutput (uniquement si `controllable = true`)
//!
//! Exposés quand `command_topic` est défini dans la config.
//! La console Venus OS détecte ces chemins et affiche un bouton ON/OFF.
//!
//! ```text
//! /SwitchableOutput/0/State              — RW  0=Off, 1=On
//! /SwitchableOutput/0/Status             — R   0x00=Off, 0x09=On
//! /SwitchableOutput/0/Name               — R   nom du canal
//! /SwitchableOutput/0/Settings/Type      — R   1=toggle
//! /SwitchableOutput/0/Settings/CustomName — RW  nom affiché
//! /SwitchableOutput/0/Settings/Group      — RW  groupe d'affichage
//! /SwitchableOutput/0/Settings/ShowUIControl — RW 1=visible
//! ```
//!
//! Lorsque l'utilisateur bascule le switch depuis la console Venus OS,
//! `set_value()` est appelé sur `/SwitchableOutput/0/State`.
//! La valeur (0=Off, 1=On) est transmise via `cmd_tx` au `SwitchManager`,
//! qui publie ensuite `"OFF"` ou `"ON"` sur le topic MQTT `command_topic`.

use crate::types::SwitchPayload;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use zbus::{connection, object_server::SignalContext, Connection};
use zvariant::{OwnedValue, Str};

// =============================================================================
// Constante
// =============================================================================

const VICTRON_SWITCH_PREFIX: &str = "com.victronenergy.switch";

// Path du canal SwitchableOutput contrôlable
const PATH_SW_STATE: &str = "/SwitchableOutput/0/State";

// Statut SwitchableOutput (wiki Victron)
const STATUS_ON:  i32 = 0x09; // Output fault (0x08) OR-ed avec Active (0x01)
const STATUS_OFF: i32 = 0x00;

// =============================================================================
// Item D-Bus
// =============================================================================

#[derive(Debug, Clone)]
pub struct DbusItem {
    pub value: serde_json::Value,
    pub text:  String,
}

impl DbusItem {
    pub fn i32(v: i32) -> Self {
        Self { value: serde_json::Value::from(v), text: v.to_string() }
    }
    pub fn str(v: &str) -> Self {
        Self { value: serde_json::Value::from(v), text: v.to_string() }
    }
    pub fn u32(v: u32) -> Self {
        Self { value: serde_json::Value::from(v), text: v.to_string() }
    }
}

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

fn item_to_inner(item: &DbusItem) -> HashMap<String, OwnedValue> {
    let mut d = HashMap::new();
    d.insert("Value".to_string(), json_to_owned(&item.value));
    d.insert("Text".to_string(), OwnedValue::from(Str::from(item.text.clone())));
    d
}

type ItemsDict = HashMap<String, HashMap<String, OwnedValue>>;

// =============================================================================
// Valeurs courantes
// =============================================================================

/// État courant d'un switch exposé sur D-Bus.
#[derive(Debug, Clone)]
pub struct SwitchValues {
    pub connected:         i32,
    pub position:          i32,
    /// État ON/OFF pour SwitchableOutput (0=Off, 1=On)
    pub switchable_state:  i32,
    /// true → expose les chemins /SwitchableOutput/0/...
    pub controllable:      bool,
    pub product_name:      String,
    pub custom_name:       String,
    pub group:             String,
    pub device_instance:   u32,
    pub last_update:       Instant,
}

impl SwitchValues {
    pub fn disconnected(
        device_instance: u32,
        product_name:    String,
        custom_name:     String,
        group:           String,
        controllable:    bool,
    ) -> Self {
        Self {
            connected:        0,
            position:         0,
            switchable_state: 0,
            controllable,
            product_name,
            custom_name,
            group,
            device_instance,
            last_update:      Instant::now(),
        }
    }

    pub fn from_payload(
        payload:         &SwitchPayload,
        device_instance: u32,
        product_name:    String,
        custom_name:     String,
        group:           String,
        controllable:    bool,
    ) -> Self {
        // switchable_state : 0=Off, 1=On — direct depuis payload.state (0/1 Tasmota)
        let switchable_state = if payload.state > 0 { 1 } else { 0 };
        Self {
            connected:        1,
            position:         payload.position,
            switchable_state,
            controllable,
            product_name,
            custom_name,
            group,
            device_instance,
            last_update:      Instant::now(),
        }
    }

    pub fn to_items(&self) -> HashMap<String, DbusItem> {
        let mut m = HashMap::new();

        // ── Identification ────────────────────────────────────────────────────
        m.insert("/Mgmt/ProcessName".into(),    DbusItem::str("dbus-mqtt-venus"));
        m.insert("/Mgmt/ProcessVersion".into(), DbusItem::str(env!("CARGO_PKG_VERSION")));
        m.insert("/Mgmt/Connection".into(),     DbusItem::str("MQTT"));
        m.insert("/ProductId".into(),           DbusItem::u32(0));
        m.insert("/ProductName".into(),         DbusItem::str(&self.product_name));
        m.insert("/DeviceInstance".into(),      DbusItem::u32(self.device_instance));
        m.insert("/Connected".into(),           DbusItem::i32(self.connected));
        m.insert("/CustomName".into(),          DbusItem::str(&self.custom_name));
        m.insert("/FirmwareVersion".into(),     DbusItem::str("1"));

        // ── Chemins switch ATS (wiki Victron — switch module state) ──────────
        m.insert("/Position".into(), DbusItem::i32(self.position));
        // /State au sens module : 0x100=Connected, 0=disconnected
        m.insert("/State".into(),    DbusItem::i32(if self.connected == 1 { 0x100 } else { 0 }));

        // ── Chemins SwitchableOutput (console Venus ON/OFF) ──────────────────
        // Exposés uniquement si controllable = true (command_topic configuré)
        if self.controllable {
            let status = if self.switchable_state != 0 { STATUS_ON } else { STATUS_OFF };

            m.insert(PATH_SW_STATE.into(),
                DbusItem::i32(self.switchable_state));
            m.insert("/SwitchableOutput/0/Status".into(),
                DbusItem::i32(status));
            m.insert("/SwitchableOutput/0/Name".into(),
                DbusItem::str(&self.product_name));
            m.insert("/SwitchableOutput/0/Settings/Type".into(),
                DbusItem::i32(1));   // 1 = toggle
            // ValidTypes : champ binaire — bit N = type N supporté.
            // Bit 1 (valeur 2) = toggle. OBLIGATOIRE pour que le GUI
            // affiche le widget de contrôle ON/OFF (sans lui, output = non-contrôlable).
            m.insert("/SwitchableOutput/0/Settings/ValidTypes".into(),
                DbusItem::i32(0b0010));  // 2 = toggle uniquement
            m.insert("/SwitchableOutput/0/Settings/CustomName".into(),
                DbusItem::str(&self.custom_name));
            // Group : omis si vide → Venus tombe en fallback par service D-Bus.
            // Quand non-vide, tous les switches partageant la même valeur
            // sont regroupés sur la même carte dans la console Venus OS.
            if !self.group.is_empty() {
                m.insert("/SwitchableOutput/0/Settings/Group".into(),
                    DbusItem::str(&self.group));
            }
            m.insert("/SwitchableOutput/0/Settings/ShowUIControl".into(),
                DbusItem::i32(1));  // visible dans toutes les UI
        }

        m
    }
}

// =============================================================================
// Interface D-Bus — objet racine `/`
// =============================================================================

struct SwitchRootIface {
    values: Arc<Mutex<SwitchValues>>,
}

#[zbus::interface(name = "com.victronenergy.BusItem")]
impl SwitchRootIface {
    fn get_items(&self) -> ItemsDict {
        let guard = self.values.lock().unwrap();
        guard
            .to_items()
            .iter()
            .map(|(path, item)| (path.clone(), item_to_inner(item)))
            .collect()
    }

    fn get_value(&self) -> OwnedValue { OwnedValue::from(0i32) }
    fn get_text(&self) -> String { String::new() }
    fn set_value(&self, _val: zvariant::Value<'_>) -> i32 { 1 }

    #[zbus(signal)]
    async fn items_changed(
        ctx:   &SignalContext<'_>,
        items: ItemsDict,
    ) -> zbus::Result<()>;
}

// =============================================================================
// Interface D-Bus — objet feuille
// =============================================================================

struct BusItemLeaf {
    path:    String,
    values:  Arc<Mutex<SwitchValues>>,
    /// Émetteur de commande ON/OFF — présent uniquement pour
    /// `/SwitchableOutput/0/State` sur un switch controllable.
    cmd_tx:  Option<mpsc::Sender<i32>>,
}

#[zbus::interface(name = "com.victronenergy.BusItem")]
impl BusItemLeaf {
    fn get_value(&self) -> OwnedValue {
        let guard = self.values.lock().unwrap();
        match guard.to_items().get(&self.path) {
            Some(item) => json_to_owned(&item.value),
            None       => OwnedValue::from(0i32),
        }
    }

    fn get_text(&self) -> String {
        let guard = self.values.lock().unwrap();
        guard.to_items().get(&self.path).map(|i| i.text.clone()).unwrap_or_default()
    }

    /// Traite les écritures D-Bus depuis la console Venus OS.
    ///
    /// Chemins inscriptibles :
    /// - `/SwitchableOutput/0/State`               → commande ON/OFF vers Tasmota
    /// - `/SwitchableOutput/0/Settings/CustomName` → renommage depuis la console (RW per spec)
    /// - `/CustomName`                             → idem (chemin alternatif lu par Venus)
    ///
    /// Retourne 0 (succès) ou 1 (non supporté / chemin en lecture seule).
    fn set_value(&self, val: zvariant::Value<'_>) -> i32 {
        // ── Commande ON/OFF ────────────────────────────────────────────────────
        if self.path == PATH_SW_STATE {
            let Some(tx) = &self.cmd_tx else { return 1 };

            // Extraire la valeur entière (0=Off, 1=On) depuis le variant D-Bus
            let state_val: i32 = match &val {
                zvariant::Value::I32(v)  => *v,
                zvariant::Value::U32(v)  => *v as i32,
                zvariant::Value::I64(v)  => *v as i32,
                zvariant::Value::U64(v)  => *v as i32,
                zvariant::Value::I16(v)  => *v as i32,
                zvariant::Value::U16(v)  => *v as i32,
                zvariant::Value::U8(v)   => *v as i32,
                _ => {
                    warn!(path = %self.path, "set_value : type D-Bus non supporté {:?}", val);
                    return 1;
                }
            };

            // Mise à jour optimiste locale (avant confirmation Tasmota)
            if let Ok(mut guard) = self.values.lock() {
                guard.switchable_state = if state_val != 0 { 1 } else { 0 };
                guard.last_update = Instant::now();
            }

            // Transmettre la commande au SwitchManager → MQTT → Tasmota
            let _ = tx.try_send(state_val);
            debug!(path = %self.path, state = state_val, "Commande switch reçue depuis console Venus");
            return 0; // succès D-Bus
        }

        // ── Renommage CustomName (RW par spec Victron) ─────────────────────────
        if self.path == "/SwitchableOutput/0/Settings/CustomName"
            || self.path == "/CustomName"
        {
            let new_name = match &val {
                zvariant::Value::Str(s) => s.as_str().to_string(),
                _ => return 0, // type inattendu → ignorer silencieusement
            };
            if let Ok(mut g) = self.values.lock() {
                debug!(path = %self.path, name = %new_name, "CustomName mis à jour depuis console Venus");
                g.custom_name = new_name;
            }
            return 0;
        }

        1 // lecture seule pour tous les autres chemins
    }
}

// =============================================================================
// Handle
// =============================================================================

pub struct SwitchServiceHandle {
    pub service_name:    String,
    pub device_instance: u32,
    pub values:          Arc<Mutex<SwitchValues>>,
    connection:          Connection,
    pub product_name:    String,
    /// Récepteur de commandes ON/OFF (0=Off, 1=On) depuis D-Bus.
    /// Pris par le SwitchManager pour lancer la tâche de publication MQTT.
    pub cmd_rx:          Option<mpsc::Receiver<i32>>,
}

impl SwitchServiceHandle {
    pub async fn update(&self, payload: &SwitchPayload) -> Result<()> {
        let controllable  = { self.values.lock().unwrap().controllable };
        let group         = { self.values.lock().unwrap().group.clone() };
        // Lire custom_name depuis le mutex : préserve les renommages effectués
        // depuis la console Venus OS entre deux mises à jour MQTT.
        let custom_name   = { self.values.lock().unwrap().custom_name.clone() };
        let new_values = SwitchValues::from_payload(
            payload,
            self.device_instance,
            self.product_name.clone(),
            custom_name,
            group,
            controllable,
        );
        let items = new_values.to_items();
        { *self.values.lock().unwrap() = new_values; }
        self.emit_items_changed(&items).await?;
        debug!(
            service = %self.service_name,
            position = payload.position,
            state    = payload.state,
            "D-Bus ItemsChanged switch émis"
        );
        Ok(())
    }

    pub async fn set_disconnected(&self) -> Result<()> {
        let items = {
            let mut g = self.values.lock().unwrap();
            g.connected = 0;
            g.to_items()
        };
        warn!(service = %self.service_name, "Switch déconnecté — watchdog timeout");
        self.emit_items_changed(&items).await
    }

    pub async fn republish(&self) -> Result<()> {
        let items = { self.values.lock().unwrap().to_items() };
        self.emit_items_changed(&items).await
    }

    async fn emit_items_changed(&self, items: &HashMap<String, DbusItem>) -> Result<()> {
        let dict: ItemsDict = items
            .iter()
            .map(|(p, i)| (p.clone(), item_to_inner(i)))
            .collect();
        let ctx = SignalContext::new(&self.connection, "/")?;
        match SwitchRootIface::items_changed(&ctx, dict).await {
            Ok(_)  => { debug!(service = %self.service_name, "ItemsChanged switch émis"); Ok(()) }
            Err(e) => { warn!(service = %self.service_name, "ItemsChanged warning : {}", e); Ok(()) }
        }
    }
}

// =============================================================================
// Création du service
// =============================================================================

/// Crée et enregistre un service D-Bus `com.victronenergy.switch.{suffix}`.
///
/// - Si `controllable = true`, expose les chemins `/SwitchableOutput/0/...`
///   et retourne un `cmd_rx` dans le handle pour la publication MQTT.
/// - Si `controllable = false` (ex: ATS CHINT), expose uniquement `/Position`
///   et `/State` (comportement hérité).
pub async fn create_switch_service(
    dbus_bus:        &str,
    service_suffix:  &str,
    device_instance: u32,
    product_name:    String,
    custom_name:     String,
    group:           String,
    controllable:    bool,
) -> Result<SwitchServiceHandle> {
    let service_name = format!("{}.{}", VICTRON_SWITCH_PREFIX, service_suffix);

    info!(
        service      = %service_name,
        device_instance,
        controllable,
        "Enregistrement service D-Bus switch Venus OS"
    );

    let initial_values = Arc::new(Mutex::new(
        SwitchValues::disconnected(
            device_instance,
            product_name.clone(),
            custom_name.clone(),
            group,
            controllable,
        )
    ));

    // Canal de commande ON/OFF (uniquement si controllable)
    let (cmd_tx, cmd_rx): (Option<mpsc::Sender<i32>>, Option<mpsc::Receiver<i32>>) =
        if controllable {
            let (tx, rx) = mpsc::channel(8);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

    let root = SwitchRootIface { values: initial_values.clone() };

    let builder = match dbus_bus {
        "session" => connection::Builder::session()?,
        _         => connection::Builder::system()?,
    };

    let conn = builder
        .name(service_name.as_str())?
        .serve_at("/", root)?
        .build()
        .await?;

    // Enregistrer un objet feuille pour chaque chemin exposé
    let leaf_paths: Vec<String> = {
        initial_values.lock().unwrap().to_items().into_keys().collect()
    };

    for path in &leaf_paths {
        // Le canal de commande n'est transmis qu'à la feuille du chemin inscriptible
        let leaf_cmd_tx = if path == PATH_SW_STATE { cmd_tx.clone() } else { None };

        conn.object_server()
            .at(path.as_str(), BusItemLeaf {
                path:   path.clone(),
                values: initial_values.clone(),
                cmd_tx: leaf_cmd_tx,
            })
            .await?;
    }

    info!(
        service      = %service_name,
        paths        = leaf_paths.len(),
        controllable,
        "Service D-Bus switch enregistré ({} chemins + racine /)",
        leaf_paths.len()
    );

    Ok(SwitchServiceHandle {
        service_name,
        device_instance,
        values:     initial_values,
        connection: conn,
        product_name,
        cmd_rx,
    })
}
