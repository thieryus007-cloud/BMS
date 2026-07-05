//! Protocole série Toshiba **SUZUMI** (connecteur CN22, UART 9600 8E1).
//!
//! Couche **PURE** : que des transformations d'octets, **aucune I/O**. Testable
//! intégralement sur host (`cargo test`). Spécification : voir
//! `docs/toshiba-suzumi-rs-plan.md` §6/§8 — vérifiée octet par octet contre
//! `pedobry/esphome_toshiba_suzumi`, recoupée avec `o0Zz/climate-uart`.
//!
//! ⚠️ Ce protocole (CN22, 9600 bauds, checksum = `256 − Σ`) n'a **rien à voir**
//! avec le protocole A/B « TCC-Link » (2400 bauds, CRC XOR) de
//! `issalig/toshiba_air_cond` — cf. plan §15. Ne pas confondre.

// ============================================================================
// Constantes de trame (§6.2)
// ============================================================================

/// En-tête de toutes les trames (STX).
pub const STX: u8 = 0x02;

/// Préfixe fixe d'une trame d'**écriture** (commande), octets `[0..12]`.
/// Suivi de `cmd`, `value`, `checksum`. Cf. §6.2.
const WRITE_PREFIX: [u8; 12] = [
    0x02, 0x00, 0x03, 0x10, 0x00, 0x00, 0x07, 0x01, 0x30, 0x01, 0x00, 0x02,
];

/// Préfixe fixe d'une trame de **lecture** (requête), octets `[0..12]`.
/// Suivi de `cmd`, `checksum`. Cf. §6.2.
const READ_PREFIX: [u8; 12] = [
    0x02, 0x00, 0x03, 0x10, 0x00, 0x00, 0x06, 0x01, 0x30, 0x01, 0x00, 0x01,
];

// ============================================================================
// Trames de handshake (§6.4) — constantes pré-calculées (checksum inclus)
// ============================================================================

/// 6 trames d'initialisation, envoyées en premier (§6.4).
///
/// ⚠️ `HANDSHAKE[4]` se termine par `0xFE` (valeur pedobry, éprouvée sur le
/// terrain) alors que la règle de checksum donnerait `0xFB` (valeur o0Zz).
/// L'unité ignore vraisemblablement le checksum des trames d'init. Conserver
/// `0xFE` ; tester `0xFB` seulement si le handshake échoue au banc (plan §6.4).
pub const HANDSHAKE: [&[u8]; 6] = [
    &[2, 255, 255, 0, 0, 0, 0, 2],
    &[2, 255, 255, 1, 0, 0, 1, 2, 254],
    &[2, 0, 0, 0, 0, 0, 2, 2, 2, 250],
    &[2, 0, 1, 129, 1, 0, 2, 0, 0, 123],
    &[2, 0, 1, 2, 0, 0, 2, 0, 0, 254], // <- 254 (0xFE) ; alt. checksum-correct = 251 (0xFB)
    &[2, 0, 2, 0, 0, 0, 0, 254],
];

/// 2 trames envoyées **après** la temporisation de 2000 ms (§6.4).
pub const AFTER_HANDSHAKE: [&[u8]; 2] = [
    &[2, 0, 2, 1, 0, 0, 2, 0, 0, 251],
    &[2, 0, 2, 2, 0, 0, 2, 0, 0, 250],
];

/// Ordre des lectures initiales après le handshake (`getInitData`, §6.4.1).
pub const INIT_QUERIES: [CmdType; 9] = [
    CmdType::PowerState,
    CmdType::Mode,
    CmdType::TargetTemp,
    CmdType::Fan,
    CmdType::PowerSel,
    CmdType::Swing,
    CmdType::RoomTemp,
    CmdType::OutdoorTemp,
    CmdType::SpecialMode,
];

// ============================================================================
// Temporisations (§8.2) — constantes du firmware d'origine
// ============================================================================

/// Délai minimal entre deux trames émises (`COMMAND_DELAY`).
pub const COMMAND_DELAY_MS: u32 = 100;
/// Temporisation stricte post-handshake avant `AFTER_HANDSHAKE`.
pub const HANDSHAKE_DELAY_MS: u32 = 2000;
/// Sans octet reçu pendant ce délai → purger le buffer partiel (`RECEIVE_TIMEOUT`).
pub const RECEIVE_TIMEOUT_MS: u32 = 200;

// ============================================================================
// Bornes de température (§6.11)
// ============================================================================

/// Consigne minimale en mode standard.
pub const MIN_TEMP_STANDARD: u8 = 17;
/// Consigne maximale.
pub const MAX_TEMP: u8 = 30;
/// Décalage appliqué sur le fil quand le mode 8° (garde hors-gel) est actif.
pub const SPECIAL_TEMP_OFFSET: u8 = 16;
/// Valeur brute signifiant « température invalide » (ambiante / extérieure).
pub const TEMP_INVALID: u8 = 127;

// ============================================================================
// Type de commande (§6.5)
// ============================================================================

/// Identifiant de commande/capteur (octet `[12]` en émission, ou octet de type
/// en réception). Valeurs exactes du firmware d'origine.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmdType {
    Handshake = 0,
    Delay = 1,
    PowerState = 128,   // 0x80
    PowerSel = 135,     // 0x87
    ComfortSleep = 148, // 0x94 — déclaré mais non utilisé par le firmware d'origine
    Fan = 160,          // 0xA0
    Swing = 163,        // 0xA3
    Mode = 176,         // 0xB0
    TargetTemp = 179,   // 0xB3
    RoomTemp = 187,     // 0xBB
    OutdoorTemp = 190,  // 0xBE
    SelfClean = 203,    // 0xCB
    WifiLed1 = 222,     // 0xDE
    WifiLed2 = 223,     // 0xDF
    IduStatus = 228,    // 0xE4
    OduStatus = 229,    // 0xE5
    SpecialMode = 247,  // 0xF7
}

impl CmdType {
    /// Convertit un octet en `CmdType`, ou `None` si inconnu.
    pub fn from_u8(v: u8) -> Option<Self> {
        use CmdType::*;
        Some(match v {
            0 => Handshake,
            1 => Delay,
            128 => PowerState,
            135 => PowerSel,
            148 => ComfortSleep,
            160 => Fan,
            163 => Swing,
            176 => Mode,
            179 => TargetTemp,
            187 => RoomTemp,
            190 => OutdoorTemp,
            203 => SelfClean,
            222 => WifiLed1,
            223 => WifiLed2,
            228 => IduStatus,
            229 => OduStatus,
            247 => SpecialMode,
            _ => return None,
        })
    }
}

// ============================================================================
// Énumérations de valeurs (§6.6 – §6.8)
// ============================================================================

/// Mode CVC. ⚠️ Il n'y a **pas** de valeur « OFF » : l'arrêt passe par
/// [`CmdType::PowerState`] + [`State::Off`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    HeatCool = 65, // 0x41 — « auto »
    Cool = 66,     // 0x42
    Heat = 67,     // 0x43
    Dry = 68,      // 0x44
    FanOnly = 69,  // 0x45
}

impl Mode {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            65 => Mode::HeatCool,
            66 => Mode::Cool,
            67 => Mode::Heat,
            68 => Mode::Dry,
            69 => Mode::FanOnly,
            _ => return None,
        })
    }
}

/// Vitesse de ventilation.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fan {
    Quiet = 49,  // 0x31
    Low = 50,    // 0x32
    Level2 = 51, // 0x33 — custom « Low-Medium »
    Medium = 52, // 0x34
    Level4 = 53, // 0x35 — custom « Medium-High »
    High = 54,   // 0x36
    Auto = 65,   // 0x41
}

impl Fan {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            49 => Fan::Quiet,
            50 => Fan::Low,
            51 => Fan::Level2,
            52 => Fan::Medium,
            53 => Fan::Level4,
            54 => Fan::High,
            65 => Fan::Auto,
            _ => return None,
        })
    }
}

/// Orientation du flux d'air (swing + positions verticales fixes).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Swing {
    Off = 49,        // 0x31
    Vertical = 65,   // 0x41
    Horizontal = 66, // 0x42
    Both = 67,       // 0x43
    VFix1 = 80,      // 0x50
    VFix2 = 81,      // 0x51
    VFix3 = 82,      // 0x52
    VFix4 = 83,      // 0x53
    VFix5 = 84,      // 0x54
}

impl Swing {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            49 => Swing::Off,
            65 => Swing::Vertical,
            66 => Swing::Horizontal,
            67 => Swing::Both,
            80 => Swing::VFix1,
            81 => Swing::VFix2,
            82 => Swing::VFix3,
            83 => Swing::VFix4,
            84 => Swing::VFix5,
            _ => return None,
        })
    }
}

/// État marche/arrêt (via [`CmdType::PowerState`]).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    On = 48,  // 0x30
    Off = 49, // 0x31
}

impl State {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            48 => State::On,
            49 => State::Off,
            _ => return None,
        })
    }
}

/// Niveau de puissance (via [`CmdType::PowerSel`]).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PwrLevel {
    Pct50 = 50,
    Pct75 = 75,
    Pct100 = 100,
}

impl PwrLevel {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            50 => PwrLevel::Pct50,
            75 => PwrLevel::Pct75,
            100 => PwrLevel::Pct100,
            _ => return None,
        })
    }
}

/// État de l'auto-nettoyage (via [`CmdType::SelfClean`]).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfCleanState {
    Running = 0x18,
    Off = 0x10,
}

impl SelfCleanState {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0x18 => SelfCleanState::Running,
            0x10 => SelfCleanState::Off,
            _ => return None,
        })
    }
}

/// Préréglages / modes spéciaux (via [`CmdType::SpecialMode`]). Valeurs **non
/// contiguës** → table explicite obligatoire (§6.7).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialMode {
    Standard = 0,
    HiPower = 1,
    Silent1 = 2,
    Eco = 3,
    EightDeg = 4, // garde hors-gel
    Sleep = 5,
    Floor = 6,
    Comfort = 7,
    Silent2 = 10,
    Fireplace1 = 32,
    Fireplace2 = 48,
}

impl SpecialMode {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => SpecialMode::Standard,
            1 => SpecialMode::HiPower,
            2 => SpecialMode::Silent1,
            3 => SpecialMode::Eco,
            4 => SpecialMode::EightDeg,
            5 => SpecialMode::Sleep,
            6 => SpecialMode::Floor,
            7 => SpecialMode::Comfort,
            10 => SpecialMode::Silent2,
            32 => SpecialMode::Fireplace1,
            48 => SpecialMode::Fireplace2,
            _ => return None,
        })
    }
}

// ============================================================================
// Checksum (§6.3)
// ============================================================================

/// Checksum SUZUMI. `frame` = octets **avant** l'octet de checksum (en-tête
/// `0x02` inclus). Algo : `256 − Σ(frame[1..])` (mod 256), soit la négation en
/// complément à deux.
pub fn checksum(frame: &[u8]) -> u8 {
    frame[1..]
        .iter()
        .fold(0u8, |acc, &b| acc.wrapping_add(b))
        .wrapping_neg()
}

// ============================================================================
// Construction de trames (§6.2)
// ============================================================================

/// Construit une trame de **lecture** (requête) pour `cmd` (14 octets).
pub fn build_read(cmd: CmdType) -> Vec<u8> {
    let mut f = Vec::with_capacity(14);
    f.extend_from_slice(&READ_PREFIX);
    f.push(cmd as u8);
    f.push(checksum(&f));
    f
}

/// Construit une trame d'**écriture** (commande) pour `cmd`=`value` (15 octets).
pub fn build_write(cmd: CmdType, value: u8) -> Vec<u8> {
    let mut f = Vec::with_capacity(15);
    f.extend_from_slice(&WRITE_PREFIX);
    f.push(cmd as u8);
    f.push(value);
    f.push(checksum(&f));
    f
}

/// Construit la trame d'écriture de consigne, en gérant le décalage `+16` du
/// mode 8° (garde hors-gel). `eight_deg_active` = mode `EightDeg` en cours.
pub fn build_target_temp(celsius: u8, eight_deg_active: bool) -> Vec<u8> {
    let on_wire = if eight_deg_active {
        celsius.wrapping_add(SPECIAL_TEMP_OFFSET)
    } else {
        celsius
    };
    build_write(CmdType::TargetTemp, on_wire)
}

/// Trames à émettre pour (dés)activer la LED WiFi de l'unité (§6.11).
pub fn build_wifi_led(on: bool) -> [Vec<u8>; 2] {
    if on {
        [
            build_write(CmdType::WifiLed1, 0x05),
            build_write(CmdType::WifiLed2, 0x00),
        ]
    } else {
        [
            build_write(CmdType::WifiLed1, 0x00),
            build_write(CmdType::WifiLed2, 0x80),
        ]
    }
}

// ============================================================================
// Validation d'une trame reçue (§8.1)
// ============================================================================

/// Erreurs de validation/parsing d'une trame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolError {
    TooShort,
    BadHeader,
    BadFrame,
    BadLength,
    ChecksumMismatch { expected: u8, got: u8 },
}

/// Valide une trame **complète** (checksum au dernier octet). Reproduit
/// `validate_message_` : en-tête `0x02`, octet `[2]==0x03`, longueur déduite de
/// `[6]` (`cks_idx = 6 + data[6] + 1`, taille = `cks_idx + 1`).
pub fn validate_frame(data: &[u8]) -> Result<(), ProtocolError> {
    if data.len() < 8 {
        return Err(ProtocolError::TooShort);
    }
    if data[0] != STX {
        return Err(ProtocolError::BadHeader);
    }
    if data[2] != 0x03 {
        return Err(ProtocolError::BadFrame);
    }
    let cks_idx = 6 + data[6] as usize + 1;
    if data.len() != cks_idx + 1 {
        return Err(ProtocolError::BadLength);
    }
    let expected = checksum(&data[..cks_idx]);
    let got = data[cks_idx];
    if got != expected {
        return Err(ProtocolError::ChecksumMismatch { expected, got });
    }
    Ok(())
}

// ============================================================================
// Parsing des réponses (§6.9 / §6.10)
// ============================================================================

/// Diagnostics unité extérieure (ODU, `0xE5`). Températures en °C (`i8`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OduStatus {
    pub cdu_td_temp: i8,
    pub cdu_ts_temp: i8,
    pub cdu_te_temp: i8,
    pub cdu_load_pct: f32, // octet / 1.7
    pub cdu_iac_a: u8,
}

/// Diagnostics unité intérieure (IDU, `0xE4`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IduStatus {
    pub fcu_tc_temp: i8,
    pub fcu_tcj_temp: i8,
    pub fcu_fan_rpm: u16,
}

/// Résultat du parsing d'une trame reçue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Field {
    /// ACK d'une commande (trame de longueur 16) — sans donnée.
    Ack,
    Power(State),
    Mode(Mode),
    /// Consigne en °C, **déjà** « dé-offsetée » si mode 8° actif.
    TargetTemp(u8),
    /// Température ambiante ; `None` si valeur invalide (127).
    RoomTemp(Option<i8>),
    /// Température extérieure ; `None` si invalide (127).
    OutdoorTemp(Option<i8>),
    Fan(Fan),
    Swing(Swing),
    PwrLevel(PwrLevel),
    SpecialMode(SpecialMode),
    SelfClean(SelfCleanState),
    Odu(OduStatus),
    Idu(IduStatus),
    /// Type/valeur non reconnus (on ne panique jamais sur une trame inconnue).
    Unknown {
        cmd: u8,
        value: u8,
    },
}

/// Parse une trame reçue **déjà validée** (cf. [`validate_frame`]).
///
/// `eight_deg_active` sert à « dé-offseter » la consigne quand le mode 8° est en
/// cours (la valeur arrive décalée de +16 sur le fil). Renvoie [`Field`], ou une
/// erreur seulement si la longueur est inexploitable.
pub fn parse_response(data: &[u8], eight_deg_active: bool) -> Result<Field, ProtocolError> {
    // Dispatch par longueur totale (§6.9).
    let (cmd_idx, val_idx, ext_off) = match data.len() {
        15 => (12usize, 13usize, 0usize),
        16 => return Ok(Field::Ack),
        17 => (14, 15, 0),
        22 => (12, 0, 13), // trames étendues ODU/IDU
        24 => (14, 0, 15),
        _ => return Err(ProtocolError::BadLength),
    };
    if cmd_idx >= data.len() {
        return Err(ProtocolError::TooShort);
    }
    let cmd = data[cmd_idx];
    let value = *data.get(val_idx).unwrap_or(&0);

    // Trames étendues (longueur 22/24) : ODU/IDU uniquement.
    if ext_off != 0 {
        return Ok(parse_extended(cmd, data, ext_off));
    }

    let field = match CmdType::from_u8(cmd) {
        Some(CmdType::PowerState) => State::from_u8(value)
            .map(Field::Power)
            .unwrap_or(Field::Unknown { cmd, value }),
        Some(CmdType::Mode) => Mode::from_u8(value)
            .map(Field::Mode)
            .unwrap_or(Field::Unknown { cmd, value }),
        Some(CmdType::TargetTemp) => {
            let celsius = if eight_deg_active {
                value.wrapping_sub(SPECIAL_TEMP_OFFSET)
            } else {
                value
            };
            Field::TargetTemp(celsius)
        }
        Some(CmdType::RoomTemp) => Field::RoomTemp(decode_temp(value)),
        Some(CmdType::OutdoorTemp) => Field::OutdoorTemp(decode_temp(value)),
        Some(CmdType::Fan) => Fan::from_u8(value)
            .map(Field::Fan)
            .unwrap_or(Field::Unknown { cmd, value }),
        Some(CmdType::Swing) => Swing::from_u8(value)
            .map(Field::Swing)
            .unwrap_or(Field::Unknown { cmd, value }),
        Some(CmdType::PowerSel) => PwrLevel::from_u8(value)
            .map(Field::PwrLevel)
            .unwrap_or(Field::Unknown { cmd, value }),
        Some(CmdType::SpecialMode) => SpecialMode::from_u8(value)
            .map(Field::SpecialMode)
            .unwrap_or(Field::Unknown { cmd, value }),
        Some(CmdType::SelfClean) => SelfCleanState::from_u8(value)
            .map(Field::SelfClean)
            .unwrap_or(Field::Unknown { cmd, value }),
        _ => Field::Unknown { cmd, value },
    };
    Ok(field)
}

/// `127` = invalide → `None`, sinon la température signée.
fn decode_temp(value: u8) -> Option<i8> {
    if value == TEMP_INVALID {
        None
    } else {
        Some(value as i8)
    }
}

/// Parse une trame étendue ODU/IDU (§6.10). `off` = offset de base (13 ou 15).
fn parse_extended(cmd: u8, data: &[u8], off: usize) -> Field {
    let at = |i: usize| data.get(off + i).copied().unwrap_or(0);
    match CmdType::from_u8(cmd) {
        Some(CmdType::OduStatus) => Field::Odu(OduStatus {
            cdu_td_temp: at(0) as i8,
            cdu_ts_temp: at(1) as i8,
            cdu_te_temp: at(2) as i8,
            cdu_load_pct: at(3) as f32 / 1.7,
            cdu_iac_a: at(6),
        }),
        Some(CmdType::IduStatus) => Field::Idu(IduStatus {
            fcu_tc_temp: at(0) as i8,
            fcu_tcj_temp: at(1) as i8,
            fcu_fan_rpm: at(2) as u16,
        }),
        _ => Field::Unknown { cmd, value: 0 },
    }
}

// ============================================================================
// Tests (host — aucun matériel requis)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Le checksum recalculé doit égaler le dernier octet de chaque trame de
    /// handshake — SAUF `HANDSHAKE[4]` (anomalie pedobry connue : 0xFE au lieu
    /// de 0xFB). Ce test *documente* l'anomalie plutôt que de la masquer.
    #[test]
    fn handshake_checksums_match_except_index_4() {
        for (i, frame) in HANDSHAKE.iter().enumerate() {
            let (body, cks) = frame.split_at(frame.len() - 1);
            let computed = checksum(body);
            if i == 4 {
                assert_eq!(cks[0], 0xFE, "HANDSHAKE[4] doit rester la valeur pedobry");
                assert_eq!(
                    computed, 0xFB,
                    "la règle de checksum donne 0xFB (valeur o0Zz)"
                );
                assert_ne!(cks[0], computed, "anomalie attendue sur HANDSHAKE[4]");
            } else {
                assert_eq!(cks[0], computed, "HANDSHAKE[{i}] checksum invalide");
            }
        }
        for (i, frame) in AFTER_HANDSHAKE.iter().enumerate() {
            let (body, cks) = frame.split_at(frame.len() - 1);
            assert_eq!(
                cks[0],
                checksum(body),
                "AFTER_HANDSHAKE[{i}] checksum invalide"
            );
        }
    }

    #[test]
    fn checksum_is_twos_complement_negation() {
        // Somme nulle → 0 (256 mod 256).
        assert_eq!(checksum(&[STX]), 0);
        // Exemple contrôlé : Σ(1..) = 5 → 256-5 = 251.
        assert_eq!(checksum(&[STX, 2, 3]), 251);
    }

    #[test]
    fn build_read_room_temp_exact_bytes() {
        // Trame lecture ROOM_TEMP (0xBB). Checksum calculé indépendamment = 0xF9.
        let f = build_read(CmdType::RoomTemp);
        assert_eq!(
            f,
            vec![
                0x02, 0x00, 0x03, 0x10, 0x00, 0x00, 0x06, 0x01, 0x30, 0x01, 0x00, 0x01, 0xBB, 0xF9
            ]
        );
        assert_eq!(f.len(), 14);
        assert!(validate_frame(&f).is_ok());
    }

    #[test]
    fn build_write_target_temp_exact_bytes() {
        // Écriture TARGET_TEMP=22 (0xB3, 22). Checksum indépendant = 0xE9.
        let f = build_write(CmdType::TargetTemp, 22);
        assert_eq!(
            f,
            vec![
                0x02, 0x00, 0x03, 0x10, 0x00, 0x00, 0x07, 0x01, 0x30, 0x01, 0x00, 0x02, 0xB3, 22,
                0xE9
            ]
        );
        assert_eq!(f.len(), 15);
        assert!(validate_frame(&f).is_ok());
    }

    #[test]
    fn eight_degree_offset_roundtrips() {
        // 8°C en mode garde hors-gel → 24 sur le fil.
        let f = build_target_temp(8, true);
        assert_eq!(f[13], 24, "8°C doit être décalé de +16 = 24");
        // Réception : la même valeur 24 doit redonner 8°C.
        let resp = make_len15(CmdType::TargetTemp, 24);
        assert_eq!(parse_response(&resp, true).unwrap(), Field::TargetTemp(8));
        // Sans mode 8° : pas de décalage.
        assert_eq!(build_target_temp(22, false)[13], 22);
    }

    #[test]
    fn validate_frame_rejects_corruption() {
        let mut f = build_write(CmdType::Mode, Mode::Heat as u8);
        // Mauvais checksum.
        *f.last_mut().unwrap() ^= 0xFF;
        assert!(matches!(
            validate_frame(&f),
            Err(ProtocolError::ChecksumMismatch { .. })
        ));
        // Mauvais en-tête.
        let mut g = build_read(CmdType::RoomTemp);
        g[0] = 0x03;
        assert_eq!(validate_frame(&g), Err(ProtocolError::BadHeader));
        // Trop court.
        assert_eq!(validate_frame(&[0x02, 0x00]), Err(ProtocolError::TooShort));
    }

    #[test]
    fn parse_common_fields() {
        assert_eq!(
            parse_response(&make_len15(CmdType::PowerState, State::On as u8), false).unwrap(),
            Field::Power(State::On)
        );
        assert_eq!(
            parse_response(&make_len15(CmdType::Mode, Mode::Cool as u8), false).unwrap(),
            Field::Mode(Mode::Cool)
        );
        assert_eq!(
            parse_response(&make_len15(CmdType::RoomTemp, 21), false).unwrap(),
            Field::RoomTemp(Some(21))
        );
        // 127 = invalide.
        assert_eq!(
            parse_response(&make_len15(CmdType::RoomTemp, 127), false).unwrap(),
            Field::RoomTemp(None)
        );
        assert_eq!(
            parse_response(&make_len15(CmdType::Fan, Fan::Auto as u8), false).unwrap(),
            Field::Fan(Fan::Auto)
        );
        assert_eq!(
            parse_response(
                &make_len15(CmdType::SpecialMode, SpecialMode::Eco as u8),
                false
            )
            .unwrap(),
            Field::SpecialMode(SpecialMode::Eco)
        );
    }

    #[test]
    fn parse_ack_and_unknown() {
        // Longueur 16 = ACK.
        assert_eq!(parse_response(&[0u8; 16], false).unwrap(), Field::Ack);
        // Type inconnu (0x99) dans une trame de longueur 15.
        let f = make_len15_raw(0x99, 0x01);
        assert_eq!(
            parse_response(&f, false).unwrap(),
            Field::Unknown {
                cmd: 0x99,
                value: 0x01
            }
        );
    }

    #[test]
    fn parse_odu_extended() {
        // Trame longueur 22, offset 13 : td=+30, ts=+10, te=-5, load byte=170, iac=3.
        let mut f = vec![0u8; 22];
        f[12] = CmdType::OduStatus as u8;
        f[13] = 30; // cdu_td
        f[14] = 10; // cdu_ts
        f[15] = (-5i8) as u8; // cdu_te
        f[16] = 170; // load → 170/1.7 = 100.0 %
        f[19] = 3; // iac (offset+6 = 13+6 = 19)
        match parse_response(&f, false).unwrap() {
            Field::Odu(o) => {
                assert_eq!(o.cdu_td_temp, 30);
                assert_eq!(o.cdu_te_temp, -5);
                assert!((o.cdu_load_pct - 100.0).abs() < 0.01);
                assert_eq!(o.cdu_iac_a, 3);
            }
            other => panic!("attendu Odu, obtenu {other:?}"),
        }
    }

    #[test]
    fn cmdtype_and_enum_roundtrips() {
        // Cohérence des tables from_u8 avec les valeurs numériques.
        assert_eq!(CmdType::from_u8(0xB3), Some(CmdType::TargetTemp));
        assert_eq!(CmdType::from_u8(0x00_u8.wrapping_sub(1)), None); // 0xFF inconnu
        assert_eq!(Mode::from_u8(65), Some(Mode::HeatCool));
        assert_eq!(Swing::from_u8(84), Some(Swing::VFix5));
        assert_eq!(SpecialMode::from_u8(48), Some(SpecialMode::Fireplace2));
        assert_eq!(SpecialMode::from_u8(99), None);
    }

    /// Validation croisée avec o0Zz/climate-uart (§14) : les codes fonction et
    /// les valeurs doivent correspondre exactement.
    #[test]
    fn cross_check_o0zz_values() {
        assert_eq!(CmdType::PowerState as u8, 0x80);
        assert_eq!(CmdType::Mode as u8, 0xB0);
        assert_eq!(CmdType::TargetTemp as u8, 0xB3);
        assert_eq!(CmdType::Fan as u8, 0xA0);
        assert_eq!(CmdType::Swing as u8, 0xA3);
        assert_eq!(CmdType::RoomTemp as u8, 0xBB);
        assert_eq!(State::On as u8, 0x30);
        assert_eq!(State::Off as u8, 0x31);
        assert_eq!(Mode::HeatCool as u8, 0x41);
        assert_eq!(Mode::FanOnly as u8, 0x45);
    }

    // ---- helpers de test ----

    /// Fabrique une trame de réponse de longueur 15 (sensor=[12], value=[13])
    /// avec un checksum valide.
    fn make_len15(cmd: CmdType, value: u8) -> Vec<u8> {
        make_len15_raw(cmd as u8, value)
    }

    fn make_len15_raw(cmd: u8, value: u8) -> Vec<u8> {
        // Reprend le gabarit d'écriture (byte[6]=7 → longueur 15, cks en [14]).
        let mut f = WRITE_PREFIX.to_vec();
        f.push(cmd);
        f.push(value);
        f.push(checksum(&f));
        assert_eq!(f.len(), 15);
        f
    }
}
