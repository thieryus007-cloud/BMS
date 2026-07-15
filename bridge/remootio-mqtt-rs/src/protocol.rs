//! Couche crypto pure du protocole Remootio (frames `ENCRYPTED`) — testable
//! sur host, sans réseau. Portage 1:1 de `remootio-api-client` (Node.js) :
//! AES-256-CBC (PKCS7) pour le payload, HMAC-SHA256 sur `{iv,payload}` pour le MAC.
//!
//! Clé utilisée : `ApiSecretKey` tant que la session n'est pas authentifiée
//! (réponse au frame `AUTH`, qui contient le `challenge.sessionKey`) ; ensuite
//! `ApiSessionKey` (reçu en base64 dans le challenge) pour tous les échanges
//! suivants. `ApiAuthKey` sert uniquement au calcul du MAC, jamais au chiffrement.

use aes::Aes256;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type Aes256CbcEnc = cbc::Encryptor<Aes256>;
type Aes256CbcDec = cbc::Decryptor<Aes256>;
type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("MAC invalide : le frame ne provient pas d'une session authentifiée avec ces clés")]
    MacMismatch,
    #[error("base64 invalide : {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("clé/IV de longueur incorrecte")]
    InvalidLength,
    #[error("JSON invalide : {0}")]
    Json(#[from] serde_json::Error),
    #[error("hex invalide : {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("échec du déchiffrement (padding invalide)")]
    Decrypt,
    #[error("frame reçu n'est pas de type ENCRYPTED")]
    NotEncrypted,
}

/// `frame.data` d'un frame `ENCRYPTED` — l'ordre des champs (iv puis payload)
/// est significatif : c'est exactement l'objet sur lequel le MAC est calculé,
/// et `serde` sérialise toujours dans l'ordre de déclaration des champs.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EncryptedData {
    pub iv: String,
    pub payload: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EncryptedFrame {
    #[serde(rename = "type")]
    pub frame_type: String,
    pub data: EncryptedData,
    pub mac: String,
}

pub fn decode_hex_key(hex_key: &str) -> Result<[u8; 32], ProtocolError> {
    let bytes = hex::decode(hex_key.trim())?;
    bytes.try_into().map_err(|_| ProtocolError::InvalidLength)
}

pub fn decode_base64_key(b64_key: &str) -> Result<[u8; 32], ProtocolError> {
    let bytes = B64.decode(b64_key)?;
    bytes.try_into().map_err(|_| ProtocolError::InvalidLength)
}

fn mac_over_data(data: &EncryptedData, auth_key: &[u8; 32]) -> Result<String, ProtocolError> {
    let data_json = serde_json::to_string(data)?;
    let mut mac = HmacSha256::new_from_slice(auth_key).expect("clé HMAC de longueur valide");
    mac.update(data_json.as_bytes());
    Ok(B64.encode(mac.finalize().into_bytes()))
}

/// Vérifie le MAC puis déchiffre `frame.data.payload`. `aes_key` doit être
/// `ApiSecretKey` (pré-auth) ou `ApiSessionKey` (post-auth) selon l'état de la session.
pub fn decrypt_frame(
    frame: &EncryptedFrame,
    aes_key: &[u8; 32],
    auth_key: &[u8; 32],
) -> Result<serde_json::Value, ProtocolError> {
    if frame.frame_type != "ENCRYPTED" {
        return Err(ProtocolError::NotEncrypted);
    }

    let expected_mac = mac_over_data(&frame.data, auth_key)?;
    if expected_mac != frame.mac {
        return Err(ProtocolError::MacMismatch);
    }

    let iv = B64.decode(&frame.data.iv)?;
    let iv: [u8; 16] = iv.try_into().map_err(|_| ProtocolError::InvalidLength)?;
    let mut buf = B64.decode(&frame.data.payload)?;

    let plaintext = Aes256CbcDec::new(aes_key.into(), &iv.into())
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|_| ProtocolError::Decrypt)?;

    Ok(serde_json::from_slice(plaintext)?)
}

/// Construit un frame `ENCRYPTED` à partir d'un payload JSON en clair (déjà
/// sérialisé en `String`, ex. `{"action":{"type":"QUERY","id":42}}`).
/// Toujours chiffré avec `ApiSessionKey` (jamais `ApiSecretKey` : on ne peut
/// envoyer de commande qu'une fois authentifié).
pub fn build_encrypted_frame(
    session_key: &[u8; 32],
    auth_key: &[u8; 32],
    plaintext_json: &str,
) -> Result<EncryptedFrame, ProtocolError> {
    let mut iv = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut iv);

    let ciphertext = Aes256CbcEnc::new(session_key.into(), &iv.into())
        .encrypt_padded_vec_mut::<Pkcs7>(plaintext_json.as_bytes());

    let data = EncryptedData {
        iv: B64.encode(iv),
        payload: B64.encode(ciphertext),
    };
    let mac = mac_over_data(&data, auth_key)?;

    Ok(EncryptedFrame {
        frame_type: "ENCRYPTED".to_string(),
        data,
        mac,
    })
}

/// Construit le JSON en clair d'une action (`{"action":{"type":...,"id":...}}`),
/// avec `duration` (en minutes) optionnel pour les variantes "hold".
pub fn action_json(action_type: &str, action_id: u32, duration_mins: Option<u32>) -> String {
    match duration_mins {
        Some(d) => {
            format!(r#"{{"action":{{"type":"{action_type}","id":{action_id},"duration":{d}}}}}"#)
        }
        None => format!(r#"{{"action":{{"type":"{action_type}","id":{action_id}}}}}"#),
    }
}

/// Prochain `id` d'action à utiliser, avec le rebouclage à 0 propre au protocole
/// Remootio (`(lastActionId + 1) % 0x7FFFFFFF`).
pub fn next_action_id(last_action_id: u32) -> u32 {
    (last_action_id + 1) % 0x7fff_ffff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_keys() -> ([u8; 32], [u8; 32]) {
        let secret: [u8; 32] = [0x11; 32];
        let auth: [u8; 32] = [0x22; 32];
        (secret, auth)
    }

    #[test]
    fn roundtrip_encrypt_decrypt() {
        let (session_key, auth_key) = test_keys();
        let plaintext = r#"{"action":{"type":"QUERY","id":1}}"#;

        let frame = build_encrypted_frame(&session_key, &auth_key, plaintext).unwrap();
        assert_eq!(frame.frame_type, "ENCRYPTED");

        let decrypted = decrypt_frame(&frame, &session_key, &auth_key).unwrap();
        assert_eq!(decrypted["action"]["type"], "QUERY");
        assert_eq!(decrypted["action"]["id"], 1);
    }

    #[test]
    fn mac_mismatch_rejected_with_wrong_auth_key() {
        let (session_key, auth_key) = test_keys();
        let wrong_auth_key: [u8; 32] = [0x33; 32];
        let plaintext = r#"{"action":{"type":"TRIGGER","id":5}}"#;

        let frame = build_encrypted_frame(&session_key, &auth_key, plaintext).unwrap();
        let result = decrypt_frame(&frame, &session_key, &wrong_auth_key);
        assert!(matches!(result, Err(ProtocolError::MacMismatch)));
    }

    #[test]
    fn wrong_session_key_fails_to_decrypt_or_parse() {
        let (session_key, auth_key) = test_keys();
        let wrong_session_key: [u8; 32] = [0x44; 32];
        let plaintext = r#"{"action":{"type":"TRIGGER","id":5}}"#;

        let frame = build_encrypted_frame(&session_key, &auth_key, plaintext).unwrap();
        // Le MAC ne dépend que de auth_key (correct ici) donc il passe la vérification ;
        // mais le déchiffrement AES avec la mauvaise clé donne un padding/JSON invalide.
        let result = decrypt_frame(&frame, &wrong_session_key, &auth_key);
        assert!(result.is_err());
    }

    #[test]
    fn next_action_id_wraps_at_0x7fffffff() {
        assert_eq!(next_action_id(0x7fff_fffe), 0x7fff_ffff % 0x7fff_ffff);
        assert_eq!(next_action_id(0), 1);
    }

    #[test]
    fn action_json_without_duration() {
        assert_eq!(
            action_json("QUERY", 42, None),
            r#"{"action":{"type":"QUERY","id":42}}"#
        );
    }

    #[test]
    fn action_json_with_duration() {
        assert_eq!(
            action_json("TRIGGER_SECONDARY", 7, Some(3)),
            r#"{"action":{"type":"TRIGGER_SECONDARY","id":7,"duration":3}}"#
        );
    }

    #[test]
    fn decode_hex_key_rejects_wrong_length() {
        assert!(matches!(
            decode_hex_key("abcd"),
            Err(ProtocolError::InvalidLength)
        ));
    }

    #[test]
    fn decode_hex_key_accepts_64_char_hexstring() {
        let key = "11".repeat(32);
        assert_eq!(decode_hex_key(&key).unwrap(), [0x11u8; 32]);
    }
}
