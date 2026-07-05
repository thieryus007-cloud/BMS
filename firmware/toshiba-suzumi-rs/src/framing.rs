//! Accumulateur de trames en réception (octet par octet).
//!
//! Reproduit la logique incrémentale de `validate_message_` / `handle_rx_byte_`
//! du firmware d'origine (§6.9), **sans notion de temps** : le time-out de
//! réception (`RECEIVE_TIMEOUT_MS`) est géré par l'appelant via [`FrameAccumulator::reset`].
//! Pur, testable sur host.

use crate::protocol::{validate_frame, ProtocolError, STX};

/// Garde-fou : au-delà de cette taille de buffer sans trame valide, on
/// resynchronise (aucune trame SUZUMI ne dépasse 24 octets).
const MAX_BUFFER: usize = 32;

/// Résultat de l'ajout d'un octet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rx {
    /// Trame incomplète : continuer à accumuler.
    Incomplete,
    /// Trame complète **et validée** (checksum OK).
    Frame(Vec<u8>),
    /// Trame complète mais **checksum invalide** (buffer réinitialisé).
    Error(ProtocolError),
}

/// Assembleur de trames. Voir [`FrameAccumulator::push`].
#[derive(Debug, Default)]
pub struct FrameAccumulator {
    buf: Vec<u8>,
}

impl FrameAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Vide le buffer (à appeler sur time-out de réception, `RECEIVE_TIMEOUT_MS`).
    pub fn reset(&mut self) {
        self.buf.clear();
    }

    /// Aucun octet en cours d'accumulation ? (le firmware n'émet une commande que
    /// si `rx_message_` est vide).
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Ajoute un octet reçu et indique l'état de l'assemblage.
    pub fn push(&mut self, byte: u8) -> Rx {
        self.buf.push(byte);
        let at = self.buf.len() - 1;

        // [0] doit être l'en-tête STX ; sinon on jette l'octet parasite.
        if at == 0 {
            if self.buf[0] != STX {
                self.buf.clear();
            }
            return Rx::Incomplete;
        }
        if at < 2 {
            return Rx::Incomplete;
        }
        // Les trames de statut ont [2]==0x03. Sinon on attend (resync par timeout
        // ou garde-fou de taille).
        if self.buf[2] != 0x03 {
            return self.guard();
        }
        if at <= 5 {
            return Rx::Incomplete;
        }
        // Index du checksum déduit de [6] : cks_idx = 6 + data[6] + 1.
        let cks_idx = 6 + self.buf[6] as usize + 1;
        if at < cks_idx {
            return self.guard();
        }
        // at == cks_idx → trame complète.
        let frame = self.buf[..=cks_idx].to_vec();
        self.buf.clear();
        match validate_frame(&frame) {
            Ok(()) => Rx::Frame(frame),
            Err(e) => Rx::Error(e),
        }
    }

    /// Resynchronise si le buffer grossit anormalement (trame malformée).
    fn guard(&mut self) -> Rx {
        if self.buf.len() > MAX_BUFFER {
            self.buf.clear();
        }
        Rx::Incomplete
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{build_read, build_write, CmdType};

    /// Pousse tous les octets d'une trame et renvoie le dernier résultat.
    fn feed(acc: &mut FrameAccumulator, frame: &[u8]) -> Rx {
        let mut last = Rx::Incomplete;
        for &b in frame {
            last = acc.push(b);
        }
        last
    }

    #[test]
    fn assembles_a_valid_frame() {
        let mut acc = FrameAccumulator::new();
        let f = build_write(CmdType::Mode, 66);
        assert_eq!(feed(&mut acc, &f), Rx::Frame(f.clone()));
        assert!(acc.is_empty(), "buffer vidé après une trame complète");
    }

    #[test]
    fn drops_leading_garbage_then_syncs() {
        let mut acc = FrameAccumulator::new();
        // Octets parasites avant l'en-tête STX.
        assert_eq!(acc.push(0xAA), Rx::Incomplete);
        assert_eq!(acc.push(0x00), Rx::Incomplete);
        assert!(acc.is_empty());
        // Puis une vraie trame passe.
        let f = build_read(CmdType::RoomTemp);
        assert_eq!(feed(&mut acc, &f), Rx::Frame(f));
    }

    #[test]
    fn reports_checksum_error_and_resets() {
        let mut acc = FrameAccumulator::new();
        let mut f = build_write(CmdType::TargetTemp, 22);
        *f.last_mut().unwrap() ^= 0xFF; // corrompt le checksum
        assert!(matches!(
            feed(&mut acc, &f),
            Rx::Error(ProtocolError::ChecksumMismatch { .. })
        ));
        assert!(acc.is_empty());
    }

    #[test]
    fn two_frames_back_to_back() {
        let mut acc = FrameAccumulator::new();
        let f1 = build_read(CmdType::RoomTemp);
        let f2 = build_write(CmdType::Fan, 65);
        assert_eq!(feed(&mut acc, &f1), Rx::Frame(f1));
        assert_eq!(feed(&mut acc, &f2), Rx::Frame(f2));
    }

    #[test]
    fn oversized_garbage_resyncs() {
        let mut acc = FrameAccumulator::new();
        // STX puis [2]!=0x03 et un flot d'octets → doit resynchroniser (pas de panique).
        acc.push(STX);
        acc.push(0x00);
        acc.push(0x99); // [2] != 0x03
        for _ in 0..40 {
            let _ = acc.push(0x55);
        }
        assert!(
            acc.is_empty(),
            "resynchronisation après dépassement de taille"
        );
    }
}
