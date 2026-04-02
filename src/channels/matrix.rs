//! Matrix channel via the Matrix Client-Server API.
//!
//! # Feature gates
//!
//! When the `matrix-sdk-channel` feature is enabled (default), the channel
//! uses the [`matrix_sdk`] crate for syncing.  The SDK handles room-state
//! bookkeeping, auto-join, to-device messages, and (once `e2e-encryption` is
//! added) transparent E2EE decryption.  State is persisted in a SQLite store
//! at `~/.ironclaw/matrix-sdk/` so the bot survives restarts without losing
//! its sync token or (later) its Olm session keys.
//!
//! When the feature is **disabled** the channel falls back to a raw
//! `reqwest`-based poll loop against `/_matrix/client/v3/sync`.  This path
//! has no external dependencies beyond those already in the binary, but it
//! cannot support E2EE.
//!
//! # Architecture (SDK path)
//!
//! ```text
//! MatrixChannel::start()
//!   → build matrix_sdk::Client (sqlite_store at sdk_store_path)
//!   → restore_session (access_token + device_id from secrets DB)
//!   → register SyncEventHandler → emits IncomingMessage via mpsc
//!   → spawn Client::sync() loop
//! MatrixChannel::respond()
//!   → sdk_client.get_room(room_id)?.send(RoomMessageEventContent)
//! MatrixChannel::send_status()
//!   → room.send_typing_notification() / room.send(text msg)
//! MatrixChannel::health_check()
//!   → GET /_matrix/client/v3/account/whoami (raw HTTP, lightweight)
//! ```
//!
//! # Architecture (raw poll path)
//!
//! ```text
//! MatrixChannel::start()
//!   → whoami (validate token, get bot user_id)
//!   → initial sync (prime next_batch)
//!   → spawn polling loop → yield IncomingMessage via mpsc channel
//! MatrixChannel::respond()
//!   → PUT /_matrix/client/v3/rooms/{room}/send/m.room.message/{txnId}
//! ```
//!
//! # Access control
//!
//! Both paths share the same access-control logic:
//!
//! - `owner_id` (if set): only that Matrix user ID is allowed; all others dropped.
//! - `dm_policy`:
//!   - `"open"` — accept all messages
//!   - `"allowlist"` — only senders in `allow_from` or pairing store
//!   - `"pairing"` (default) — same as allowlist, but sends a pairing prompt
//!     to unknown senders in private (2-member) rooms only
//! - Pairing prompts are never sent to group rooms (> 2 members).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use secrecy::ExposeSecret;
use tokio::sync::{RwLock, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::config::MatrixConfig;
use crate::error::ChannelError;
use crate::pairing::PairingStore;

const CHANNEL_NAME: &str = "matrix";

// ── Shared: access-control decision ──────────────────────────────────────────

/// Decision returned by `check_sender`.
enum SenderDecision {
    Allow,
    Deny,
    /// Denied — send pairing prompt to this room.
    Pair(String),
}

/// Check whether `allow_from` config or the pairing store allows `sender`.
async fn is_allowed_by_config_or_store(
    config: &MatrixConfig,
    sender: &str,
    pairing_store: &PairingStore,
) -> bool {
    let in_config = config
        .allow_from
        .iter()
        .any(|e| e.trim() == "*" || e.trim().to_lowercase() == sender.to_lowercase());
    if in_config {
        return true;
    }
    // Check whether the sender has been paired (approved) in the database.
    pairing_store
        .resolve_identity(CHANNEL_NAME, sender)
        .await
        .ok()
        .flatten()
        .is_some()
}

/// Determine if a message from `sender` in `room_id` should be processed.
async fn check_sender(
    config: &MatrixConfig,
    sender: &str,
    room_id: &str,
    pairing_store: &PairingStore,
) -> SenderDecision {
    if let Some(ref owner) = config.owner_id {
        if sender.to_lowercase() != owner.to_lowercase() {
            return SenderDecision::Deny;
        }
        return SenderDecision::Allow;
    }

    match config.dm_policy.as_str() {
        "open" => SenderDecision::Allow,
        "allowlist" => {
            if is_allowed_by_config_or_store(config, sender, pairing_store).await {
                SenderDecision::Allow
            } else {
                SenderDecision::Deny
            }
        }
        _ => {
            // "pairing" (default)
            if is_allowed_by_config_or_store(config, sender, pairing_store).await {
                SenderDecision::Allow
            } else {
                SenderDecision::Pair(room_id.to_string())
            }
        }
    }
}

// ── Shared helpers (used by both SDK and raw paths) ──────────────────────────

/// Extract `room_id` from message metadata, falling back to `thread_id`.
fn extract_room_id(msg: &IncomingMessage) -> Result<&str, ChannelError> {
    msg.metadata
        .get("room_id")
        .and_then(|v| v.as_str())
        .or(msg.thread_id.as_ref().map(|t| t.as_str()))
        .ok_or_else(|| ChannelError::MissingRoutingTarget {
            name: CHANNEL_NAME.to_string(),
            reason: "no room_id in message metadata".to_string(),
        })
}

/// Build the `conversation_context` map from message metadata.
fn build_conversation_context(metadata: &serde_json::Value) -> HashMap<String, String> {
    let mut ctx = HashMap::new();
    if let Some(room_id) = metadata.get("room_id").and_then(|v| v.as_str()) {
        ctx.insert("matrix_room_id".to_string(), room_id.to_string());
    }
    if let Some(homeserver) = metadata.get("homeserver").and_then(|v| v.as_str()) {
        ctx.insert("matrix_homeserver".to_string(), homeserver.to_string());
    }
    ctx
}

/// Format a `StatusUpdate` into a text message to send, or `None` for
/// variants that don't produce text (e.g. `Thinking`, which triggers a
/// typing indicator instead).
fn format_status_message(status: &StatusUpdate) -> Option<String> {
    match status {
        StatusUpdate::ApprovalNeeded {
            tool_name,
            description,
            ..
        } => Some(format!(
            "⏳ Approval needed for `{}`: {}",
            tool_name, description
        )),
        StatusUpdate::AuthRequired {
            extension_name,
            instructions,
            ..
        } => {
            if let Some(inst) = instructions {
                Some(format!(
                    "🔑 Auth required for `{}`: {}",
                    extension_name, inst
                ))
            } else {
                Some(format!("🔑 Auth required for `{}`.", extension_name))
            }
        }
        StatusUpdate::AuthCompleted {
            extension_name,
            success,
            message,
        } => {
            if *success {
                Some(format!(
                    "✅ Auth completed for `{}`: {}",
                    extension_name, message
                ))
            } else {
                Some(format!(
                    "❌ Auth failed for `{}`: {}",
                    extension_name, message
                ))
            }
        }
        _ => None,
    }
}

/// Send a pairing prompt to an unknown sender in a DM room.
///
/// Checks the room member count first (only sends in DMs, ≤ 2 members).
/// Used by both the SDK event handler (via `tokio::spawn`) and the raw
/// poll path's inline handler.
async fn handle_pairing(
    homeserver: &str,
    access_token: &secrecy::SecretString,
    sender: &str,
    room_id: &str,
    pairing_store: &PairingStore,
    client: &reqwest::Client,
) {
    // Check member count — only send pairing prompts in DMs.
    let members_url = format!(
        "{}/_matrix/client/v3/rooms/{}/joined_members",
        homeserver,
        urlencoding::encode(room_id)
    );
    let auth = format!("Bearer {}", access_token.expose_secret());
    let is_dm = match client
        .get(&members_url)
        .header("Authorization", &auth)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| {
                v.get("joined")
                    .and_then(|j| j.as_object())
                    .map(|m| m.len() <= 2)
            })
            .unwrap_or(false),
        _ => false,
    };

    if !is_dm {
        debug!(
            sender,
            room_id, "Matrix: unknown sender in group room, silently dropping"
        );
        return;
    }

    match pairing_store
        .upsert_request(
            CHANNEL_NAME,
            sender,
            Some(serde_json::json!({ "room_id": room_id })),
        )
        .await
    {
        Ok(result) if result.created => {
            let msg = format!(
                "To pair with this bot, run: `ironclaw pairing approve matrix {}`",
                result.code
            );
            let txn_id = format!("ironclaw-{}", Uuid::new_v4().as_u128());
            let url = format!(
                "{}/_matrix/client/v3/rooms/{}/send/m.room.message/{}",
                homeserver,
                urlencoding::encode(room_id),
                urlencoding::encode(&txn_id),
            );
            let body = serde_json::json!({ "msgtype": "m.text", "body": msg });
            if let Err(e) = client
                .put(&url)
                .header("Authorization", &auth)
                .json(&body)
                .send()
                .await
            {
                warn!(sender, room_id, error = %e, "Matrix: failed to send pairing reply");
            }
        }
        Ok(_) => {}
        Err(e) => warn!(sender, error = %e, "Matrix: pairing upsert failed"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SDK-backed implementation  (`matrix-sdk-channel` feature)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "matrix-sdk-channel")]
mod sdk {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    use matrix_sdk::{
        Client,
        authentication::matrix::MatrixSession,
        config::SyncSettings,
        room::Room,
        ruma::{
            OwnedUserId,
            api::client::filter::FilterDefinition,
            events::room::{
                encrypted::OriginalSyncRoomEncryptedEvent,
                member::StrippedRoomMemberEvent,
                message::{MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent},
            },
        },
    };

    /// Secrets store reference — used to persist `device_id` after first login.
    pub type SecretsStoreRef = Option<Arc<dyn crate::secrets::SecretsStore + Send + Sync>>;

    // ── MatrixChannel ─────────────────────────────────────────────────────────

    /// Native Matrix channel backed by `matrix_sdk`.
    pub struct MatrixChannel {
        config: MatrixConfig,
        /// SDK client — populated by `start()`, used by `respond()` etc.
        sdk_client: RwLock<Option<Client>>,
        /// Shared HTTP client for raw requests (health checks, whoami, etc.).
        http_client: reqwest::Client,
        /// Pairing store for DM pairing (guest access control).
        pairing_store: Arc<PairingStore>,
        /// Secrets store for persisting the device ID across restarts.
        secrets_store: SecretsStoreRef,
        /// Owner user ID from `config.owner_id`, resolved to the DB's user_id
        /// for secret writes.  Falls back to `"default"`.
        secrets_user_id: String,
    }

    impl MatrixChannel {
        /// Create a new SDK-backed `MatrixChannel`.
        pub fn new(
            config: MatrixConfig,
            db: Option<Arc<dyn crate::db::Database>>,
            ownership_cache: Arc<crate::ownership::OwnershipCache>,
        ) -> Result<Self, ChannelError> {
            validate_homeserver(&config.homeserver)?;
            let http_client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(|e| ChannelError::StartupFailed {
                    name: CHANNEL_NAME.to_string(),
                    reason: format!("Failed to build HTTP client: {e}"),
                })?;
            let pairing_store = if let Some(db) = db {
                Arc::new(PairingStore::new(db, ownership_cache))
            } else {
                Arc::new(PairingStore::new_noop())
            };
            Ok(Self {
                config,
                sdk_client: RwLock::new(None),
                http_client,
                pairing_store,
                secrets_store: None,
                secrets_user_id: "default".to_string(),
            })
        }

        /// Attach a secrets store so the channel can persist the device ID.
        ///
        /// Called from `main.rs` after construction.  The `user_id` should be
        /// the IronClaw owner ID used for all other secrets in the same store.
        pub fn with_secrets(
            mut self,
            store: Arc<dyn crate::secrets::SecretsStore + Send + Sync>,
            user_id: &str,
        ) -> Self {
            self.secrets_store = Some(store);
            self.secrets_user_id = user_id.to_string();
            self
        }

        // ── SDK client construction ──────────────────────────────────────────

        async fn build_client(&self) -> Result<Client, ChannelError> {
            let store_path = &self.config.sdk_store_path;
            std::fs::create_dir_all(store_path).map_err(|e| ChannelError::StartupFailed {
                name: CHANNEL_NAME.to_string(),
                reason: format!(
                    "failed to create SDK store dir {}: {e}",
                    store_path.display()
                ),
            })?;

            let builder = Client::builder()
                .homeserver_url(&self.config.homeserver)
                .sqlite_store(store_path, None);

            // Enable E2EE when the matrix-e2ee feature is on.
            // NOTE: auto_enable_cross_signing requires the SDK to have the
            // password for UIA (User-Interactive Auth). Since we restore from
            // a stored token, the SDK can't bootstrap cross-signing. The bot
            // device will show as unverified — cosmetic only, doesn't block
            // E2EE. To verify, use Element: Settings → Sessions → verify the
            // bot device, or store the password and use login_username().
            #[cfg(feature = "matrix-e2ee")]
            let builder = {
                use matrix_sdk::encryption::EncryptionSettings;
                info!("Matrix: E2EE enabled (matrix-e2ee feature)");
                builder.with_encryption_settings(EncryptionSettings::default())
            };

            #[cfg(not(feature = "matrix-e2ee"))]
            info!("Matrix: E2EE disabled (build with matrix-e2ee feature to enable)");

            builder
                .build()
                .await
                .map_err(|e| ChannelError::StartupFailed {
                    name: CHANNEL_NAME.to_string(),
                    reason: format!("matrix-sdk client build failed: {e}"),
                })
        }

        /// Build a `MatrixSession` from the access token and an explicit device ID.
        ///
        /// `device_id` should come from `whoami` (the real device ID the token
        /// belongs to) or from the secrets DB if we persisted it previously.
        /// If both are `None` the SDK will generate a new device — this should
        /// not happen in practice because `whoami` always returns one.
        fn make_session_with_device(
            &self,
            user_id: OwnedUserId,
            device_id: Option<&str>,
        ) -> MatrixSession {
            use matrix_sdk::SessionTokens;

            let device_id: matrix_sdk::ruma::OwnedDeviceId =
                device_id.unwrap_or("IRONCLAW_UNKNOWN").into();

            MatrixSession {
                meta: matrix_sdk::SessionMeta { user_id, device_id },
                tokens: SessionTokens {
                    access_token: self.config.access_token.expose_secret().to_string(),
                    refresh_token: None,
                },
            }
        }

        /// Persist the device ID to the secrets store so we reuse it on restart.
        async fn persist_device_id(&self, device_id: &str) {
            let Some(ref store) = self.secrets_store else {
                debug!("Matrix: no secrets store attached — skipping device_id persistence");
                return;
            };
            use crate::secrets::CreateSecretParams;
            let params = CreateSecretParams::new("matrix_device_id", device_id);
            match store.create(&self.secrets_user_id, params).await {
                Ok(_) => {
                    debug!("Matrix: persisted device_id '{}' to secrets DB", device_id);
                }
                Err(e) => {
                    warn!("Matrix: failed to persist device_id '{}': {e}", device_id);
                }
            }
        }

        // ── Sending ──────────────────────────────────────────────────────────

        /// Get a joined room by ID from the SDK client.
        async fn get_room(&self, room_id_str: &str) -> Option<Room> {
            use matrix_sdk::ruma::RoomId;
            let client_guard = self.sdk_client.read().await;
            let client = client_guard.as_ref()?;
            let room_id = RoomId::parse(room_id_str).ok()?;
            client.get_room(&room_id)
        }

        /// Send a plain-text message to a room.
        async fn send_text_to_room(
            &self,
            room_id_str: &str,
            text: &str,
        ) -> Result<(), ChannelError> {
            let room =
                self.get_room(room_id_str)
                    .await
                    .ok_or_else(|| ChannelError::SendFailed {
                        name: CHANNEL_NAME.to_string(),
                        reason: format!("room '{}' not found in SDK client", room_id_str),
                    })?;
            let content = RoomMessageEventContent::text_plain(text);
            room.send(content)
                .await
                .map_err(|e| ChannelError::SendFailed {
                    name: CHANNEL_NAME.to_string(),
                    reason: format!("send to room '{}': {e}", room_id_str),
                })?;
            Ok(())
        }

        // ── DM room resolution (for broadcast) ───────────────────────────────

        async fn resolve_dm_room(&self, user_id_str: &str) -> Result<String, ChannelError> {
            use matrix_sdk::ruma::UserId;

            let client_guard = self.sdk_client.read().await;
            let client = client_guard
                .as_ref()
                .ok_or_else(|| ChannelError::SendFailed {
                    name: CHANNEL_NAME.to_string(),
                    reason: "SDK client not initialised".to_string(),
                })?;

            // Look for an existing DM room with this user in the SDK's room list.
            let target_uid = UserId::parse(user_id_str).map_err(|e| ChannelError::SendFailed {
                name: CHANNEL_NAME.to_string(),
                reason: format!("invalid user ID '{}': {e}", user_id_str),
            })?;

            for room in client.joined_rooms() {
                // A DM room has exactly 2 members and is_direct.
                if room.is_direct().await.unwrap_or(false) && room.joined_members_count() == 2 {
                    // Check if the target user is in this room.
                    if let Ok(Some(_)) = room.get_member(&target_uid).await {
                        return Ok(room.room_id().to_string());
                    }
                }
            }

            // No existing room — create one.
            use matrix_sdk::ruma::api::client::room::create_room::v3::Request as CreateRoomRequest;
            let mut req = CreateRoomRequest::new();
            req.is_direct = true;
            req.invite = vec![target_uid];

            let resp = client
                .create_room(req)
                .await
                .map_err(|e| ChannelError::SendFailed {
                    name: CHANNEL_NAME.to_string(),
                    reason: format!("createRoom for '{}': {e}", user_id_str),
                })?;

            Ok(resp.room_id().to_string())
        }
    }

    // ── Channel trait (SDK path) ──────────────────────────────────────────────

    #[async_trait]
    impl Channel for MatrixChannel {
        fn name(&self) -> &str {
            CHANNEL_NAME
        }

        async fn start(&self) -> Result<MessageStream, ChannelError> {
            // Build the SDK client.
            let client = self.build_client().await?;

            // Log token prefix/suffix to aid debugging (never log the full token).
            {
                let tok = self.config.access_token.expose_secret();
                let len = tok.len();
                let preview = if len >= 8 {
                    format!("{}...{}", &tok[..4], &tok[len - 4..])
                } else {
                    format!("<{}chars>", len)
                };
                debug!(token_preview = %preview, homeserver = %self.config.homeserver, "Matrix: starting with token");
            }

            // Derive user_id and device_id from the access token via whoami.
            // whoami always returns the device ID the token was issued for — this
            // is the only source of truth on first boot before we've persisted it.
            let (user_id, whoami_device_id) = {
                let whoami_url = format!(
                    "{}/_matrix/client/v3/account/whoami",
                    self.config.homeserver
                );
                let resp = self
                    .http_client
                    .get(&whoami_url)
                    .header(
                        "Authorization",
                        format!("Bearer {}", self.config.access_token.expose_secret()),
                    )
                    .send()
                    .await
                    .map_err(|e| ChannelError::StartupFailed {
                        name: CHANNEL_NAME.to_string(),
                        reason: format!("whoami request failed: {e}"),
                    })?;

                let status = resp.status();
                if status == reqwest::StatusCode::FORBIDDEN
                    || status == reqwest::StatusCode::UNAUTHORIZED
                {
                    let body = resp.text().await.unwrap_or_else(|_| "<no body>".into());
                    return Err(ChannelError::AuthFailed {
                        name: CHANNEL_NAME.to_string(),
                        reason: format!("whoami 401/403: {body}"),
                    });
                }
                if !status.is_success() {
                    return Err(ChannelError::StartupFailed {
                        name: CHANNEL_NAME.to_string(),
                        reason: format!("whoami returned {status}"),
                    });
                }

                #[derive(serde::Deserialize)]
                struct WhoAmI {
                    user_id: String,
                    device_id: Option<String>,
                }
                let body: WhoAmI = resp.json().await.map_err(|e| ChannelError::StartupFailed {
                    name: CHANNEL_NAME.to_string(),
                    reason: format!("whoami parse failed: {e}"),
                })?;
                (body.user_id, body.device_id)
            };

            let uid: OwnedUserId = matrix_sdk::ruma::UserId::parse(&user_id).map_err(|e| {
                ChannelError::StartupFailed {
                    name: CHANNEL_NAME.to_string(),
                    reason: format!("invalid user_id from whoami '{user_id}': {e}"),
                }
            })?;

            // Resolve device ID: config (from secrets DB) > whoami response > none.
            // whoami always returns the device ID the token was issued for, so this
            // is the authoritative source on first boot before we've persisted it.
            let resolved_device_id: Option<String> =
                self.config.device_id.clone().or(whoami_device_id);

            info!(
                user_id = %uid,
                device_id = ?resolved_device_id,
                homeserver = %self.config.homeserver,
                dm_policy = %self.config.dm_policy,
                auto_join = self.config.auto_join,
                sdk_store = %self.config.sdk_store_path.display(),
                "Matrix channel starting (SDK mode)"
            );

            // Restore the session using the stored access token + device ID.
            let session = self.make_session_with_device(uid.clone(), resolved_device_id.as_deref());
            let device_id = session.meta.device_id.to_string();

            client
                .restore_session(session)
                .await
                .map_err(|e| ChannelError::StartupFailed {
                    name: CHANNEL_NAME.to_string(),
                    reason: format!("restore_session failed: {e}"),
                })?;

            info!(device_id = %device_id, "Matrix: session restored");

            // If device_id was not previously persisted in the secrets DB, do it now.
            if self.config.device_id.is_none() {
                self.persist_device_id(&device_id).await;
            }

            // Store the client for use in respond() / send_status() / etc.
            *self.sdk_client.write().await = Some(client.clone());

            // ── Event handlers ────────────────────────────────────────────────

            let (tx, rx) = mpsc::channel::<IncomingMessage>(64);

            // Flipped to true after the priming sync_once completes.
            // Event handlers check this and discard messages until it's set,
            // preventing replay of history on cold-start or sync-token loss.
            let ready = Arc::new(AtomicBool::new(false));

            // Clone what the handler needs.
            let config = self.config.clone();
            let bot_user_id = uid.clone();
            let tx_msg = tx.clone();

            // Text message handler.
            client.add_event_handler({
                let config = config.clone();
                let bot_user_id = bot_user_id.clone();
                let tx = tx_msg.clone();
                let ready = Arc::clone(&ready);
                let pairing_store = Arc::clone(&self.pairing_store);
                let http_client = self.http_client.clone();
                move |ev: OriginalSyncRoomMessageEvent, room: Room| {
                    let config = config.clone();
                    let bot_user_id = bot_user_id.clone();
                    let http_client = http_client.clone();
                    let tx = tx.clone();
                    let ready = Arc::clone(&ready);
                    let pairing_store = Arc::clone(&pairing_store);
                    async move {
                        // Discard events from the priming sync.
                        if !ready.load(Ordering::Acquire) {
                            return;
                        }

                        let sender = ev.sender.to_string();

                        // Skip our own messages.
                        if sender == bot_user_id.as_str() {
                            return;
                        }

                        let room_id = room.room_id().to_string();

                        // Extract text body.
                        let text = match &ev.content.msgtype {
                            MessageType::Text(text_content) => text_content.body.clone(),
                            other => {
                                debug!(
                                    room_id,
                                    sender,
                                    msgtype = ?other.msgtype(),
                                    "Matrix: skipping non-text message"
                                );
                                return;
                            }
                        };

                        let event_id = ev.event_id.to_string();

                        // Access control.
                        match check_sender(&config, &sender, &room_id, &pairing_store).await {
                            SenderDecision::Allow => {}
                            SenderDecision::Deny => {
                                debug!(sender, room_id, "Matrix: sender denied, dropping");
                                return;
                            }
                            SenderDecision::Pair(pair_room) => {
                                // Pairing is handled via a raw HTTP call below.
                                // We don't have Arc<Self> here, so use
                                // PairingStore directly and send via raw HTTP.
                                let config2 = config.clone();
                                let sender2 = sender.clone();
                                let ps = Arc::clone(&pairing_store);
                                let hc = http_client.clone();
                                tokio::spawn(async move {
                                    handle_pairing(
                                        &config2.homeserver,
                                        &config2.access_token,
                                        &sender2,
                                        &pair_room,
                                        &ps,
                                        &hc,
                                    )
                                    .await;
                                });
                                return;
                            }
                        }

                        info!(
                            sender = %sender,
                            room_id = %room_id,
                            event_id = %event_id,
                            content_len = text.len(),
                            "Matrix: message received"
                        );

                        let msg = IncomingMessage::new(CHANNEL_NAME, &sender, &text)
                            .with_sender_id(sender.clone())
                            .with_thread(room_id.clone())
                            .with_metadata(serde_json::json!({
                                "room_id": room_id,
                                "event_id": event_id,
                                "homeserver": config.homeserver,
                            }));

                        if tx.send(msg).await.is_err() {
                            debug!("Matrix: message receiver dropped");
                        }
                    }
                }
            });

            // Log undecryptable encrypted events — helps diagnose missing room keys.
            client.add_event_handler({
                let ready = Arc::clone(&ready);
                move |ev: OriginalSyncRoomEncryptedEvent, room: Room| {
                    let ready = Arc::clone(&ready);
                    async move {
                        if !ready.load(Ordering::Acquire) {
                            return;
                        }
                        warn!(
                            sender = %ev.sender,
                            room_id = %room.room_id(),
                            event_id = %ev.event_id,
                            "Matrix: received m.room.encrypted event that could not be decrypted \
                             (missing room key — sender may need to re-share keys with this device)"
                        );
                    }
                }
            });

            // Auto-join: handle room invites.
            if self.config.auto_join {
                let join_config = self.config.clone();
                let join_ps = Arc::clone(&self.pairing_store);
                client.add_event_handler(
                    move |ev: StrippedRoomMemberEvent, room: Room, client: Client| {
                        let join_config = join_config.clone();
                        let join_ps = Arc::clone(&join_ps);
                        async move {
                            // Only react to invites addressed to us.
                            let Some(my_user_id) = client.user_id() else {
                                return;
                            };
                            if ev.state_key != *my_user_id {
                                return;
                            }
                            let room_id = room.room_id().to_string();
                            let inviter = ev.sender.to_string();

                            // Check the inviter against dm_policy / allow_from
                            // before joining.  "open" joins all; "pairing" and
                            // "allowlist" require the inviter to be in allow_from
                            // (or owner_id).
                            match check_sender(&join_config, &inviter, &room_id, &join_ps).await {
                                SenderDecision::Allow => {}
                                SenderDecision::Deny => {
                                    debug!(
                                        room_id,
                                        inviter, "Matrix: ignoring invite from denied sender"
                                    );
                                    return;
                                }
                                SenderDecision::Pair(_) => {
                                    // In pairing mode, still join — message handling
                                    // will send the pairing prompt.
                                }
                            }

                            info!(room_id, inviter, "Matrix: received invite, joining room");
                            if let Err(e) = room.join().await {
                                warn!(room_id, error = %e, "Matrix: failed to join invited room");
                            } else {
                                info!(room_id, "Matrix: joined room");
                            }
                        }
                    },
                );
            }

            // ── Sync loop ─────────────────────────────────────────────────────
            // If the SDK store has an existing sync position (normal restart),
            // resume from that position — messages that arrived while offline
            // are delivered normally.  Set `ready` immediately so nothing is
            // suppressed.
            //
            // If the store was just wiped (fresh onboard), do a priming
            // `sync_once` first with `ready = false` so historical room
            // backfill is discarded.  Then flip `ready` and start the live
            // loop.
            //
            // The setup wizard writes a `.fresh` marker file after wiping the
            // store.  We use that marker to distinguish the two cases instead
            // of probing directory contents (which is unreliable because
            // `build_client()` creates SQLite files before any sync happens).

            let fresh_marker = self.config.sdk_store_path.join(".fresh");
            let is_fresh_store = fresh_marker.exists();

            let sync_client = client.clone();
            let ready_for_spawn = Arc::clone(&ready);
            tokio::spawn(async move {
                let filter = FilterDefinition::with_lazy_loading();
                let settings = SyncSettings::default().filter(filter.into());

                if is_fresh_store {
                    // Fresh start — prime the token and discard backfill.
                    info!(
                        "Matrix: fresh store detected (.fresh marker), performing priming sync to discard backfill"
                    );
                    match sync_client.sync_once(settings.clone()).await {
                        Ok(resp) => {
                            info!(
                                next_batch = %resp.next_batch,
                                "Matrix: priming sync complete, starting live sync"
                            );
                        }
                        Err(e) => {
                            warn!(error = %e, "Matrix: priming sync failed, continuing anyway");
                        }
                    }
                    // Remove the marker so subsequent restarts resume normally.
                    if let Err(e) = std::fs::remove_file(&fresh_marker) {
                        warn!(error = %e, "Matrix: failed to remove .fresh marker");
                    }
                    ready_for_spawn.store(true, Ordering::Release);
                } else {
                    // Resume from stored position — deliver offline messages.
                    info!("Matrix: resuming from stored sync position, starting live sync");
                    ready_for_spawn.store(true, Ordering::Release);
                }

                let mut backoff_secs = 1u64;
                loop {
                    info!("Matrix: starting SDK sync loop");
                    match sync_client.sync(settings.clone()).await {
                        Ok(()) => {
                            // sync() returned Ok — homeserver requested a stop.
                            info!("Matrix: sync loop exited cleanly");
                            break;
                        }
                        Err(e) => {
                            error!(
                                error = %e,
                                backoff_secs,
                                "Matrix: sync loop terminated, restarting after backoff"
                            );
                            tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                            backoff_secs = (backoff_secs * 2).min(60);
                        }
                    }
                }
            });

            Ok(Box::pin(ReceiverStream::new(rx)))
        }

        async fn respond(
            &self,
            msg: &IncomingMessage,
            response: OutgoingResponse,
        ) -> Result<(), ChannelError> {
            let room_id = extract_room_id(msg)?;
            self.send_text_to_room(room_id, &response.content).await
        }

        async fn send_status(
            &self,
            status: StatusUpdate,
            metadata: &serde_json::Value,
        ) -> Result<(), ChannelError> {
            let room_id = match metadata.get("room_id").and_then(|v| v.as_str()) {
                Some(r) => r.to_string(),
                None => return Ok(()),
            };

            if let StatusUpdate::Thinking(_) = &status {
                let Some(room) = self.get_room(&room_id).await else {
                    return Ok(());
                };
                if let Err(e) = room.typing_notice(true).await {
                    debug!(error = %e, "Matrix: failed to send typing indicator");
                }
            } else if let Some(text) = format_status_message(&status) {
                if let Err(e) = self.send_text_to_room(&room_id, &text).await {
                    debug!(error = %e, "Matrix: failed to send status notification");
                }
            }
            Ok(())
        }

        async fn broadcast(
            &self,
            user_id: &str,
            response: OutgoingResponse,
        ) -> Result<(), ChannelError> {
            let room_id = self.resolve_dm_room(user_id).await?;
            self.send_text_to_room(&room_id, &response.content).await
        }

        async fn health_check(&self) -> Result<(), ChannelError> {
            // Lightweight whoami ping — same as the raw path.
            let client_guard = self.sdk_client.read().await;
            if client_guard.is_none() {
                return Err(ChannelError::HealthCheckFailed {
                    name: format!("{}: SDK client not yet initialised", CHANNEL_NAME),
                });
            }
            // Use the shared HTTP client for a lightweight whoami ping.
            let url = format!(
                "{}/_matrix/client/v3/account/whoami",
                self.config.homeserver
            );
            let resp = self
                .http_client
                .get(&url)
                .header(
                    "Authorization",
                    format!("Bearer {}", self.config.access_token.expose_secret()),
                )
                .send()
                .await
                .map_err(|e| ChannelError::HealthCheckFailed {
                    name: format!("{}: {e}", CHANNEL_NAME),
                })?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(ChannelError::HealthCheckFailed {
                    name: format!("{}: HTTP {}", CHANNEL_NAME, resp.status()),
                })
            }
        }

        fn conversation_context(&self, metadata: &serde_json::Value) -> HashMap<String, String> {
            build_conversation_context(metadata)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Raw reqwest poll implementation  (no `matrix-sdk-channel` feature)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(not(feature = "matrix-sdk-channel"))]
mod raw {
    use super::*;

    /// Backoff delays for poll errors (seconds): 1, 2, 5, 10, 30.
    const BACKOFF_SECS: &[u64] = &[1, 2, 5, 10, 30];

    // ── JSON shapes for Matrix Client-Server API ──────────────────────────────

    #[derive(serde::Deserialize)]
    struct SyncResponse {
        next_batch: String,
        #[serde(default)]
        rooms: SyncRooms,
    }

    #[derive(serde::Deserialize, Default)]
    struct SyncRooms {
        #[serde(default)]
        join: HashMap<String, JoinedRoom>,
        #[serde(default)]
        invite: HashMap<String, serde_json::Value>,
    }

    #[derive(serde::Deserialize)]
    struct JoinedRoom {
        timeline: Option<Timeline>,
    }

    #[derive(serde::Deserialize)]
    struct Timeline {
        #[serde(default)]
        events: Vec<TimelineEvent>,
    }

    #[derive(serde::Deserialize)]
    struct TimelineEvent {
        #[serde(rename = "type")]
        event_type: String,
        sender: Option<String>,
        content: Option<serde_json::Value>,
        event_id: Option<String>,
    }

    #[derive(serde::Deserialize)]
    struct JoinedMembersResponse {
        joined: HashMap<String, serde_json::Value>,
    }

    // ── MatrixChannel ─────────────────────────────────────────────────────────

    /// Native Matrix channel using raw Matrix Client-Server API (no SDK).
    pub struct MatrixChannel {
        config: MatrixConfig,
        client: reqwest::Client,
        /// Pairing store for DM pairing (guest access control).
        pairing_store: Arc<PairingStore>,
        bot_user_id: RwLock<String>,
        next_batch: RwLock<Option<String>>,
    }

    impl MatrixChannel {
        pub fn new(
            config: MatrixConfig,
            db: Option<Arc<dyn crate::db::Database>>,
            ownership_cache: Arc<crate::ownership::OwnershipCache>,
        ) -> Result<Self, ChannelError> {
            validate_homeserver(&config.homeserver)?;
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(40))
                .build()
                .map_err(|e| ChannelError::StartupFailed {
                    name: CHANNEL_NAME.to_string(),
                    reason: format!("Failed to build HTTP client: {e}"),
                })?;
            let pairing_store = if let Some(db) = db {
                Arc::new(PairingStore::new(db, ownership_cache))
            } else {
                Arc::new(PairingStore::new_noop())
            };
            Ok(Self {
                config,
                client,
                pairing_store,
                bot_user_id: RwLock::new(String::new()),
                next_batch: RwLock::new(None),
            })
        }

        fn auth_header(&self) -> String {
            format!("Bearer {}", self.config.access_token.expose_secret())
        }

        /// Maximum response body size for raw HTTP calls (50 MB).
        const MAX_RESPONSE_BYTES: usize = 50 * 1024 * 1024;

        async fn matrix_get(&self, url: &str) -> Result<serde_json::Value, ChannelError> {
            let resp = self
                .client
                .get(url)
                .header("Authorization", self.auth_header())
                .send()
                .await
                .map_err(|e| ChannelError::Http(format!("GET {url}: {e}")))?;
            let status = resp.status();
            if status == 429 {
                return Err(ChannelError::RateLimited {
                    name: CHANNEL_NAME.to_string(),
                });
            }
            if !status.is_success() {
                let body = resp
                    .text()
                    .await
                    .unwrap_or_else(|_| "<no body>".to_string());
                return Err(ChannelError::Http(format!("GET {url} → {status}: {body}")));
            }
            // Pre-screen with Content-Length when available to avoid buffering
            // oversized responses.
            if let Some(len) = resp.content_length() {
                if len as usize > MAX_RESPONSE_BYTES {
                    return Err(ChannelError::Http(format!(
                        "GET {url}: response too large ({len} bytes, Content-Length)"
                    )));
                }
            }
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| ChannelError::Http(format!("GET {url} read: {e}")))?;
            if bytes.len() > MAX_RESPONSE_BYTES {
                return Err(ChannelError::Http(format!(
                    "GET {url}: response too large ({} bytes)",
                    bytes.len()
                )));
            }
            serde_json::from_slice(&bytes)
                .map_err(|e| ChannelError::Http(format!("GET {url} parse: {e}")))
        }

        async fn matrix_post(
            &self,
            url: &str,
            body: &serde_json::Value,
        ) -> Result<serde_json::Value, ChannelError> {
            let resp = self
                .client
                .post(url)
                .header("Authorization", self.auth_header())
                .json(body)
                .send()
                .await
                .map_err(|e| ChannelError::Http(format!("POST {url}: {e}")))?;
            let status = resp.status();
            if status == 429 {
                return Err(ChannelError::RateLimited {
                    name: CHANNEL_NAME.to_string(),
                });
            }
            if !status.is_success() {
                let body_text = resp
                    .text()
                    .await
                    .unwrap_or_else(|_| "<no body>".to_string());
                return Err(ChannelError::Http(format!(
                    "POST {url} → {status}: {body_text}"
                )));
            }
            resp.json()
                .await
                .map_err(|e| ChannelError::Http(format!("POST {url} parse: {e}")))
        }

        async fn matrix_put(
            &self,
            url: &str,
            body: &serde_json::Value,
        ) -> Result<serde_json::Value, ChannelError> {
            let resp = self
                .client
                .put(url)
                .header("Authorization", self.auth_header())
                .json(body)
                .send()
                .await
                .map_err(|e| ChannelError::Http(format!("PUT {url}: {e}")))?;
            let status = resp.status();
            if status == 429 {
                return Err(ChannelError::RateLimited {
                    name: CHANNEL_NAME.to_string(),
                });
            }
            if !status.is_success() {
                let body_text = resp
                    .text()
                    .await
                    .unwrap_or_else(|_| "<no body>".to_string());
                return Err(ChannelError::Http(format!(
                    "PUT {url} → {status}: {body_text}"
                )));
            }
            resp.json()
                .await
                .map_err(|e| ChannelError::Http(format!("PUT {url} parse: {e}")))
        }

        async fn send_text(
            &self,
            room_id: &str,
            text: &str,
            reply_to_event_id: Option<&str>,
        ) -> Result<(), ChannelError> {
            let txn_id = format!("ironclaw-{}", Uuid::new_v4().as_u128());
            let url = format!(
                "{}/_matrix/client/v3/rooms/{}/send/m.room.message/{}",
                self.config.homeserver,
                urlencoding::encode(room_id),
                urlencoding::encode(&txn_id),
            );
            let mut body = serde_json::json!({ "msgtype": "m.text", "body": text });
            if let Some(event_id) = reply_to_event_id {
                body["m.relates_to"] =
                    serde_json::json!({ "m.in_reply_to": { "event_id": event_id } });
            }
            self.matrix_put(&url, &body).await?;
            debug!(room_id, content_len = text.len(), "Matrix: message sent");
            Ok(())
        }

        async fn handle_pairing_request(&self, sender: &str, room_id: &str) {
            handle_pairing(
                &self.config.homeserver,
                &self.config.access_token,
                sender,
                room_id,
                &self.pairing_store,
                &self.client,
            )
            .await;
        }

        async fn poll_once(
            self: &Arc<Self>,
        ) -> Result<(String, Vec<IncomingMessage>), ChannelError> {
            let since = self.next_batch.read().await.clone();
            let bot_user_id = self.bot_user_id.read().await.clone();
            let url = if let Some(ref token) = since {
                format!(
                    "{}/_matrix/client/v3/sync?since={}&timeout=0",
                    self.config.homeserver,
                    urlencoding::encode(token)
                )
            } else {
                format!(
                    "{}/_matrix/client/v3/sync?timeout=0",
                    self.config.homeserver
                )
            };

            let sync: SyncResponse = {
                let val = self.matrix_get(&url).await?;
                serde_json::from_value(val)
                    .map_err(|e| ChannelError::InvalidMessage(format!("sync parse: {e}")))?
            };

            let mut messages = Vec::new();
            let invite_count = sync.rooms.invite.len();
            let join_count = sync.rooms.join.len();
            if invite_count > 0 || join_count > 0 {
                debug!(
                    next_batch = %sync.next_batch,
                    joined_rooms = join_count,
                    pending_invites = invite_count,
                    "Matrix: sync received"
                );
            }

            if self.config.auto_join {
                for (room_id, invite_state) in &sync.rooms.invite {
                    // Try to extract the inviter from invite_state events so we
                    // can check allow_from before joining.
                    let inviter = invite_state
                        .pointer("/invite_state/events")
                        .and_then(|evts| evts.as_array())
                        .and_then(|evts| {
                            evts.iter().find_map(|e| {
                                if e.get("type")?.as_str()? == "m.room.member"
                                    && e.get("content")
                                        .and_then(|c| c.get("membership"))
                                        .and_then(|m| m.as_str())
                                        == Some("invite")
                                {
                                    e.get("sender")?.as_str().map(String::from)
                                } else {
                                    None
                                }
                            })
                        });

                    if let Some(ref inviter) = inviter {
                        match check_sender(&self.config, inviter, room_id, &self.pairing_store)
                            .await
                        {
                            SenderDecision::Allow | SenderDecision::Pair(_) => {}
                            SenderDecision::Deny => {
                                debug!(
                                    room_id,
                                    inviter, "Matrix: ignoring invite from denied sender"
                                );
                                continue;
                            }
                        }
                    }

                    let join_url = format!(
                        "{}/_matrix/client/v3/rooms/{}/join",
                        self.config.homeserver,
                        urlencoding::encode(room_id)
                    );
                    match self.matrix_post(&join_url, &serde_json::json!({})).await {
                        Ok(_) => info!(room_id, "Matrix: auto-joined room"),
                        Err(ChannelError::RateLimited { .. }) => {
                            warn!(room_id, "Matrix: rate limited joining room")
                        }
                        Err(e) => warn!(room_id, error = %e, "Matrix: failed to join invited room"),
                    }
                }
            }

            for (room_id, room) in sync.rooms.join {
                let events = room.timeline.map(|t| t.events).unwrap_or_default();
                for event in events {
                    if event.event_type != "m.room.message" {
                        continue;
                    }
                    let sender = match event.sender {
                        Some(s) => s,
                        None => continue,
                    };
                    if !bot_user_id.is_empty() && sender == bot_user_id {
                        continue;
                    }
                    let content = match event.content {
                        Some(c) => c,
                        None => continue,
                    };
                    let msgtype = content
                        .get("msgtype")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if msgtype != "m.text" {
                        debug!(
                            room_id,
                            sender, msgtype, "Matrix: skipping non-text message"
                        );
                        continue;
                    }
                    let text = match content.get("body").and_then(|v| v.as_str()) {
                        Some(t) => t.to_string(),
                        None => continue,
                    };
                    let event_id = event.event_id.clone().unwrap_or_default();

                    match check_sender(&self.config, &sender, &room_id, &self.pairing_store).await {
                        SenderDecision::Allow => {}
                        SenderDecision::Deny => {
                            debug!(sender, room_id, "Matrix: sender denied, dropping");
                            continue;
                        }
                        SenderDecision::Pair(ref pair_room) => {
                            let arc_self = Arc::clone(self);
                            let pair_room = pair_room.clone();
                            let sender_clone = sender.clone();
                            tokio::spawn(async move {
                                arc_self
                                    .handle_pairing_request(&sender_clone, &pair_room)
                                    .await;
                            });
                            continue;
                        }
                    }

                    info!(
                        sender = %sender,
                        room_id = %room_id,
                        event_id = %event_id,
                        content_len = text.len(),
                        "Matrix: message received"
                    );
                    let msg = IncomingMessage::new(CHANNEL_NAME, &sender, &text)
                        .with_sender_id(sender.clone())
                        .with_thread(room_id.clone())
                        .with_metadata(serde_json::json!({
                            "room_id": room_id,
                            "event_id": event_id,
                            "homeserver": self.config.homeserver,
                        }));
                    messages.push(msg);
                }
            }

            Ok((sync.next_batch, messages))
        }

        async fn poll_loop(self: Arc<Self>, tx: mpsc::Sender<IncomingMessage>) {
            let interval = Duration::from_secs(u64::from(self.config.poll_interval_secs));
            let mut backoff_idx: usize = 0;
            debug!(
                interval_secs = self.config.poll_interval_secs,
                "Matrix: poll loop started"
            );
            loop {
                match self.poll_once().await {
                    Ok((next_batch, messages)) => {
                        if backoff_idx > 0 {
                            info!("Matrix: poll recovered after backoff");
                        }
                        backoff_idx = 0;
                        *self.next_batch.write().await = Some(next_batch);
                        for msg in messages {
                            if tx.send(msg).await.is_err() {
                                debug!("Matrix: message receiver dropped, stopping poll loop");
                                return;
                            }
                        }
                        tokio::time::sleep(interval).await;
                    }
                    Err(ChannelError::RateLimited { .. }) => {
                        let delay = BACKOFF_SECS
                            .get(backoff_idx)
                            .copied()
                            .unwrap_or(*BACKOFF_SECS.last().unwrap());
                        warn!(delay, "Matrix: rate limited, backing off");
                        backoff_idx = (backoff_idx + 1).min(BACKOFF_SECS.len() - 1);
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                    }
                    Err(e) => {
                        let delay = BACKOFF_SECS
                            .get(backoff_idx)
                            .copied()
                            .unwrap_or(*BACKOFF_SECS.last().unwrap());
                        error!(error = %e, delay, "Matrix: poll error, retrying");
                        backoff_idx = (backoff_idx + 1).min(BACKOFF_SECS.len() - 1);
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                    }
                }
            }
        }

        async fn resolve_dm_room(&self, user_id: &str) -> Result<String, ChannelError> {
            // m.direct is account data on the *bot's* account, not the target user.
            let bot_id = self.bot_user_id.read().await;
            let direct_url = format!(
                "{}/_matrix/client/v3/user/{}/account_data/m.direct",
                self.config.homeserver,
                urlencoding::encode(&bot_id),
            );
            if let Ok(data) = self.matrix_get(&direct_url).await {
                if let Some(rooms) = data.get(user_id).and_then(|v| v.as_array()) {
                    if let Some(room_id) = rooms.first().and_then(|v| v.as_str()) {
                        return Ok(room_id.to_string());
                    }
                }
            }
            let create_url = format!("{}/_matrix/client/v3/createRoom", self.config.homeserver);
            let body = serde_json::json!({
                "is_direct": true,
                "invite": [user_id],
                "preset": "trusted_private_chat",
            });
            let resp = self.matrix_post(&create_url, &body).await?;
            resp.get("room_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| ChannelError::SendFailed {
                    name: CHANNEL_NAME.to_string(),
                    reason: format!("createRoom response missing room_id for user {user_id}"),
                })
        }
    }

    #[async_trait]
    impl Channel for MatrixChannel {
        fn name(&self) -> &str {
            CHANNEL_NAME
        }

        async fn start(&self) -> Result<MessageStream, ChannelError> {
            let whoami_url = format!(
                "{}/_matrix/client/v3/account/whoami",
                self.config.homeserver
            );
            let resp = self
                .client
                .get(&whoami_url)
                .header("Authorization", self.auth_header())
                .send()
                .await
                .map_err(|e| ChannelError::StartupFailed {
                    name: CHANNEL_NAME.to_string(),
                    reason: format!("whoami request failed: {e}"),
                })?;
            let status = resp.status();
            if status == reqwest::StatusCode::FORBIDDEN
                || status == reqwest::StatusCode::UNAUTHORIZED
            {
                return Err(ChannelError::AuthFailed {
                    name: CHANNEL_NAME.to_string(),
                    reason: "Invalid access token — check MATRIX_ACCESS_TOKEN".to_string(),
                });
            }
            if !status.is_success() {
                return Err(ChannelError::StartupFailed {
                    name: CHANNEL_NAME.to_string(),
                    reason: format!("whoami returned {status}"),
                });
            }
            #[derive(serde::Deserialize)]
            struct WhoAmIBody {
                user_id: String,
            }
            let body: WhoAmIBody = resp.json().await.map_err(|e| ChannelError::StartupFailed {
                name: CHANNEL_NAME.to_string(),
                reason: format!("whoami parse failed: {e}"),
            })?;

            *self.bot_user_id.write().await = body.user_id.clone();
            info!(
                user_id = %body.user_id,
                homeserver = %self.config.homeserver,
                dm_policy = %self.config.dm_policy,
                poll_interval_secs = self.config.poll_interval_secs,
                auto_join = self.config.auto_join,
                "Matrix channel starting (raw poll mode)"
            );

            info!("Matrix: performing initial sync to prime next_batch token");
            let sync_url = format!(
                "{}/_matrix/client/v3/sync?timeout=0",
                self.config.homeserver
            );
            let init_sync = self
                .client
                .get(&sync_url)
                .header("Authorization", self.auth_header())
                .send()
                .await
                .map_err(|e| ChannelError::StartupFailed {
                    name: CHANNEL_NAME.to_string(),
                    reason: format!("initial sync failed: {e}"),
                })?;
            let init_status = init_sync.status();
            if init_status.is_success() {
                match init_sync.json::<SyncResponse>().await {
                    Ok(body) => {
                        *self.next_batch.write().await = Some(body.next_batch.clone());
                        info!(
                            next_batch = %body.next_batch,
                            joined_rooms = body.rooms.join.len(),
                            pending_invites = body.rooms.invite.len(),
                            "Matrix: initial sync complete"
                        );
                    }
                    Err(e) => warn!("Matrix: failed to parse initial sync: {e}"),
                }
            } else {
                warn!(status = %init_status, "Matrix: initial sync returned non-success status");
            }

            info!(
                poll_interval_secs = self.config.poll_interval_secs,
                "Matrix: starting poll loop"
            );
            let (tx, rx) = mpsc::channel::<IncomingMessage>(64);
            let arc_self = Arc::new(MatrixChannel {
                config: self.config.clone(),
                client: self.client.clone(),
                pairing_store: Arc::clone(&self.pairing_store),
                bot_user_id: RwLock::new(self.bot_user_id.read().await.clone()),
                next_batch: RwLock::new(self.next_batch.read().await.clone()),
            });
            tokio::spawn(async move { arc_self.poll_loop(tx).await });
            Ok(Box::pin(ReceiverStream::new(rx)))
        }

        async fn respond(
            &self,
            msg: &IncomingMessage,
            response: OutgoingResponse,
        ) -> Result<(), ChannelError> {
            let room_id = extract_room_id(msg)?;
            let reply_to = msg
                .metadata
                .get("event_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty());
            self.send_text(room_id, &response.content, reply_to).await
        }

        async fn send_status(
            &self,
            status: StatusUpdate,
            metadata: &serde_json::Value,
        ) -> Result<(), ChannelError> {
            let room_id = match metadata.get("room_id").and_then(|v| v.as_str()) {
                Some(r) => r.to_string(),
                None => return Ok(()),
            };

            if let StatusUpdate::Thinking(_) = &status {
                let bot_user_id = self.bot_user_id.read().await.clone();
                let url = format!(
                    "{}/_matrix/client/v3/rooms/{}/typing/{}",
                    self.config.homeserver,
                    urlencoding::encode(&room_id),
                    urlencoding::encode(&bot_user_id),
                );
                let body = serde_json::json!({ "typing": true, "timeout": 30000 });
                if let Err(e) = self.matrix_put(&url, &body).await {
                    debug!(error = %e, "Matrix: failed to send typing indicator");
                }
            } else if let Some(text) = format_status_message(&status) {
                if let Err(e) = self.send_text(&room_id, &text, None).await {
                    debug!(error = %e, "Matrix: failed to send status notification");
                }
            }
            Ok(())
        }

        async fn broadcast(
            &self,
            user_id: &str,
            response: OutgoingResponse,
        ) -> Result<(), ChannelError> {
            let room_id = self.resolve_dm_room(user_id).await?;
            self.send_text(&room_id, &response.content, None).await
        }

        async fn health_check(&self) -> Result<(), ChannelError> {
            let url = format!(
                "{}/_matrix/client/v3/account/whoami",
                self.config.homeserver
            );
            let resp = self
                .client
                .get(&url)
                .header("Authorization", self.auth_header())
                .timeout(Duration::from_secs(10))
                .send()
                .await
                .map_err(|e| ChannelError::HealthCheckFailed {
                    name: format!("{}: {}", CHANNEL_NAME, e),
                })?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(ChannelError::HealthCheckFailed {
                    name: format!("{}: HTTP {}", CHANNEL_NAME, resp.status()),
                })
            }
        }

        fn conversation_context(&self, metadata: &serde_json::Value) -> HashMap<String, String> {
            build_conversation_context(metadata)
        }
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

fn validate_homeserver(homeserver: &str) -> Result<(), ChannelError> {
    if !homeserver.starts_with("https://") && !homeserver.starts_with("http://") {
        return Err(ChannelError::StartupFailed {
            name: CHANNEL_NAME.to_string(),
            reason: format!(
                "MATRIX_HOMESERVER must be a full URL (https://...), got: {}",
                homeserver
            ),
        });
    }
    Ok(())
}

// ── Re-export the right MatrixChannel ─────────────────────────────────────────

#[cfg(feature = "matrix-sdk-channel")]
pub use sdk::MatrixChannel;

#[cfg(not(feature = "matrix-sdk-channel"))]
pub use raw::MatrixChannel;

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    fn test_config() -> MatrixConfig {
        MatrixConfig {
            homeserver: "https://matrix.org".to_string(),
            access_token: SecretString::from("syt_test_token"),
            device_id: None,
            allow_from: vec![],
            dm_policy: "pairing".to_string(),
            owner_id: None,
            poll_interval_secs: 5,
            auto_join: true,
            display_name: None,
            sdk_store_path: std::path::PathBuf::from("/tmp/test-matrix-sdk"),
        }
    }

    fn noop_store() -> PairingStore {
        PairingStore::new_noop()
    }

    #[test]
    fn test_channel_name() {
        let oc = Arc::new(crate::ownership::OwnershipCache::new());
        let ch = MatrixChannel::new(test_config(), None, oc).unwrap();
        assert_eq!(ch.name(), "matrix");
    }

    #[test]
    fn test_invalid_homeserver_rejected() {
        let mut cfg = test_config();
        cfg.homeserver = "matrix.org".to_string();
        let oc = Arc::new(crate::ownership::OwnershipCache::new());
        assert!(MatrixChannel::new(cfg, None, oc).is_err());
    }

    #[test]
    fn test_http_homeserver_allowed() {
        let mut cfg = test_config();
        cfg.homeserver = "http://localhost:8448".to_string();
        let oc = Arc::new(crate::ownership::OwnershipCache::new());
        assert!(MatrixChannel::new(cfg, None, oc).is_ok());
    }

    #[tokio::test]
    async fn test_owner_restriction_allows_owner() {
        let mut cfg = test_config();
        let ps = noop_store();
        cfg.owner_id = Some("@alice:matrix.org".to_string());
        assert!(matches!(
            check_sender(&cfg, "@alice:matrix.org", "!room:matrix.org", &ps).await,
            SenderDecision::Allow
        ));
    }

    #[tokio::test]
    async fn test_owner_restriction_denies_others() {
        let mut cfg = test_config();
        let ps = noop_store();
        cfg.owner_id = Some("@alice:matrix.org".to_string());
        assert!(matches!(
            check_sender(&cfg, "@bob:matrix.org", "!room:matrix.org", &ps).await,
            SenderDecision::Deny
        ));
    }

    #[tokio::test]
    async fn test_open_policy_allows_all() {
        let mut cfg = test_config();
        let ps = noop_store();
        cfg.dm_policy = "open".to_string();
        assert!(matches!(
            check_sender(&cfg, "@stranger:matrix.org", "!room:matrix.org", &ps).await,
            SenderDecision::Allow
        ));
    }

    #[tokio::test]
    async fn test_allowlist_policy_denies_unlisted() {
        let mut cfg = test_config();
        let ps = noop_store();
        cfg.dm_policy = "allowlist".to_string();
        cfg.allow_from = vec!["@alice:matrix.org".to_string()];
        assert!(matches!(
            check_sender(&cfg, "@bob:matrix.org", "!room:matrix.org", &ps).await,
            SenderDecision::Deny
        ));
    }

    #[tokio::test]
    async fn test_allowlist_policy_allows_listed() {
        let mut cfg = test_config();
        let ps = noop_store();
        cfg.dm_policy = "allowlist".to_string();
        cfg.allow_from = vec!["@alice:matrix.org".to_string()];
        assert!(matches!(
            check_sender(&cfg, "@alice:matrix.org", "!room:matrix.org", &ps).await,
            SenderDecision::Allow
        ));
    }

    #[tokio::test]
    async fn test_wildcard_allow_from_allows_all() {
        let mut cfg = test_config();
        let ps = noop_store();
        cfg.dm_policy = "allowlist".to_string();
        cfg.allow_from = vec!["*".to_string()];
        assert!(matches!(
            check_sender(&cfg, "@anyone:matrix.org", "!room:matrix.org", &ps).await,
            SenderDecision::Allow
        ));
    }

    #[tokio::test]
    async fn test_pairing_policy_triggers_pair_for_unknown() {
        let cfg = test_config();
        let ps = noop_store();
        assert!(matches!(
            check_sender(&cfg, "@stranger:matrix.org", "!room:matrix.org", &ps).await,
            SenderDecision::Pair(_)
        ));
    }

    #[tokio::test]
    async fn test_config_case_insensitive_allow_from() {
        let mut cfg = test_config();
        let ps = noop_store();
        cfg.dm_policy = "allowlist".to_string();
        cfg.allow_from = vec!["@Alice:Matrix.ORG".to_string()];
        assert!(matches!(
            check_sender(&cfg, "@alice:matrix.org", "!room:matrix.org", &ps).await,
            SenderDecision::Allow
        ));
    }

    #[test]
    fn test_conversation_context_includes_room_and_homeserver() {
        let oc = Arc::new(crate::ownership::OwnershipCache::new());
        let ch = MatrixChannel::new(test_config(), None, oc).unwrap();
        let meta = serde_json::json!({
            "room_id": "!xyz:matrix.org",
            "homeserver": "https://matrix.org"
        });
        let ctx = ch.conversation_context(&meta);
        assert_eq!(
            ctx.get("matrix_room_id").map(|s| s.as_str()),
            Some("!xyz:matrix.org")
        );
        assert_eq!(
            ctx.get("matrix_homeserver").map(|s| s.as_str()),
            Some("https://matrix.org")
        );
    }

    #[cfg(not(feature = "matrix-sdk-channel"))]
    #[test]
    fn test_sync_response_parses_invite() {
        use super::raw::*;
        // Ensure the raw JSON shapes compile in test mode.
        let json = serde_json::json!({
            "next_batch": "s123",
            "rooms": { "invite": { "!room:matrix.org": {} } }
        });
        let sync: SyncResponse = serde_json::from_value(json).unwrap();
        assert_eq!(sync.next_batch, "s123");
        assert!(sync.rooms.invite.contains_key("!room:matrix.org"));
    }
}
