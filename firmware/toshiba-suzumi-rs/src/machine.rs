//! Client de connexion Toshiba — **logique pure, sans I/O**.
//!
//! Orchestration : séquence de handshake, temporisation inter-trames
//! (`COMMAND_DELAY_MS`), file de commandes, assemblage des trames reçues,
//! mise à jour de l'état, watchdog de re-handshake. Piloté par des *événements*
//! (`poll_tx`, `on_rx_byte`, `on_tick`) → intégralement testable sur host, y
//! compris avec un simulateur d'unité (voir les tests).
//!
//! Le branchement I/O réel (UART/WiFi/MQTT ESP-IDF) se contentera d'appeler ces
//! méthodes : lire l'UART → `on_rx_byte`, écrire l'UART ← `poll_tx`, timer →
//! `on_tick`. Voir `docs/toshiba-suzumi-rs-plan.md` §5 (Phase 5).

use std::collections::VecDeque;

use crate::framing::{FrameAccumulator, Rx};
use crate::mqtt_payload::Command;
use crate::protocol::{
    build_read, build_target_temp, build_write, parse_response, CmdType, Fan, Field, Mode,
    PwrLevel, SpecialMode, State, Swing, AFTER_HANDSHAKE, COMMAND_DELAY_MS, HANDSHAKE,
    HANDSHAKE_DELAY_MS, INIT_QUERIES, MIN_TEMP_STANDARD, RECEIVE_TIMEOUT_MS,
};
use crate::state::ToshibaState;

/// Sans trame reçue pendant ce délai en phase `Online`, on rejoue le handshake.
pub const WATCHDOG_MS: u32 = 60_000;

/// Phase de connexion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Avant [`Client::start`].
    Idle,
    /// Handshake + lectures initiales en cours d'émission.
    Handshaking,
    /// Communication normale établie.
    Online,
}

/// Élément de la file d'émission.
#[derive(Debug, Clone, PartialEq, Eq)]
enum OutFrame {
    Bytes(Vec<u8>),
    /// Attente stricte (ms) avant la trame suivante (temporisation post-handshake).
    Delay(u32),
}

/// Client protocolaire. Ne fait **aucune** I/O : l'appelant relaie octets et temps.
#[derive(Debug)]
pub struct Client {
    phase: Phase,
    queue: VecDeque<OutFrame>,
    accumulator: FrameAccumulator,
    state: ToshibaState,
    last_tx_ms: u32,
    last_rx_ms: u32,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    pub fn new() -> Self {
        Self {
            phase: Phase::Idle,
            queue: VecDeque::new(),
            accumulator: FrameAccumulator::new(),
            state: ToshibaState::new(),
            last_tx_ms: 0,
            last_rx_ms: 0,
        }
    }

    /// (Re)démarre la connexion : purge, puis programme handshake + init.
    pub fn start(&mut self, now: u32) {
        self.queue.clear();
        self.accumulator.reset();
        for frame in HANDSHAKE {
            self.queue.push_back(OutFrame::Bytes(frame.to_vec()));
        }
        self.queue.push_back(OutFrame::Delay(HANDSHAKE_DELAY_MS));
        for frame in AFTER_HANDSHAKE {
            self.queue.push_back(OutFrame::Bytes(frame.to_vec()));
        }
        for cmd in INIT_QUERIES {
            self.queue.push_back(OutFrame::Bytes(build_read(cmd)));
        }
        self.phase = Phase::Handshaking;
        self.last_rx_ms = now;
        // Autorise une première émission immédiate.
        self.last_tx_ms = now.wrapping_sub(COMMAND_DELAY_MS);
    }

    /// État courant de l'unité.
    pub fn status(&self) -> &ToshibaState {
        &self.state
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn is_online(&self) -> bool {
        self.phase == Phase::Online
    }

    /// Nombre de trames encore à émettre (diagnostic/tests).
    pub fn pending_tx(&self) -> usize {
        self.queue.len()
    }

    // ---- Émission (appelé par l'écrivain UART) --------------------------------

    /// Renvoie la prochaine trame à émettre **maintenant**, en respectant le délai
    /// inter-trames et en n'émettant jamais pendant une réception en cours.
    pub fn poll_tx(&mut self, now: u32) -> Option<Vec<u8>> {
        if !self.accumulator.is_empty() {
            return None; // réception en cours → on n'émet pas
        }
        if now.wrapping_sub(self.last_tx_ms) < COMMAND_DELAY_MS {
            return None;
        }
        loop {
            match self.queue.front() {
                None => return None,
                Some(OutFrame::Delay(d)) => {
                    if now.wrapping_sub(self.last_tx_ms) < *d {
                        return None; // délai pas encore écoulé
                    }
                    self.queue.pop_front(); // délai consommé, on tente la trame suivante
                }
                Some(OutFrame::Bytes(_)) => {
                    let Some(OutFrame::Bytes(bytes)) = self.queue.pop_front() else {
                        unreachable!()
                    };
                    self.last_tx_ms = now;
                    if self.queue.is_empty() && self.phase == Phase::Handshaking {
                        self.phase = Phase::Online;
                    }
                    return Some(bytes);
                }
            }
        }
    }

    // ---- Réception (appelé par le lecteur UART) -------------------------------

    /// Ajoute un octet reçu. Sur trame complète et valide, met à jour l'état et
    /// renvoie le [`Field`] parsé.
    pub fn on_rx_byte(&mut self, byte: u8, now: u32) -> Option<Field> {
        self.last_rx_ms = now;
        match self.accumulator.push(byte) {
            Rx::Frame(frame) => {
                let field = parse_response(&frame, self.state.eight_deg_active()).ok()?;
                self.state.apply(field);
                Some(field)
            }
            Rx::Incomplete | Rx::Error(_) => None,
        }
    }

    // ---- Timer ----------------------------------------------------------------

    /// À appeler périodiquement. Gère le time-out de réception (purge d'une trame
    /// partielle) et le watchdog de re-handshake.
    pub fn on_tick(&mut self, now: u32) {
        if !self.accumulator.is_empty() && now.wrapping_sub(self.last_rx_ms) > RECEIVE_TIMEOUT_MS {
            self.accumulator.reset();
        }
        if self.phase == Phase::Online && now.wrapping_sub(self.last_rx_ms) > WATCHDOG_MS {
            self.start(now); // unité muette → on rejoue tout
        }
    }

    // ---- Commandes de haut niveau (enfilées, émises par poll_tx) --------------

    /// Enfile une lecture (requête) d'un capteur/réglage.
    pub fn queue_read(&mut self, cmd: CmdType) {
        self.queue.push_back(OutFrame::Bytes(build_read(cmd)));
    }

    /// Enfile une écriture brute.
    pub fn queue_write(&mut self, cmd: CmdType, value: u8) {
        self.queue
            .push_back(OutFrame::Bytes(build_write(cmd, value)));
    }

    /// Allume/éteint l'unité (via `PowerState`).
    pub fn set_power(&mut self, on: bool) {
        let v = if on { State::On } else { State::Off } as u8;
        self.queue_write(CmdType::PowerState, v);
    }

    /// Change le mode. Rallume l'unité au préalable si elle est éteinte
    /// (comportement du firmware d'origine, `control()`).
    pub fn set_mode(&mut self, mode: Mode) {
        if !self.state.is_on() {
            self.queue_write(CmdType::PowerState, State::On as u8);
        }
        self.queue_write(CmdType::Mode, mode as u8);
    }

    pub fn set_fan(&mut self, fan: Fan) {
        self.queue_write(CmdType::Fan, fan as u8);
    }

    pub fn set_swing(&mut self, swing: Swing) {
        self.queue_write(CmdType::Swing, swing as u8);
    }

    pub fn set_power_level(&mut self, level: PwrLevel) {
        self.queue_write(CmdType::PowerSel, level as u8);
    }

    pub fn set_preset(&mut self, preset: SpecialMode) {
        self.queue_write(CmdType::SpecialMode, preset as u8);
        self.state.special_mode = Some(preset);
    }

    /// Règle la consigne (°C), en gérant la bascule automatique du mode 8°
    /// (garde hors-gel) sous 17 °C et le décalage `+16` sur le fil (§6.11).
    pub fn set_target_temp(&mut self, celsius: u8) {
        let want_eight = celsius < MIN_TEMP_STANDARD;
        let is_eight = self.state.eight_deg_active();
        if want_eight && !is_eight {
            self.queue_write(CmdType::SpecialMode, SpecialMode::EightDeg as u8);
            self.state.special_mode = Some(SpecialMode::EightDeg);
        } else if !want_eight && is_eight {
            self.queue_write(CmdType::SpecialMode, SpecialMode::Standard as u8);
            self.state.special_mode = Some(SpecialMode::Standard);
        }
        let frame = build_target_temp(celsius, self.state.eight_deg_active());
        self.queue.push_back(OutFrame::Bytes(frame));
    }

    /// Applique une commande typée (issue de [`crate::mqtt_payload::parse_command`]).
    pub fn apply_command(&mut self, cmd: Command) {
        match cmd {
            Command::SetPower(on) => self.set_power(on),
            Command::SetMode(m) => self.set_mode(m),
            Command::SetTargetTemp(t) => self.set_target_temp(t),
            Command::SetFan(f) => self.set_fan(f),
            Command::SetSwing(s) => self.set_swing(s),
            Command::SetPreset(p) => self.set_preset(p),
            Command::SetPowerLevel(l) => self.set_power_level(l),
        }
    }
}

// ============================================================================
// Tests — simulateur d'unité en mémoire (aucun matériel)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::checksum;

    /// Unité intérieure simulée : répond aux lectures, applique les écritures.
    struct FakeUnit {
        power: State,
        mode: Mode,
        target: u8,
        room: i8,
        outdoor: i8,
        fan: Fan,
        swing: Swing,
        pwr: PwrLevel,
        special: SpecialMode,
    }

    impl Default for FakeUnit {
        fn default() -> Self {
            Self {
                power: State::On,
                mode: Mode::Cool,
                target: 24,
                room: 21,
                outdoor: 8,
                fan: Fan::Auto,
                swing: Swing::Vertical,
                pwr: PwrLevel::Pct100,
                special: SpecialMode::Standard,
            }
        }
    }

    impl FakeUnit {
        /// Reçoit une trame émise par le client ; renvoie la réponse (peut être vide).
        fn feed(&mut self, frame: &[u8]) -> Vec<u8> {
            // Handshake : en-tête 0x02 mais [2] != 0x03 → ignoré.
            if frame.len() < 14 || frame[0] != 0x02 || frame[2] != 0x03 {
                return Vec::new();
            }
            let cmd = frame[12];
            if frame[11] == 0x01 {
                // lecture
                match self.value_for(cmd) {
                    Some(v) => response15(cmd, v),
                    None => Vec::new(),
                }
            } else {
                // écriture (valeur en [13])
                self.apply_write(cmd, frame[13]);
                Vec::new() // pas d'ACK simulé
            }
        }

        fn value_for(&self, cmd: u8) -> Option<u8> {
            Some(match CmdType::from_u8(cmd)? {
                CmdType::PowerState => self.power as u8,
                CmdType::Mode => self.mode as u8,
                CmdType::TargetTemp => {
                    if self.special == SpecialMode::EightDeg {
                        self.target.wrapping_add(16)
                    } else {
                        self.target
                    }
                }
                CmdType::RoomTemp => self.room as u8,
                CmdType::OutdoorTemp => self.outdoor as u8,
                CmdType::Fan => self.fan as u8,
                CmdType::Swing => self.swing as u8,
                CmdType::PowerSel => self.pwr as u8,
                CmdType::SpecialMode => self.special as u8,
                _ => return None,
            })
        }

        fn apply_write(&mut self, cmd: u8, v: u8) {
            let Some(c) = CmdType::from_u8(cmd) else {
                return;
            };
            match c {
                CmdType::PowerState => {
                    if let Some(s) = State::from_u8(v) {
                        self.power = s;
                    }
                }
                CmdType::Mode => {
                    if let Some(m) = Mode::from_u8(v) {
                        self.mode = m;
                    }
                }
                CmdType::TargetTemp => {
                    self.target = if self.special == SpecialMode::EightDeg {
                        v.wrapping_sub(16)
                    } else {
                        v
                    };
                }
                CmdType::Fan => {
                    if let Some(f) = Fan::from_u8(v) {
                        self.fan = f;
                    }
                }
                CmdType::Swing => {
                    if let Some(s) = Swing::from_u8(v) {
                        self.swing = s;
                    }
                }
                CmdType::PowerSel => {
                    if let Some(p) = PwrLevel::from_u8(v) {
                        self.pwr = p;
                    }
                }
                CmdType::SpecialMode => {
                    if let Some(sp) = SpecialMode::from_u8(v) {
                        self.special = sp;
                    }
                }
                _ => {}
            }
        }
    }

    /// Fabrique une réponse de longueur 15 (gabarit d'écriture : cmd [12], val [13]).
    fn response15(cmd: u8, value: u8) -> Vec<u8> {
        let mut f = vec![
            0x02, 0x00, 0x03, 0x10, 0x00, 0x00, 0x07, 0x01, 0x30, 0x01, 0x00, 0x02,
        ];
        f.push(cmd);
        f.push(value);
        f.push(checksum(&f));
        f
    }

    /// Fait tourner client ↔ unité pendant `steps` pas de 25 ms.
    fn drive(client: &mut Client, unit: &mut FakeUnit, now: &mut u32, steps: u32) {
        for _ in 0..steps {
            *now += 25;
            client.on_tick(*now);
            if let Some(tx) = client.poll_tx(*now) {
                let resp = unit.feed(&tx);
                for b in resp {
                    client.on_rx_byte(b, *now);
                }
            }
        }
    }

    #[test]
    fn full_handshake_then_online_reads_state() {
        let mut client = Client::new();
        let mut unit = FakeUnit::default();
        assert_eq!(client.phase(), Phase::Idle);

        client.start(0);
        let mut now = 0;
        drive(&mut client, &mut unit, &mut now, 400); // ~10 s simulées

        assert!(
            client.is_online(),
            "le client doit passer Online après le handshake"
        );
        let s = client.status();
        assert_eq!(s.power, Some(State::On));
        assert_eq!(s.mode, Some(Mode::Cool));
        assert_eq!(s.room_temp, Some(21));
        assert_eq!(s.target_temp, Some(24));
        assert_eq!(s.outdoor_temp, Some(8));
        assert_eq!(s.fan, Some(Fan::Auto));
        assert_eq!(s.pwr_level, Some(PwrLevel::Pct100));
    }

    #[test]
    fn set_mode_reaches_unit() {
        let mut client = Client::new();
        let mut unit = FakeUnit::default();
        client.start(0);
        let mut now = 0;
        drive(&mut client, &mut unit, &mut now, 400);
        assert!(client.is_online());

        client.set_mode(Mode::Heat);
        drive(&mut client, &mut unit, &mut now, 40);
        assert_eq!(unit.mode, Mode::Heat, "la commande doit atteindre l'unité");
    }

    #[test]
    fn set_target_temp_switches_to_frost_guard() {
        let mut client = Client::new();
        let mut unit = FakeUnit::default();
        client.start(0);
        let mut now = 0;
        drive(&mut client, &mut unit, &mut now, 400);

        // Consigne 8 °C → bascule en mode 8° + valeur décalée de +16 (=24) sur le fil.
        client.set_target_temp(8);
        drive(&mut client, &mut unit, &mut now, 40);
        assert_eq!(unit.special, SpecialMode::EightDeg);
        assert_eq!(unit.target, 8, "l'unité doit dé-offseter 24 → 8 °C");
    }

    #[test]
    fn json_command_reaches_unit() {
        let mut client = Client::new();
        let mut unit = FakeUnit::default();
        client.start(0);
        let mut now = 0;
        drive(&mut client, &mut unit, &mut now, 400);
        assert!(client.is_online());

        // Commande MQTT (JSON) → liste de Command → application.
        let cmds =
            crate::mqtt_payload::parse_command(r#"{"mode":"heat","target_temp":23,"fan":"high"}"#)
                .unwrap();
        for c in cmds {
            client.apply_command(c);
        }
        drive(&mut client, &mut unit, &mut now, 60);

        assert_eq!(unit.mode, Mode::Heat);
        assert_eq!(unit.target, 23);
        assert_eq!(unit.fan, Fan::High);
    }

    #[test]
    fn watchdog_rehandshakes_after_silence() {
        let mut client = Client::new();
        let mut unit = FakeUnit::default();
        client.start(0);
        let mut now = 0;
        drive(&mut client, &mut unit, &mut now, 400);
        assert!(client.is_online());

        // Silence prolongé (> WATCHDOG_MS) sans aucune trame reçue.
        now += WATCHDOG_MS + 1_000;
        client.on_tick(now);
        assert_eq!(
            client.phase(),
            Phase::Handshaking,
            "watchdog → re-handshake"
        );
        assert!(client.pending_tx() > 0);
    }

    #[test]
    fn does_not_emit_during_reception() {
        // Un octet partiel en cours (STX) doit bloquer l'émission.
        let mut client = Client::new();
        client.start(0);
        // Consomme la 1re trame dispo pour caler last_tx.
        let _ = client.poll_tx(1000);
        // Injecte un début de trame (STX) → réception en cours.
        client.on_rx_byte(0x02, 1000);
        assert!(
            client.poll_tx(2000).is_none(),
            "pas d'émission pendant une réception"
        );
    }
}
