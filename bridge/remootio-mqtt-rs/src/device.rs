//! Session WebSocket vers un boîtier Remootio : connexion, authentification,
//! keepalive PING/PONG, réception des events/réponses, envoi des commandes.
//! Reconnexion automatique en boucle (le process reste `up`, la session se
//! rétablit toute seule) — cohérent avec `SharedBus::reopen()` côté daly-bms.

use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

use crate::protocol::{self, EncryptedFrame};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Open,
    Close,
    Trigger,
    TriggerSecondary,
    Query,
}

#[derive(Debug, Clone)]
pub enum Event {
    Authenticated,
    Disconnected,
    /// État remonté par un événement `StateChange` (mouvement détecté) ou par
    /// la réponse à une action `QUERY` — les deux portent le même champ `state`.
    State(String),
}

pub struct DeviceConfig {
    pub name: String,
    pub ip: String,
    pub secret_key: [u8; 32],
    pub auth_key: [u8; 32],
    pub ping_interval: Duration,
}

/// Boucle de vie du device : (re)connecte indéfiniment, journalise les
/// coupures, attend avant de retenter. Ne retourne jamais (sauf process kill).
pub async fn run(
    cfg: DeviceConfig,
    mut cmd_rx: mpsc::Receiver<Command>,
    event_tx: mpsc::Sender<(String, Event)>,
) {
    loop {
        info!("[{}] connexion à ws://{}:8080/ ...", cfg.name, cfg.ip);
        match run_once(&cfg, &mut cmd_rx, &event_tx).await {
            Ok(()) => debug!("[{}] session terminée normalement", cfg.name),
            Err(e) => warn!("[{}] session interrompue: {e:#}", cfg.name),
        }
        let _ = event_tx.send((cfg.name.clone(), Event::Disconnected)).await;
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn run_once(
    cfg: &DeviceConfig,
    cmd_rx: &mut mpsc::Receiver<Command>,
    event_tx: &mpsc::Sender<(String, Event)>,
) -> anyhow::Result<()> {
    let url = format!("ws://{}:8080/", cfg.ip);
    let (ws_stream, _response) = tokio_tungstenite::connect_async(&url).await?;
    let (mut write, mut read) = ws_stream.split();
    info!("[{}] websocket connecté, authentification...", cfg.name);
    write
        .send(Message::Text(r#"{"type":"AUTH"}"#.into()))
        .await?;

    let mut session_key: Option<[u8; 32]> = None;
    let mut last_action_id: Option<u32> = None;
    let mut waiting_for_auth_query = false;
    let mut authenticated = false;
    let mut last_message_at = Instant::now();

    let ping_reply_timeout = cfg.ping_interval / 2;
    let mut ping_ticker = tokio::time::interval(cfg.ping_interval);
    ping_ticker.tick().await; // ignore le premier tick immédiat

    loop {
        tokio::select! {
            maybe_msg = read.next() => {
                let Some(msg) = maybe_msg else {
                    anyhow::bail!("websocket fermé par le boîtier");
                };
                let msg = msg?;
                match msg {
                    Message::Text(text) => {
                        last_message_at = Instant::now();
                        let needs_auth_query = handle_incoming(
                            &cfg.name,
                            &text,
                            cfg,
                            &mut session_key,
                            &mut last_action_id,
                            &mut waiting_for_auth_query,
                            &mut authenticated,
                            event_tx,
                        ).await;
                        if needs_auth_query {
                            // Le challenge vient d'établir la sessionKey : on doit envoyer
                            // une action QUERY pour finaliser l'authentification (le
                            // boîtier attend cette confirmation, sinon il ferme la session).
                            if let (Some(session_key), Some(last_id)) = (session_key, last_action_id) {
                                if let Err(e) = send_command(&mut write, &session_key, &cfg.auth_key, last_id, Command::Query).await {
                                    warn!("[{}] échec envoi QUERY d'authentification: {e:#}", cfg.name);
                                }
                            }
                        }
                    }
                    Message::Ping(_) | Message::Pong(_) => {
                        last_message_at = Instant::now();
                    }
                    Message::Close(_) => {
                        anyhow::bail!("websocket fermé par le boîtier (close frame)");
                    }
                    _ => {}
                }
            }
            _ = ping_ticker.tick() => {
                if last_message_at.elapsed() > cfg.ping_interval + ping_reply_timeout {
                    anyhow::bail!(
                        "aucun message reçu depuis {:?} (> {:?}), connexion considérée morte",
                        last_message_at.elapsed(), cfg.ping_interval + ping_reply_timeout
                    );
                }
                write.send(Message::Text(r#"{"type":"PING"}"#.into())).await?;
            }
            Some(cmd) = cmd_rx.recv() => {
                debug!("[{}] commande reçue: {:?}", cfg.name, cmd);
                if !authenticated {
                    warn!("[{}] commande {:?} ignorée : session non authentifiée", cfg.name, cmd);
                    continue;
                }
                match (session_key, last_action_id) {
                    (Some(session_key), Some(last_id)) => {
                        if let Err(e) = send_command(&mut write, &session_key, &cfg.auth_key, last_id, cmd).await {
                            warn!("[{}] échec envoi commande {:?}: {e:#}", cfg.name, cmd);
                        }
                    }
                    _ => warn!("[{}] commande {:?} ignorée : session_key/last_action_id absents malgré authenticated=true", cfg.name, cmd),
                }
            }
        }
    }
}

/// Traite un message texte reçu. Retourne `true` si l'appelant doit envoyer
/// une action `QUERY` pour finaliser l'authentification (le challenge vient
/// d'établir la `sessionKey`, mais l'envoi nécessite l'accès au `write` sink
/// que cette fonction n'a pas).
#[allow(clippy::too_many_arguments)]
async fn handle_incoming(
    name: &str,
    text: &str,
    cfg: &DeviceConfig,
    session_key: &mut Option<[u8; 32]>,
    last_action_id: &mut Option<u32>,
    waiting_for_auth_query: &mut bool,
    authenticated: &mut bool,
    event_tx: &mpsc::Sender<(String, Event)>,
) -> bool {
    let Ok(frame) = serde_json::from_str::<EncryptedFrame>(text) else {
        debug!("[{name}] frame non-ENCRYPTED ignorée: {text}");
        return false;
    };

    let aes_key = session_key.unwrap_or(cfg.secret_key);
    let decrypted = match protocol::decrypt_frame(&frame, &aes_key, &cfg.auth_key) {
        Ok(v) => v,
        Err(e) => {
            warn!("[{name}] déchiffrement/MAC invalide: {e}");
            return false;
        }
    };

    if let Some(challenge) = decrypted.get("challenge") {
        let Some(session_key_b64) = challenge.get("sessionKey").and_then(|v| v.as_str()) else {
            warn!("[{name}] challenge sans sessionKey");
            return false;
        };
        match protocol::decode_base64_key(session_key_b64) {
            Ok(key) => *session_key = Some(key),
            Err(e) => {
                warn!("[{name}] sessionKey invalide: {e}");
                return false;
            }
        }
        *last_action_id = challenge
            .get("initialActionId")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        *waiting_for_auth_query = true;
        info!("[{name}] challenge reçu, envoi de la QUERY d'authentification");
        return true;
    }

    if let Some(response) = decrypted.get("response") {
        if response.get("success").and_then(|v| v.as_bool()) == Some(false) {
            // Le boîtier a explicitement rejeté l'action (ex. TRIGGER_SECONDARY sur une
            // sortie qui n'est pas configurée en "free output" dans l'app Remootio :
            // ERR_INVALID_REQUEST) — un cas vécu, à faire remonter clairement.
            warn!("[{name}] action rejetée par le boîtier: {response}");
        } else {
            debug!("[{name}] réponse reçue: {response}");
        }
        if let Some(id) = response.get("id").and_then(|v| v.as_u64()) {
            let id = id as u32;
            match last_action_id {
                Some(prev) if *prev < id || (id == 0 && *prev == 0x7fff_ffff) => {
                    *last_action_id = Some(id)
                }
                None => *last_action_id = Some(id),
                _ => {}
            }
        }
        if response.get("type").and_then(|v| v.as_str()) == Some("QUERY") {
            if *waiting_for_auth_query {
                *waiting_for_auth_query = false;
                *authenticated = true;
                info!("[{name}] session authentifiée");
                let _ = event_tx
                    .send((name.to_string(), Event::Authenticated))
                    .await;
            }
            if let Some(state) = response.get("state").and_then(|v| v.as_str()) {
                let _ = event_tx
                    .send((name.to_string(), Event::State(state.to_string())))
                    .await;
            }
        }
        return false;
    }

    if let Some(event) = decrypted.get("event") {
        if event.get("type").and_then(|v| v.as_str()) == Some("StateChange") {
            if let Some(state) = event.get("state").and_then(|v| v.as_str()) {
                let _ = event_tx
                    .send((name.to_string(), Event::State(state.to_string())))
                    .await;
            }
        }
    }

    false
}

async fn send_command(
    write: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    session_key: &[u8; 32],
    auth_key: &[u8; 32],
    last_action_id: u32,
    cmd: Command,
) -> anyhow::Result<()> {
    let action_type = match cmd {
        Command::Open => "OPEN",
        Command::Close => "CLOSE",
        Command::Trigger => "TRIGGER",
        Command::TriggerSecondary => "TRIGGER_SECONDARY",
        Command::Query => "QUERY",
    };
    let next_id = protocol::next_action_id(last_action_id);
    let plaintext = protocol::action_json(action_type, next_id, None);
    let frame = protocol::build_encrypted_frame(session_key, auth_key, &plaintext)?;
    let text = serde_json::to_string(&frame)?;
    write.send(Message::Text(text)).await?;
    Ok(())
}
