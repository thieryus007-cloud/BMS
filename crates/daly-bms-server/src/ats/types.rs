//! Types de données pour le commutateur automatique CHINT ATS (NXZB/NXZBN).
//!
//! Protocole : Modbus RTU FC=03 (lecture) / FC=06 (écriture)
//! Parité    : Even (8E1) — BUS RS485 SÉPARÉ du bus BMS (qui est 8N1)
//!
//! ## Registres principaux
//!
//! | Adresse | Nom         | Description                        |
//! |---------|-------------|-----------------------------------|
//! | 0x0006  | v1a         | Tension source 1 phase A (V)       |
//! | 0x0007  | v1b         | Tension source 1 phase B (V)       |
//! | 0x0008  | v1c         | Tension source 1 phase C (V)       |
//! | 0x0009  | v2a         | Tension source 2 phase A (V)       |
//! | 0x000A  | v2b         | Tension source 2 phase B (V)       |
//! | 0x000B  | v2c         | Tension source 2 phase C (V)       |
//! | 0x000C  | sw_version  | Version logicielle (÷100)          |
//! | 0x000D  | freq        | Fréquences (MN : hi=f1, lo=f2)     |
//! | 0x000E  | parity_code | Parité Modbus (0=N, 1=O, 2=E)      |
//! | 0x000F  | max1        | Tension max enregistrée source 1   |
//! | 0x0012  | max2        | Tension max enregistrée source 2   |
//! | 0x0015  | cnt1        | Compteur commutations S1→S2        |
//! | 0x0016  | cnt2        | Compteur commutations S2→S1        |
//! | 0x0017  | runtime_h   | Durée de fonctionnement (h)        |
//! | 0x004F  | pwr_status  | Statut tension (bitfield)          |
//! | 0x0050  | sw_status   | Statut commutation (bitfield)      |
//! | 0x0100  | modbus_addr | Adresse Modbus configurée          |
//! | 0x0101  | modbus_baud | Baud rate Modbus (0=4800..3=38400) |
//! | 0x2065  | uv1         | Seuil sous-tension source 1 (MN)   |
//! | 0x2066  | uv2         | Seuil sous-tension source 2 (MN)   |
//! | 0x2067  | ov1         | Seuil sur-tension source 1 (MN)    |
//! | 0x2068  | ov2         | Seuil sur-tension source 2 (MN)    |
//! | 0x2069  | t1          | Délai commutation (s, MN)          |
//! | 0x206A  | t2          | Délai commutation (s, MN)          |
//! | 0x206B  | t3          | Délai commutation (s, MN)          |
//! | 0x206C  | t4          | Délai commutation (s, MN)          |
//! | 0x206D  | op_mode     | Mode opératoire (MN)               |

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

// =============================================================================
// Statut tension d'une phase
// =============================================================================

/// Statut tension d'une phase (2 bits dans le registre 0x004F).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhaseStatus {
    Normal,
    UnderVoltage,
    OverVoltage,
    Error,
}

impl PhaseStatus {
    pub fn from_bits(bits: u8) -> Self {
        match bits & 0x03 {
            0 => Self::Normal,
            1 => Self::UnderVoltage,
            2 => Self::OverVoltage,
            _ => Self::Error,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Normal       => "Normal",
            Self::UnderVoltage => "Sous-tension",
            Self::OverVoltage  => "Sur-tension",
            Self::Error        => "Erreur",
        }
    }

    #[allow(dead_code)]
    pub fn is_ok(&self) -> bool {
        *self == Self::Normal
    }
}

// =============================================================================
// Code de défaut
// =============================================================================

/// Code de défaut extrait des bits 5-7 du registre 0x0050.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FaultCode {
    None,
    FireInterlock,
    MotorOverload,
    Disconnect1,
    Disconnect2,
    AbnormalClose,
    AbnormalPhase1,
    AbnormalPhase2,
}

impl FaultCode {
    pub fn from_u8(v: u8) -> Self {
        match v & 0x07 {
            0 => Self::None,
            1 => Self::FireInterlock,
            2 => Self::MotorOverload,
            3 => Self::Disconnect1,
            4 => Self::Disconnect2,
            5 => Self::AbnormalClose,
            6 => Self::AbnormalPhase1,
            _ => Self::AbnormalPhase2,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::None           => "Aucun",
            Self::FireInterlock  => "Interconnexion incendie",
            Self::MotorOverload  => "Surcharge moteur",
            Self::Disconnect1    => "Disjonction I Onduleur",
            Self::Disconnect2    => "Disjonction II Réseau",
            Self::AbnormalClose  => "Fermeture anormale",
            Self::AbnormalPhase1 => "Phase anormale I",
            Self::AbnormalPhase2 => "Phase anormale II",
        }
    }

    pub fn is_fault(&self) -> bool {
        *self != Self::None
    }
}

// =============================================================================
// Mode opératoire (MN uniquement)
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationMode {
    AutoRearm,
    AutoNoRearm,
    Backup,
    Generator,
    GeneratorNoRearm,
    GeneratorBackup,
    Unknown(u16),
}

impl OperationMode {
    pub fn from_u16(v: u16) -> Self {
        match v {
            0 => Self::AutoRearm,
            1 => Self::AutoNoRearm,
            2 => Self::Backup,
            3 => Self::Generator,
            4 => Self::GeneratorNoRearm,
            5 => Self::GeneratorBackup,
            x => Self::Unknown(x),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::AutoRearm         => "Auto-réarmement".to_string(),
            Self::AutoNoRearm       => "Auto-non-réarmement".to_string(),
            Self::Backup            => "Secours".to_string(),
            Self::Generator         => "Générateur".to_string(),
            Self::GeneratorNoRearm  => "Générateur non réarmé".to_string(),
            Self::GeneratorBackup   => "Générateur de secours".to_string(),
            Self::Unknown(v)        => format!("Inconnu ({})", v),
        }
    }
}

// =============================================================================
// Source active
// =============================================================================

/// Source actuellement alimentant la charge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActiveSource {
    #[serde(rename = "Onduleur")]
    Source1, // Onduleur / UPS
    #[serde(rename = "Réseau")]
    Source2, // Réseau / Grid
    #[serde(rename = "Neutre")]
    Neutral, // Position centrale (double déclenché)
}

impl ActiveSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Source1 => "Onduleur",
            Self::Source2 => "Réseau",
            Self::Neutral => "Neutre",
        }
    }

    /// Valeur de /Position pour Venus OS switch.
    /// 0 = AC1 (réseau), 1 = AC2 (onduleur/générateur)
    pub fn venus_position(&self) -> i32 {
        match self {
            Self::Source1 => 1, // AC2 = onduleur/générateur
            Self::Source2 => 0, // AC1 = réseau
            Self::Neutral => 0,
        }
    }

    /// Valeur de /State pour Venus OS switch.
    pub fn venus_state(&self, fault: &FaultCode) -> i32 {
        if fault.is_fault() { return 2; } // alerted
        match self {
            Self::Neutral => 0, // inactive
            _             => 1, // active
        }
    }
}

// =============================================================================
// Snapshot ATS complet
// =============================================================================

/// Snapshot complet d'un ATS CHINT à un instant donné.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtsSnapshot {
    // ── Identification ──────────────────────────────────────────────────────
    pub address:  u8,
    pub name:     String,
    pub model:    String, // "MN" ou "BN"
    pub timestamp: DateTime<Local>,

    // ── Tensions (V) ────────────────────────────────────────────────────────
    pub v1a: f32, // Source 1 phase A
    pub v1b: f32, // Source 1 phase B
    pub v1c: f32, // Source 1 phase C
    pub v2a: f32, // Source 2 phase A
    pub v2b: f32, // Source 2 phase B
    pub v2c: f32, // Source 2 phase C

    // ── Tensions max enregistrées ────────────────────────────────────────────
    pub max1_v: u16, // Source 1 phase A max
    pub max2_v: u16, // Source 2 phase A max

    // ── Statut phases ────────────────────────────────────────────────────────
    pub s1a: PhaseStatus,
    pub s1b: PhaseStatus,
    pub s1c: PhaseStatus,
    pub s2a: PhaseStatus,
    pub s2b: PhaseStatus,
    pub s2c: PhaseStatus,

    // ── Commutation ─────────────────────────────────────────────────────────
    pub sw_mode:    bool,       // true=Auto, false=Manuel
    pub sw1_closed: bool,       // SW1 fermé (Onduleur alimenté)
    pub sw2_closed: bool,       // SW2 fermé (Réseau alimenté)
    pub middle_off: bool,       // Position centrale (double déclenché)
    pub remote:     bool,       // Télécommande activée
    pub fault:      FaultCode,  // Code de défaut

    // ── Source active ────────────────────────────────────────────────────────
    pub active_source: ActiveSource,

    // ── Compteurs & runtime ──────────────────────────────────────────────────
    pub cnt1:      u16, // Commutations S1→S2
    pub cnt2:      u16, // Commutations S2→S1
    pub runtime_h: u16, // Durée totale de fonctionnement (h)

    // ── Version logicielle ───────────────────────────────────────────────────
    pub sw_version: f32, // ex: 2.56

    // ── Fréquences (MN uniquement) ────────────────────────────────────────────
    pub freq1_hz: Option<u8>,
    pub freq2_hz: Option<u8>,

    // ── Mode opératoire (MN uniquement) ──────────────────────────────────────
    pub operation_mode: Option<OperationMode>,

    // ── Seuils de tension (MN uniquement, V) ─────────────────────────────────
    pub uv1: Option<u16>, // Seuil sous-tension source 1
    pub uv2: Option<u16>, // Seuil sous-tension source 2
    pub ov1: Option<u16>, // Seuil sur-tension source 1
    pub ov2: Option<u16>, // Seuil sur-tension source 2

    // ── Délais de commutation (MN uniquement, s) ─────────────────────────────
    pub t1_s: Option<u16>,
    pub t2_s: Option<u16>,
    pub t3_s: Option<u16>,
    pub t4_s: Option<u16>,

    // ── Configuration Modbus de l'ATS ─────────────────────────────────────────
    pub modbus_addr:      Option<u16>,
    pub modbus_baud_code: Option<u16>,
}

impl AtsSnapshot {
    /// Baud rate textuel depuis le code (0=4800, 1=9600, 2=19200, 3=38400).
    pub fn modbus_baud_label(&self) -> &'static str {
        match self.modbus_baud_code {
            Some(0) => "4800",
            Some(1) => "9600",
            Some(2) => "19200",
            Some(3) => "38400",
            _       => "?",
        }
    }

    /// Source 1 OK si phase A est normale.
    #[allow(dead_code)]
    pub fn source1_ok(&self) -> bool {
        self.s1a.is_ok()
    }

    /// Source 2 OK si phase A est normale.
    #[allow(dead_code)]
    pub fn source2_ok(&self) -> bool {
        self.s2a.is_ok()
    }
}

// =============================================================================
// Commandes ATS (registres d'écriture)
// =============================================================================

/// Commandes d'écriture Modbus FC=06 pour l'ATS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AtsCommand {
    /// Activer la télécommande (0x2800 = 0x0004).
    RemoteOn,
    /// Désactiver la télécommande (0x2800 = 0x0000).
    RemoteOff,
    /// Forcer position onduleur/source1 (0x2700 = 0x0000).
    ForceSource1,
    /// Forcer position réseau/source2 (0x2700 = 0x00AA).
    ForceSource2,
    /// Forcer double déclenché / position centrale (0x2700 = 0x00FF).
    ForceDouble,
}

impl AtsCommand {
    /// Registre et valeur à écrire.
    pub fn register_value(&self) -> (u16, u16) {
        match self {
            Self::RemoteOn    => (0x2800, 0x0004),
            Self::RemoteOff   => (0x2800, 0x0000),
            Self::ForceSource1 => (0x2700, 0x0000),
            Self::ForceSource2 => (0x2700, 0x00AA),
            Self::ForceDouble  => (0x2700, 0x00FF),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::RemoteOn    => "Télécommande activée",
            Self::RemoteOff   => "Télécommande désactivée",
            Self::ForceSource1 => "Forçage Onduleur",
            Self::ForceSource2 => "Forçage Réseau",
            Self::ForceDouble  => "Double déclenché",
        }
    }
}
