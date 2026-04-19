//! XMPP channel with OMEMO-aware inbound handling.
//!
//! Connects to an XMPP server via tokio-xmpp, listens for incoming messages,
//! and routes replies back over the same XMPP channel.

mod config;
pub mod omemo;

use std::collections::{HashMap, HashSet, VecDeque};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use lru::LruCache;
use secrecy::ExposeSecret;
use tokio::sync::{RwLock, mpsc};
use uuid::Uuid;
use xmpp_parsers::data_forms::{DataForm, DataFormType, Field, FieldType};
use xmpp_parsers::disco::{DiscoInfoQuery, DiscoInfoResult};
use xmpp_parsers::eme::ExplicitMessageEncryption;
use xmpp_parsers::iq::Iq;
use xmpp_parsers::legacy_omemo::{Bundle, Device, DeviceList, Encrypted};
use xmpp_parsers::message::MessageType;
use xmpp_parsers::muc::Muc;
use xmpp_parsers::muc::muc::History;
use xmpp_parsers::muc::user::{Affiliation as MucAffiliation, MucUser, Status as MucStatus};
use xmpp_parsers::presence::{Presence, Type as PresenceType};
use xmpp_parsers::pubsub::pubsub::{Item as PubSubItem, Items, PubSub, Publish, PublishOptions};
use xmpp_parsers::pubsub::{NodeName, PubSubPayload};
use xmpp_parsers::{minidom::Element, ns};

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::config::XmppConfig;
use crate::error::ChannelError;
use crate::pairing::PairingStore;
use omemo::{OmemoManager, RemoteDeviceBundle};

use config::{bare_jid, is_jid_allowed, thread_id_from_jid};

const MAX_REPLY_TARGETS: usize = 10_000;
const OMEMO_SEND_TIMEOUT_SECS: u64 = 10;
const MUC_ADMIN_NS: &str = "http://jabber.org/protocol/muc#admin";
const OUTBOUND_RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, Default)]
struct EncryptedRoomState {
    ready: bool,
    members_only: bool,
    non_anonymous: bool,
    members: HashSet<String>,
    occupant_real_jids: HashMap<String, String>,
    self_nick: Option<String>,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct MucEncryptionDiagnostics {
    pub encrypted_rooms_total: usize,
    pub encrypted_rooms_ready: usize,
    pub last_room_error: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct MucPresenceDiagnostics {
    pub rooms_with_presence: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct OutboundRateLimitDiagnostics {
    pub max_messages_per_hour: u32,
    pub messages_in_current_window: usize,
}

/// An outbound stanza queued from `respond()`/`broadcast()` into the client task.
#[derive(Debug)]
struct OutboundMessage {
    to: String,
    body: String,
    groupchat: bool,
    preferred_device_id: Option<u32>,
}

#[derive(Debug)]
struct OutboundRateLimiter {
    max_messages_per_hour: u32,
    sent_at: VecDeque<Instant>,
}

impl OutboundRateLimiter {
    fn new(max_messages_per_hour: u32) -> Self {
        Self {
            max_messages_per_hour,
            sent_at: VecDeque::new(),
        }
    }

    fn prune_expired(&mut self, now: Instant) {
        while self
            .sent_at
            .front()
            .is_some_and(|sent_at| now.duration_since(*sent_at) >= OUTBOUND_RATE_LIMIT_WINDOW)
        {
            self.sent_at.pop_front();
        }
    }

    fn try_acquire(&mut self) -> Result<(), u32> {
        if self.max_messages_per_hour == 0 {
            return Ok(());
        }

        let now = Instant::now();
        self.prune_expired(now);

        let limit = self.max_messages_per_hour as usize;
        if self.sent_at.len() >= limit {
            return Err(self.max_messages_per_hour);
        }

        self.sent_at.push_back(now);
        Ok(())
    }

    fn update(&mut self, max_messages_per_hour: u32, reset_counter: bool) {
        self.max_messages_per_hour = max_messages_per_hour;
        if reset_counter {
            self.sent_at.clear();
        } else {
            self.prune_expired(Instant::now());
        }
    }

    fn diagnostics(&mut self) -> OutboundRateLimitDiagnostics {
        self.prune_expired(Instant::now());
        OutboundRateLimitDiagnostics {
            max_messages_per_hour: self.max_messages_per_hour,
            messages_in_current_window: self.sent_at.len(),
        }
    }
}

pub struct XmppChannel {
    config: XmppConfig,
    reply_targets: Arc<RwLock<LruCache<Uuid, String>>>,
    omemo: Arc<OmemoManager>,
    /// MUC participant tracking: room JID -> set of seen occupant resources.
    muc_participants: Arc<RwLock<HashMap<String, HashSet<String>>>>,
    encrypted_room_states: Arc<RwLock<HashMap<String, EncryptedRoomState>>>,
    /// Sender half for outbound stanzas.
    outbound_tx: mpsc::Sender<OutboundMessage>,
    outbound_rate_limiter: tokio::sync::Mutex<OutboundRateLimiter>,
    /// Receiver half taken by `start()` on the first call.
    outbound_rx: Arc<tokio::sync::Mutex<Option<mpsc::Receiver<OutboundMessage>>>>,
}

impl XmppChannel {
    pub async fn new(config: XmppConfig) -> Result<Self, ChannelError> {
        let omemo = OmemoManager::new(&config)
            .await
            .map_err(|e| ChannelError::StartupFailed {
                name: "xmpp".into(),
                reason: format!("OMEMO init failed: {e}"),
            })?;

        let cap =
            NonZeroUsize::new(MAX_REPLY_TARGETS).ok_or_else(|| ChannelError::StartupFailed {
                name: "xmpp".into(),
                reason: "reply target cache capacity must be non-zero".into(),
            })?;
        let (outbound_tx, outbound_rx) = mpsc::channel::<OutboundMessage>(64);
        let outbound_rate_limiter =
            tokio::sync::Mutex::new(OutboundRateLimiter::new(config.max_messages_per_hour));

        Ok(Self {
            config,
            reply_targets: Arc::new(RwLock::new(LruCache::new(cap))),
            omemo: Arc::new(omemo),
            muc_participants: Arc::new(RwLock::new(HashMap::new())),
            encrypted_room_states: Arc::new(RwLock::new(HashMap::new())),
            outbound_tx,
            outbound_rate_limiter,
            outbound_rx: Arc::new(tokio::sync::Mutex::new(Some(outbound_rx))),
        })
    }

    async fn queue_outbound(
        &self,
        target_jid: String,
        body: String,
        groupchat: bool,
        preferred_device_id: Option<u32>,
    ) -> Result<(), ChannelError> {
        tracing::debug!(
            to = %target_jid,
            len = body.len(),
            groupchat,
            "XMPP queuing outbound message"
        );

        if let Err(active_limit) = self.outbound_rate_limiter.lock().await.try_acquire() {
            tracing::warn!(
                to = %target_jid,
                groupchat,
                limit_per_hour = active_limit,
                "XMPP outbound message dropped by hourly rate limit"
            );
            return Err(ChannelError::SendFailed {
                name: "xmpp".into(),
                reason: format!(
                    "outbound XMPP message rate limit exceeded ({} messages/hour)",
                    active_limit
                ),
            });
        }

        self.outbound_tx
            .send(OutboundMessage {
                to: target_jid,
                body,
                groupchat,
                preferred_device_id,
            })
            .await
            .map_err(|_| ChannelError::SendFailed {
                name: "xmpp".into(),
                reason: "outbound channel closed (client task exited)".into(),
            })
    }

    async fn known_room_target(&self, target_jid: &str) -> bool {
        if self
            .config
            .allow_rooms
            .iter()
            .any(|room| room != "*" && room.eq_ignore_ascii_case(target_jid))
        {
            return true;
        }

        self.muc_participants.read().await.contains_key(target_jid)
    }

    pub async fn omemo_diagnostics(&self) -> omemo::OmemoDiagnostics {
        self.omemo.diagnostics().await
    }

    pub async fn muc_encryption_diagnostics(&self) -> MucEncryptionDiagnostics {
        let states = self.encrypted_room_states.read().await;
        let encrypted_rooms_total = self.config.encrypted_rooms.len();
        let encrypted_rooms_ready = self
            .config
            .encrypted_rooms
            .iter()
            .filter(|room| states.get(room.as_str()).is_some_and(|state| state.ready))
            .count();
        let last_room_error = self
            .config
            .encrypted_rooms
            .iter()
            .filter_map(|room| {
                states
                    .get(room.as_str())
                    .and_then(|state| state.last_error.clone())
            })
            .last();

        MucEncryptionDiagnostics {
            encrypted_rooms_total,
            encrypted_rooms_ready,
            last_room_error,
        }
    }

    pub async fn muc_presence_diagnostics(&self) -> MucPresenceDiagnostics {
        let participants = self.muc_participants.read().await;
        let mut rooms_with_presence = participants
            .iter()
            .filter(|(_, occupants)| !occupants.is_empty())
            .map(|(room, _)| room.clone())
            .collect::<Vec<_>>();
        rooms_with_presence.sort();

        MucPresenceDiagnostics {
            rooms_with_presence,
        }
    }

    pub async fn outbound_rate_limit_diagnostics(&self) -> OutboundRateLimitDiagnostics {
        self.outbound_rate_limiter.lock().await.diagnostics()
    }

    pub async fn set_outbound_rate_limit(
        &self,
        max_messages_per_hour: u32,
        reset_counter: bool,
    ) -> OutboundRateLimitDiagnostics {
        let mut limiter = self.outbound_rate_limiter.lock().await;
        limiter.update(max_messages_per_hour, reset_counter);
        limiter.diagnostics()
    }
}

fn room_requires_encryption(config: &XmppConfig, room_jid: &str) -> bool {
    is_jid_allowed(room_jid, &config.encrypted_rooms)
}

fn build_initial_presence() -> Presence {
    Presence::available()
}

fn default_muc_nick(bound_jid: &xmpp_parsers::jid::Jid) -> String {
    if let Some(resource) = bound_jid.resource() {
        return resource.to_string();
    }
    if let Some(node) = bound_jid.node() {
        return node.to_string();
    }
    "ironclaw".to_string()
}

fn build_muc_join_presence(
    room_jid: &str,
    nick: &str,
) -> Result<Presence, xmpp_parsers::jid::Error> {
    let room: xmpp_parsers::jid::BareJid = room_jid.parse()?;
    let occupant = room.with_resource_str(nick)?;
    Ok(Presence::available()
        .with_to(occupant)
        .with_payload(Muc::new().with_history(History::new().with_maxstanzas(0))))
}

fn build_muc_affiliation_query(affiliation: &str) -> Result<Element, ChannelError> {
    format!("<query xmlns='{MUC_ADMIN_NS}'><item affiliation='{affiliation}'/></query>")
        .parse::<Element>()
        .map_err(|err| ChannelError::SendFailed {
            name: "xmpp".into(),
            reason: format!("failed to build MUC affiliation query: {err}"),
        })
}

fn parse_muc_affiliation_jids(payload: Element) -> Result<Vec<String>, ChannelError> {
    if payload.name() != "query" || payload.ns() != MUC_ADMIN_NS {
        return Err(ChannelError::SendFailed {
            name: "xmpp".into(),
            reason: format!(
                "unexpected MUC affiliation payload: {{{}}}{}",
                payload.ns(),
                payload.name()
            ),
        });
    }

    let mut jids = Vec::new();
    for item in payload.children() {
        if item.name() != "item" || item.ns() != MUC_ADMIN_NS {
            continue;
        }
        let Some(jid) = item.attr("jid") else {
            continue;
        };
        let bare = bare_jid(jid).trim();
        if !bare.is_empty()
            && !jids
                .iter()
                .any(|value: &String| value.eq_ignore_ascii_case(bare))
        {
            jids.push(bare.to_string());
        }
    }

    Ok(jids)
}

async fn record_encrypted_room_error(
    encrypted_room_states: &Arc<RwLock<HashMap<String, EncryptedRoomState>>>,
    room_jid: &str,
    error: impl Into<String>,
) {
    let mut states = encrypted_room_states.write().await;
    let state = states
        .entry(room_jid.to_string())
        .or_insert_with(EncryptedRoomState::default);
    state.ready = false;
    state.last_error = Some(error.into());
}

async fn fetch_room_capabilities(
    client: &mut tokio_xmpp::Client,
    room_jid: &str,
    tx: &tokio::sync::mpsc::Sender<IncomingMessage>,
    config: &XmppConfig,
    reply_targets: &Arc<RwLock<LruCache<Uuid, String>>>,
    omemo: &Arc<OmemoManager>,
    muc_participants: &Arc<RwLock<HashMap<String, HashSet<String>>>>,
    encrypted_room_states: &Arc<RwLock<HashMap<String, EncryptedRoomState>>>,
    pairing_store: &PairingStore,
    outbound_tx: &mpsc::Sender<OutboundMessage>,
) -> Result<(bool, bool), ChannelError> {
    let room =
        room_jid
            .parse::<xmpp_parsers::jid::Jid>()
            .map_err(|e| ChannelError::SendFailed {
                name: "xmpp".into(),
                reason: format!("invalid XMPP room JID '{}': {e}", room_jid),
            })?;
    let payload = send_iq_request(
        client,
        Some(room),
        tokio_xmpp::IqRequest::Get(DiscoInfoQuery { node: None }.into()),
        tx,
        config,
        reply_targets,
        omemo,
        muc_participants,
        encrypted_room_states,
        pairing_store,
        outbound_tx,
    )
    .await?
    .ok_or_else(|| ChannelError::SendFailed {
        name: "xmpp".into(),
        reason: format!("room {} returned empty disco#info payload", room_jid),
    })?;
    let disco = DiscoInfoResult::try_from(payload).map_err(|e| ChannelError::SendFailed {
        name: "xmpp".into(),
        reason: format!("failed to parse room disco#info payload: {e}"),
    })?;
    let features: HashSet<String> = disco
        .features
        .into_iter()
        .map(|feature| feature.var)
        .collect();
    Ok((
        features.contains("muc_nonanonymous"),
        features.contains("muc_membersonly"),
    ))
}

async fn fetch_room_affiliation_jids(
    client: &mut tokio_xmpp::Client,
    room_jid: &str,
    affiliation: &str,
    tx: &tokio::sync::mpsc::Sender<IncomingMessage>,
    config: &XmppConfig,
    reply_targets: &Arc<RwLock<LruCache<Uuid, String>>>,
    omemo: &Arc<OmemoManager>,
    muc_participants: &Arc<RwLock<HashMap<String, HashSet<String>>>>,
    encrypted_room_states: &Arc<RwLock<HashMap<String, EncryptedRoomState>>>,
    pairing_store: &PairingStore,
    outbound_tx: &mpsc::Sender<OutboundMessage>,
) -> Result<Vec<String>, ChannelError> {
    let room =
        room_jid
            .parse::<xmpp_parsers::jid::Jid>()
            .map_err(|e| ChannelError::SendFailed {
                name: "xmpp".into(),
                reason: format!("invalid XMPP room JID '{}': {e}", room_jid),
            })?;
    let payload = send_iq_request(
        client,
        Some(room),
        tokio_xmpp::IqRequest::Get(build_muc_affiliation_query(affiliation)?),
        tx,
        config,
        reply_targets,
        omemo,
        muc_participants,
        encrypted_room_states,
        pairing_store,
        outbound_tx,
    )
    .await?;
    match payload {
        Some(payload) => parse_muc_affiliation_jids(payload),
        None => Ok(Vec::new()),
    }
}

async fn refresh_encrypted_room_state(
    client: &mut tokio_xmpp::Client,
    room_jid: &str,
    tx: &tokio::sync::mpsc::Sender<IncomingMessage>,
    config: &XmppConfig,
    reply_targets: &Arc<RwLock<LruCache<Uuid, String>>>,
    omemo: &Arc<OmemoManager>,
    muc_participants: &Arc<RwLock<HashMap<String, HashSet<String>>>>,
    encrypted_room_states: &Arc<RwLock<HashMap<String, EncryptedRoomState>>>,
    pairing_store: &PairingStore,
    outbound_tx: &mpsc::Sender<OutboundMessage>,
) -> Result<(), ChannelError> {
    let (non_anonymous, members_only) = fetch_room_capabilities(
        client,
        room_jid,
        tx,
        config,
        reply_targets,
        omemo,
        muc_participants,
        encrypted_room_states,
        pairing_store,
        outbound_tx,
    )
    .await?;

    if !non_anonymous {
        let reason = format!("encrypted room {room_jid} is not non-anonymous");
        record_encrypted_room_error(encrypted_room_states, room_jid, reason.clone()).await;
        return Err(ChannelError::SendFailed {
            name: "xmpp".into(),
            reason,
        });
    }
    if !members_only {
        let reason = format!("encrypted room {room_jid} is not members-only");
        record_encrypted_room_error(encrypted_room_states, room_jid, reason.clone()).await;
        return Err(ChannelError::SendFailed {
            name: "xmpp".into(),
            reason,
        });
    }

    let mut members = HashSet::new();
    for affiliation in ["member", "admin", "owner"] {
        for jid in fetch_room_affiliation_jids(
            client,
            room_jid,
            affiliation,
            tx,
            config,
            reply_targets,
            omemo,
            muc_participants,
            encrypted_room_states,
            pairing_store,
            outbound_tx,
        )
        .await?
        {
            members.insert(jid);
        }
    }
    members.insert(bare_jid(&config.jid).to_string());

    let mut states = encrypted_room_states.write().await;
    let state = states
        .entry(room_jid.to_string())
        .or_insert_with(EncryptedRoomState::default);
    state.ready = true;
    state.non_anonymous = non_anonymous;
    state.members_only = members_only;
    state.members = members;
    state.last_error = None;
    Ok(())
}

async fn join_configured_rooms(
    client: &mut tokio_xmpp::Client,
    config: &XmppConfig,
    bound_jid: &xmpp_parsers::jid::Jid,
    muc_participants: &Arc<RwLock<HashMap<String, HashSet<String>>>>,
    encrypted_room_states: &Arc<RwLock<HashMap<String, EncryptedRoomState>>>,
) {
    let nick = default_muc_nick(bound_jid);
    for room in &config.allow_rooms {
        if room == "*" {
            continue;
        }

        match build_muc_join_presence(room, &nick) {
            Ok(presence) => {
                if let Err(err) = client
                    .send_stanza(tokio_xmpp::Stanza::Presence(presence))
                    .await
                {
                    tracing::warn!(room = %room, error = %err, "XMPP room join failed");
                    continue;
                }
                muc_participants
                    .write()
                    .await
                    .entry(room.clone())
                    .or_insert_with(HashSet::new);
                encrypted_room_states
                    .write()
                    .await
                    .entry(room.clone())
                    .or_insert_with(EncryptedRoomState::default)
                    .self_nick = Some(nick.clone());
                tracing::info!(room = %room, nick = %nick, "XMPP room join requested");
            }
            Err(err) => {
                tracing::warn!(room = %room, error = %err, "Invalid XMPP room JID for join");
            }
        }
    }
}

fn build_plaintext_dm_stanza(
    to_jid: xmpp_parsers::jid::Jid,
    body: String,
) -> xmpp_parsers::message::Message {
    xmpp_parsers::message::Message::chat(to_jid)
        .with_body(xmpp_parsers::message::Lang::from(""), body)
}

fn session_only_remote_bundle(jid: &str, device_id: u32) -> RemoteDeviceBundle {
    RemoteDeviceBundle {
        jid: jid.to_string(),
        device_id,
        bundle: Bundle {
            signed_pre_key_public: None,
            signed_pre_key_signature: None,
            identity_key: None,
            prekeys: None,
        },
    }
}

fn maybe_fallback_plaintext_dm(
    config: &XmppConfig,
    to_jid: &xmpp_parsers::jid::Jid,
    body: &str,
    target_bare: &str,
    phase: &str,
    err: &ChannelError,
) -> Option<xmpp_parsers::message::Message> {
    if !config.allow_plaintext_fallback {
        return None;
    }

    tracing::warn!(
        target = %target_bare,
        phase,
        error = %err,
        "XMPP OMEMO lookup failed, falling back to plaintext DM"
    );
    Some(build_plaintext_dm_stanza(to_jid.clone(), body.to_string()))
}

fn is_sender_allowed_with_pairing(
    config: &XmppConfig,
    sender_bare: &str,
    pairing_store: &PairingStore,
) -> bool {
    if is_jid_allowed(sender_bare, &config.allow_from) {
        return true;
    }

    pairing_store
        .read_allow_from("xmpp")
        .map(|allowed| {
            allowed
                .iter()
                .any(|entry| entry == "*" || entry == sender_bare)
        })
        .unwrap_or(false)
}

async fn handle_pairing_request(
    pairing_store: &PairingStore,
    outbound_tx: &mpsc::Sender<OutboundMessage>,
    sender_bare: &str,
) -> Result<(), ChannelError> {
    let meta = serde_json::json!({
        "sender": sender_bare,
    });

    let result = pairing_store
        .upsert_request("xmpp", sender_bare, Some(meta))
        .map_err(|e| ChannelError::SendFailed {
            name: "xmpp".into(),
            reason: format!("failed to create pairing request: {e}"),
        })?;

    tracing::info!(
        sender = %sender_bare,
        code = %result.code,
        created = result.created,
        "XMPP pairing request upserted"
    );

    if result.created {
        outbound_tx
            .try_send(OutboundMessage {
                to: sender_bare.to_string(),
                body: format!(
                    "To pair with this bot, run: `ironclaw pairing approve xmpp {}`",
                    result.code
                ),
                groupchat: false,
                preferred_device_id: None,
            })
            .map_err(|e| ChannelError::SendFailed {
                name: "xmpp".into(),
                reason: format!("failed to enqueue pairing reply: {e}"),
            })?;
    }

    Ok(())
}

#[async_trait]
impl Channel for XmppChannel {
    fn name(&self) -> &str {
        "xmpp"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let jid_str = self.config.jid.clone();
        let password = self.config.password.expose_secret().to_string();

        let jid: xmpp_parsers::jid::Jid =
            jid_str
                .parse()
                .map_err(|e: xmpp_parsers::jid::Error| ChannelError::StartupFailed {
                    name: "xmpp".into(),
                    reason: format!("invalid JID '{}': {e}", jid_str),
                })?;

        let mut client = tokio_xmpp::Client::new(jid, password);
        let (tx, rx) = tokio::sync::mpsc::channel::<IncomingMessage>(64);

        let mut outbound_rx =
            self.outbound_rx
                .lock()
                .await
                .take()
                .ok_or_else(|| ChannelError::StartupFailed {
                    name: "xmpp".into(),
                    reason: "XmppChannel::start() called more than once".into(),
                })?;

        let config = self.config.clone();
        let reply_targets = Arc::clone(&self.reply_targets);
        let omemo = Arc::clone(&self.omemo);
        let muc_participants = Arc::clone(&self.muc_participants);
        let encrypted_room_states = Arc::clone(&self.encrypted_room_states);
        let outbound_tx = self.outbound_tx.clone();
        let pairing_store = PairingStore::new();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    event = client.next() => {
                        match event {
                            Some(tokio_xmpp::Event::Online { bound_jid, .. }) => {
                                tracing::info!(jid = %bound_jid, "XMPP connected");
                                if let Err(err) = client
                                    .send_stanza(tokio_xmpp::Stanza::Presence(build_initial_presence()))
                                    .await
                                {
                                    tracing::warn!("XMPP initial presence send failed: {err}");
                                }
                                join_configured_rooms(
                                    &mut client,
                                    &config,
                                    &bound_jid,
                                    &muc_participants,
                                    &encrypted_room_states,
                                )
                                .await;
                                for room in &config.encrypted_rooms {
                                    if let Err(err) = refresh_encrypted_room_state(
                                        &mut client,
                                        room,
                                        &tx,
                                        &config,
                                        &reply_targets,
                                        &omemo,
                                        &muc_participants,
                                        &encrypted_room_states,
                                        &pairing_store,
                                        &outbound_tx,
                                    )
                                    .await
                                    {
                                        tracing::warn!(room = %room, error = %err, "XMPP encrypted room initialization failed");
                                    }
                                }
                                tracing::debug!("XMPP OMEMO device_id={}", omemo.device_id().await);
                                if let Err(err) = publish_omemo_state(
                                    &mut client,
                                    &tx,
                                    &config,
                                    &reply_targets,
                                    &omemo,
                                    &muc_participants,
                                    &encrypted_room_states,
                                    &pairing_store,
                                    &outbound_tx,
                                )
                                .await
                                {
                                    omemo.record_error(err.to_string()).await;
                                    tracing::warn!("XMPP OMEMO publish failed: {err}");
                                }
                            }
                            Some(tokio_xmpp::Event::Stanza(stanza)) => {
                                if let Err(e) = process_stanza(
                                    stanza,
                                    &tx,
                                    &config,
                                    &reply_targets,
                                    &omemo,
                                    &muc_participants,
                                    &encrypted_room_states,
                                    &pairing_store,
                                    &outbound_tx,
                                )
                                .await
                                {
                                    tracing::warn!("XMPP stanza processing error: {e}");
                                }
                            }
                            Some(tokio_xmpp::Event::Disconnected(reason)) => {
                                tracing::warn!("XMPP disconnected: {:?}", reason);
                                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                                let jid_str2 = config.jid.clone();
                                let password2 = config.password.expose_secret().to_string();
                                match jid_str2.parse::<xmpp_parsers::jid::Jid>() {
                                    Ok(jid2) => {
                                        client = tokio_xmpp::Client::new(jid2, password2);
                                    }
                                    Err(e) => {
                                        tracing::error!("XMPP reconnect failed (bad JID): {e}");
                                        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                                    }
                                }
                            }
                            None => {
                                tracing::warn!("XMPP stream ended");
                                break;
                            }
                        }
                    }

                    outbound = outbound_rx.recv() => {
                        match outbound {
                            Some(msg) => match msg.to.parse::<xmpp_parsers::jid::Jid>() {
                                Ok(to_jid) => {
                                    let stanza = if msg.groupchat {
                                        if room_requires_encryption(&config, bare_jid(&msg.to)) {
                                            match build_outbound_groupchat_stanza(
                                                &mut client,
                                                to_jid,
                                                msg.body,
                                                &tx,
                                                &config,
                                                &reply_targets,
                                                &omemo,
                                                &muc_participants,
                                                &encrypted_room_states,
                                                &pairing_store,
                                                &outbound_tx,
                                            )
                                            .await
                                            {
                                                Ok(Some(stanza)) => stanza,
                                                Ok(None) => continue,
                                                Err(err) => {
                                                    omemo.record_error(err.to_string()).await;
                                                    tracing::warn!("XMPP encrypted room send preparation failed: {err}");
                                                    continue;
                                                }
                                            }
                                        } else {
                                            xmpp_parsers::message::Message::groupchat(to_jid)
                                                .with_body(xmpp_parsers::message::Lang::from(""), msg.body)
                                        }
                                    } else {
                                        match build_outbound_dm_stanza(
                                            &mut client,
                                            to_jid,
                                            msg.body,
                                            msg.preferred_device_id,
                                            &tx,
                                            &config,
                                            &reply_targets,
                                            &omemo,
                                            &muc_participants,
                                            &encrypted_room_states,
                                            &pairing_store,
                                            &outbound_tx,
                                        )
                                        .await
                                        {
                                            Ok(Some(stanza)) => stanza,
                                            Ok(None) => continue,
                                            Err(err) => {
                                                omemo.record_error(err.to_string()).await;
                                                tracing::warn!("XMPP OMEMO send preparation failed: {err}");
                                                continue;
                                            }
                                        }
                                    };
                                    if let Err(e) =
                                        client.send_stanza(tokio_xmpp::Stanza::Message(stanza)).await
                                    {
                                        tracing::warn!("XMPP send error: {e:?}");
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("XMPP outbound: invalid JID '{}': {e}", msg.to);
                                }
                            },
                            None => {
                                tracing::debug!("XMPP outbound channel closed, stopping task");
                                break;
                            }
                        }
                    }
                }
            }
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let target_jid = {
            let targets = self.reply_targets.read().await;
            targets.peek(&msg.id).cloned()
        }
        .or_else(|| xmpp_target_from_metadata(&msg.metadata))
        .ok_or_else(|| ChannelError::MissingRoutingTarget {
            name: "xmpp".into(),
            reason: format!("no routing target for message {}", msg.id),
        })?;

        self.queue_outbound(
            target_jid,
            response.content.clone(),
            metadata_is_groupchat(&msg.metadata),
            xmpp_sender_device_id_from_metadata(&msg.metadata),
        )
        .await
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        // Approval responses are currently scoped only to the thread, so
        // surfacing approval prompts in shared rooms would let anyone in the
        // room answer "yes" or "no". Keep status delivery DM-only for now.
        if metadata_is_groupchat(metadata) {
            return Ok(());
        }

        let Some(target_jid) = xmpp_target_from_metadata(metadata) else {
            return Ok(());
        };

        let text = match status {
            StatusUpdate::ApprovalNeeded {
                request_id,
                tool_name,
                parameters,
                allow_always,
                ..
            } => {
                let params_json = serde_json::to_string_pretty(&parameters)
                    .unwrap_or_else(|_| parameters.to_string());
                let always_line = if allow_always {
                    format!(
                        "\nReply with `always` or `a` to approve and auto-approve future {} requests.",
                        tool_name
                    )
                } else {
                    String::new()
                };
                Some(format!(
                    "Approval required for `{}`.\nRequest ID: `{}`\nParameters:\n{}\nReply with `yes`/`y` to approve or `no`/`n` to deny.{}",
                    tool_name, request_id, params_json, always_line
                ))
            }
            StatusUpdate::AuthRequired {
                extension_name,
                instructions,
                auth_url,
                setup_url,
            } => {
                let mut msg = format!("Authentication required for `{}`.", extension_name);
                if let Some(instructions) = instructions.filter(|value| !value.is_empty()) {
                    msg.push_str(&format!("\n{}", instructions));
                }
                if let Some(auth_url) = auth_url {
                    msg.push_str(&format!("\nAuth URL: {}", auth_url));
                }
                if let Some(setup_url) = setup_url {
                    msg.push_str(&format!("\nSetup URL: {}", setup_url));
                }
                Some(msg)
            }
            StatusUpdate::AuthCompleted {
                extension_name,
                success,
                message,
            } => {
                let mut msg = format!(
                    "Authentication {} for `{}`.",
                    if success { "completed" } else { "failed" },
                    extension_name
                );
                if !message.is_empty() {
                    msg.push_str(&format!("\n{}", message));
                }
                Some(msg)
            }
            StatusUpdate::Status(message) => {
                let normalized = message.trim();
                if normalized.eq_ignore_ascii_case("done")
                    || normalized.eq_ignore_ascii_case("awaiting approval")
                    || normalized.eq_ignore_ascii_case("rejected")
                {
                    None
                } else {
                    Some(message)
                }
            }
            _ => None,
        };

        if let Some(text) = text {
            self.queue_outbound(
                target_jid,
                text,
                false,
                xmpp_sender_device_id_from_metadata(metadata),
            )
            .await?;
        }

        Ok(())
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let groupchat = response
            .metadata
            .get("xmpp_room")
            .and_then(|value| value.as_str())
            .is_some_and(|room| room == user_id)
            || response
                .metadata
                .get("xmpp_type")
                .and_then(|value| value.as_str())
                == Some("groupchat")
            || self.known_room_target(user_id).await;

        self.queue_outbound(
            user_id.to_string(),
            response.content.clone(),
            groupchat,
            xmpp_sender_device_id_from_metadata(&response.metadata),
        )
        .await
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }

    fn conversation_context(
        &self,
        metadata: &serde_json::Value,
    ) -> std::collections::HashMap<String, String> {
        let mut ctx = std::collections::HashMap::new();
        if metadata_is_groupchat(metadata) {
            if let Some(nick) = metadata.get("xmpp_nick").and_then(|v| v.as_str()) {
                ctx.insert("sender".to_string(), nick.to_string());
            }
            if let Some(room) = metadata.get("xmpp_room").and_then(|v| v.as_str()) {
                ctx.insert("group".to_string(), room.to_string());
                ctx.insert("room".to_string(), room.to_string());
            }
        } else if let Some(from) = metadata.get("xmpp_from").and_then(|v| v.as_str()) {
            ctx.insert("sender".to_string(), from.to_string());
        }
        ctx
    }
}

async fn process_stanza(
    stanza: tokio_xmpp::Stanza,
    tx: &tokio::sync::mpsc::Sender<IncomingMessage>,
    config: &XmppConfig,
    reply_targets: &Arc<RwLock<LruCache<Uuid, String>>>,
    omemo: &Arc<OmemoManager>,
    muc_participants: &Arc<RwLock<HashMap<String, HashSet<String>>>>,
    encrypted_room_states: &Arc<RwLock<HashMap<String, EncryptedRoomState>>>,
    pairing_store: &PairingStore,
    outbound_tx: &mpsc::Sender<OutboundMessage>,
) -> Result<(), ChannelError> {
    match stanza {
        tokio_xmpp::Stanza::Message(msg) => {
            handle_message_stanza(
                msg,
                tx,
                config,
                reply_targets,
                omemo,
                muc_participants,
                encrypted_room_states,
                pairing_store,
                outbound_tx,
            )
            .await?;
        }
        tokio_xmpp::Stanza::Presence(presence) => {
            handle_presence_stanza(presence, config, muc_participants, encrypted_room_states).await;
        }
        tokio_xmpp::Stanza::Iq(_iq) => {}
    }
    Ok(())
}

async fn handle_message_stanza(
    msg: xmpp_parsers::message::Message,
    tx: &tokio::sync::mpsc::Sender<IncomingMessage>,
    config: &XmppConfig,
    reply_targets: &Arc<RwLock<LruCache<Uuid, String>>>,
    omemo: &Arc<OmemoManager>,
    muc_participants: &Arc<RwLock<HashMap<String, HashSet<String>>>>,
    encrypted_room_states: &Arc<RwLock<HashMap<String, EncryptedRoomState>>>,
    pairing_store: &PairingStore,
    outbound_tx: &mpsc::Sender<OutboundMessage>,
) -> Result<(), ChannelError> {
    let msg_type = &msg.type_;
    match msg_type {
        MessageType::Chat | MessageType::Groupchat => {}
        _ => return Ok(()),
    }

    let from_jid = match &msg.from {
        Some(jid) => jid.to_string(),
        None => return Ok(()),
    };
    let sender_bare = bare_jid(&from_jid).to_string();

    match msg_type {
        MessageType::Chat => match config.dm_policy.as_str() {
            "open" => {}
            "pairing" => {
                if !is_sender_allowed_with_pairing(config, &sender_bare, pairing_store) {
                    handle_pairing_request(pairing_store, outbound_tx, &sender_bare).await?;
                    return Ok(());
                }
            }
            "allowlist" => {
                if !is_jid_allowed(&sender_bare, &config.allow_from) {
                    tracing::debug!(
                        from = %sender_bare,
                        "XMPP DM from non-allowlisted sender, dropping"
                    );
                    return Ok(());
                }
            }
            other => {
                tracing::warn!(
                    policy = %other,
                    "XMPP DM policy not recognized, falling back to allowlist"
                );
                if !is_jid_allowed(&sender_bare, &config.allow_from) {
                    tracing::debug!(
                        from = %sender_bare,
                        "XMPP DM from non-allowlisted sender, dropping"
                    );
                    return Ok(());
                }
            }
        },
        MessageType::Groupchat => {
            let room = bare_jid(&from_jid);
            if !is_jid_allowed(room, &config.allow_rooms) {
                tracing::debug!(room = %room, "XMPP groupchat from non-allowlisted room, dropping");
                return Ok(());
            }
        }
        _ => return Ok(()),
    }

    let room_jid =
        matches!(msg_type, MessageType::Groupchat).then(|| bare_jid(&from_jid).to_string());
    let encrypted_room = room_jid
        .as_deref()
        .is_some_and(|room| room_requires_encryption(config, room));

    let mut sender_device_id = None;
    let content = if !matches!(msg_type, MessageType::Groupchat) || encrypted_room {
        if let Some(encrypted_elem) = find_omemo_payload(&msg.payloads) {
            match Encrypted::try_from(encrypted_elem.clone()) {
                Ok(encrypted) => {
                    sender_device_id = Some(encrypted.header.sid);
                    let sender_device_id = encrypted.header.sid;
                    let decrypt_sender = if encrypted_room {
                        let Some(room) = room_jid.as_deref() else {
                            return Ok(());
                        };
                        let Some(nick) = jid_resource(&from_jid) else {
                            omemo
                                .record_error(format!(
                                    "encrypted room message from {} is missing an occupant nick",
                                    from_jid
                                ))
                                .await;
                            return Ok(());
                        };
                        let states = encrypted_room_states.read().await;
                        let Some(state) = states.get(room) else {
                            omemo
                                .record_error(format!(
                                    "encrypted room {} has no cached room state",
                                    room
                                ))
                                .await;
                            return Ok(());
                        };
                        if state.self_nick.as_deref() == Some(nick) {
                            return Ok(());
                        }
                        let Some(real_jid) = state.occupant_real_jids.get(nick).cloned() else {
                            omemo
                                .record_error(format!(
                                    "encrypted room {} has no real JID mapping for occupant {}",
                                    room, nick
                                ))
                                .await;
                            return Ok(());
                        };
                        real_jid
                    } else {
                        sender_bare.clone()
                    };
                    match omemo
                        .decrypt(&decrypt_sender, sender_device_id, encrypted)
                        .await
                    {
                        Ok(result) => result.plaintext,
                        Err(e) => {
                            omemo
                                .record_error(format!(
                                    "OMEMO decrypt failed from {}: {}",
                                    sender_bare, e
                                ))
                                .await;
                            tracing::warn!("OMEMO decrypt failed from {}: {}", sender_bare, e);
                            if encrypted_room {
                                return Ok(());
                            }
                            if config.allow_plaintext_fallback {
                                extract_body_from_message(&msg).unwrap_or_default()
                            } else {
                                return Ok(());
                            }
                        }
                    }
                }
                Err(err) => {
                    omemo
                        .record_error(format!(
                            "OMEMO payload parse failed from {}: {:?}",
                            from_jid, err
                        ))
                        .await;
                    tracing::warn!("OMEMO payload parse failed from {}: {:?}", from_jid, err);
                    if encrypted_room {
                        return Ok(());
                    }
                    if config.allow_plaintext_fallback {
                        extract_body_from_message(&msg).unwrap_or_default()
                    } else {
                        return Ok(());
                    }
                }
            }
        } else {
            if encrypted_room {
                omemo
                    .record_error(format!(
                        "encrypted room message from {} arrived without an OMEMO payload",
                        from_jid
                    ))
                    .await;
                return Ok(());
            }
            match extract_body_from_message(&msg) {
                Some(body) if !body.is_empty() => body,
                _ => return Ok(()),
            }
        }
    } else {
        match extract_body_from_message(&msg) {
            Some(body) if !body.is_empty() => body,
            _ => return Ok(()),
        }
    };

    if content.trim().is_empty() {
        return Ok(());
    }

    let is_groupchat = matches!(msg_type, MessageType::Groupchat);
    if is_groupchat && let (Some(room), Some(nick)) = (room_jid.as_deref(), jid_resource(&from_jid))
    {
        let states = encrypted_room_states.read().await;
        if states
            .get(room)
            .and_then(|state| state.self_nick.as_deref())
            == Some(nick)
        {
            tracing::debug!(room = %room, nick = %nick, "Dropping self-authored XMPP groupchat");
            return Ok(());
        }
    }
    let target_jid = room_jid.clone().unwrap_or_else(|| sender_bare.clone());
    let thread_id = thread_id_from_jid(&target_jid);
    let sender_id = if is_groupchat {
        from_jid.clone()
    } else {
        sender_bare.clone()
    };

    let mut metadata = serde_json::json!({
        "chat_type": if is_groupchat { "group" } else { "private" },
        "xmpp_from": sender_bare,
        "xmpp_target": target_jid.clone(),
        "xmpp_type": if is_groupchat { "groupchat" } else { "chat" },
    });
    if let Some(device_id) = sender_device_id {
        metadata["xmpp_sender_device_id"] = serde_json::json!(device_id);
    }
    if let Some(room) = &room_jid {
        metadata["xmpp_room"] = serde_json::Value::String(room.clone());
        if let Some(nick) = jid_resource(&from_jid) {
            metadata["xmpp_nick"] = serde_json::Value::String(nick.to_string());
        }
        muc_participants
            .write()
            .await
            .entry(room.clone())
            .or_insert_with(HashSet::new);
    }

    let mut incoming = IncomingMessage::new("xmpp", target_jid.clone(), content)
        .with_owner_id(&config.jid)
        .with_thread(&thread_id)
        .with_sender_id(sender_id)
        .with_metadata(metadata);
    if is_groupchat && let Some(nick) = jid_resource(&from_jid) {
        incoming = incoming.with_user_name(nick);
    }

    reply_targets.write().await.put(incoming.id, target_jid);

    if tx.send(incoming).await.is_err() {
        tracing::warn!("XMPP message stream receiver dropped");
    }

    Ok(())
}

async fn handle_presence_stanza(
    presence: xmpp_parsers::presence::Presence,
    config: &XmppConfig,
    muc_participants: &Arc<RwLock<HashMap<String, HashSet<String>>>>,
    encrypted_room_states: &Arc<RwLock<HashMap<String, EncryptedRoomState>>>,
) {
    let from_jid = match &presence.from {
        Some(jid) => jid.to_string(),
        None => return,
    };

    let is_muc = presence.payloads.iter().any(|p| {
        p.ns() == "http://jabber.org/protocol/muc#user"
            || p.ns() == "http://jabber.org/protocol/muc"
    });

    if !is_muc {
        return;
    }

    let room_jid = bare_jid(&from_jid).to_string();
    let participant = jid_resource(&from_jid)
        .unwrap_or(room_jid.as_str())
        .to_string();

    let mut participants = muc_participants.write().await;
    match presence.type_ {
        PresenceType::None => {
            participants
                .entry(room_jid.clone())
                .or_insert_with(HashSet::new)
                .insert(participant);
        }
        PresenceType::Unavailable => {
            if let Some(set) = participants.get_mut(&room_jid) {
                set.remove(&participant);
            }
        }
        _ => {}
    }

    let Some(muc_user) = presence
        .payloads
        .iter()
        .find(|payload| payload.name() == "x" && payload.ns() == ns::MUC_USER)
        .cloned()
        .and_then(|payload| MucUser::try_from(payload).ok())
    else {
        return;
    };

    let mut states = encrypted_room_states.write().await;
    let state = states
        .entry(room_jid.clone())
        .or_insert_with(EncryptedRoomState::default);
    let is_self_presence = muc_user
        .status
        .iter()
        .any(|status| *status == MucStatus::SelfPresence);
    if is_self_presence
        && let Some(nick) = muc_user
            .items
            .iter()
            .find_map(|item| item.nick.as_deref())
            .or_else(|| jid_resource(&from_jid))
            .filter(|nick| !nick.is_empty())
            .map(ToOwned::to_owned)
    {
        state.self_nick = Some(nick);
    }

    if !room_requires_encryption(config, &room_jid) {
        return;
    }

    for item in muc_user.items {
        let nick = item
            .nick
            .as_deref()
            .or_else(|| jid_resource(&from_jid))
            .filter(|nick| !nick.is_empty())
            .map(ToOwned::to_owned);
        let real_jid = item
            .jid
            .as_ref()
            .map(ToString::to_string)
            .map(|jid| bare_jid(&jid).to_string());

        if let (Some(nick), Some(real_jid)) = (nick.as_deref(), real_jid.as_deref()) {
            if presence.type_ == PresenceType::Unavailable {
                state.occupant_real_jids.remove(nick);
            } else {
                state
                    .occupant_real_jids
                    .insert(nick.to_string(), real_jid.to_string());
            }
        }
        if let Some(real_jid) = real_jid {
            match item.affiliation {
                MucAffiliation::Member | MucAffiliation::Admin | MucAffiliation::Owner => {
                    state.members.insert(real_jid);
                }
                MucAffiliation::None | MucAffiliation::Outcast => {
                    state.members.remove(&real_jid);
                }
            }
        }
    }

    if muc_user
        .status
        .iter()
        .any(|status| *status == MucStatus::ConfigRoomNonAnonymous)
    {
        state.non_anonymous = true;
    }
}

async fn build_outbound_dm_stanza(
    client: &mut tokio_xmpp::Client,
    to_jid: xmpp_parsers::jid::Jid,
    body: String,
    preferred_device_id: Option<u32>,
    tx: &tokio::sync::mpsc::Sender<IncomingMessage>,
    config: &XmppConfig,
    reply_targets: &Arc<RwLock<LruCache<Uuid, String>>>,
    omemo: &Arc<OmemoManager>,
    muc_participants: &Arc<RwLock<HashMap<String, HashSet<String>>>>,
    encrypted_room_states: &Arc<RwLock<HashMap<String, EncryptedRoomState>>>,
    pairing_store: &PairingStore,
    outbound_tx: &mpsc::Sender<OutboundMessage>,
) -> Result<Option<xmpp_parsers::message::Message>, ChannelError> {
    if !omemo.diagnostics().await.bundle_published {
        if let Err(err) = publish_omemo_state(
            client,
            tx,
            config,
            reply_targets,
            omemo,
            muc_participants,
            encrypted_room_states,
            pairing_store,
            outbound_tx,
        )
        .await
        {
            let target_bare = bare_jid(&to_jid.to_string()).to_string();
            if let Some(stanza) = maybe_fallback_plaintext_dm(
                config,
                &to_jid,
                &body,
                &target_bare,
                "local_publish",
                &err,
            ) {
                return Ok(Some(stanza));
            }
            return Err(err);
        }
    }

    let target_bare = bare_jid(&to_jid.to_string()).to_string();
    let mut recipients = match fetch_recipient_bundles(
        client,
        &target_bare,
        None,
        tx,
        config,
        reply_targets,
        omemo,
        muc_participants,
        encrypted_room_states,
        pairing_store,
        outbound_tx,
    )
    .await
    {
        Ok(recipients) => recipients,
        Err(err) => {
            if let Some(stanza) = maybe_fallback_plaintext_dm(
                config,
                &to_jid,
                &body,
                &target_bare,
                "recipient_devices",
                &err,
            ) {
                return Ok(Some(stanza));
            }
            return Err(err);
        }
    };
    if let Some(device_id) = preferred_device_id {
        let already_targeted = recipients
            .iter()
            .any(|recipient| recipient.jid == target_bare && recipient.device_id == device_id);
        if !already_targeted
            && omemo
                .has_session(&target_bare, device_id)
                .await
                .map_err(|e| ChannelError::SendFailed {
                    name: "xmpp".into(),
                    reason: format!("OMEMO session lookup failed: {e}"),
                })?
        {
            recipients.push(session_only_remote_bundle(&target_bare, device_id));
        }
    }

    let own_bare = bare_jid(&config.jid).to_string();
    let own_device_id = omemo.device_id().await;
    if own_bare != target_bare {
        let own_devices = match fetch_recipient_bundles(
            client,
            &own_bare,
            Some(own_device_id),
            tx,
            config,
            reply_targets,
            omemo,
            muc_participants,
            encrypted_room_states,
            pairing_store,
            outbound_tx,
        )
        .await
        {
            Ok(devices) => devices,
            Err(err) => {
                if let Some(stanza) = maybe_fallback_plaintext_dm(
                    config,
                    &to_jid,
                    &body,
                    &target_bare,
                    "own_devices",
                    &err,
                ) {
                    return Ok(Some(stanza));
                }
                return Err(err);
            }
        };
        recipients.extend(own_devices);
    }

    if recipients.is_empty() {
        if config.allow_plaintext_fallback {
            return Ok(Some(build_plaintext_dm_stanza(to_jid, body)));
        }
        return Err(ChannelError::SendFailed {
            name: "xmpp".into(),
            reason: format!("no OMEMO recipient devices available for {}", target_bare),
        });
    }

    let encrypted =
        omemo
            .encrypt(&body, recipients)
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "xmpp".into(),
                reason: format!("OMEMO encrypt failed: {e}"),
            })?;

    let mut message = xmpp_parsers::message::Message::chat(to_jid);
    message.payloads.push(encrypted.into());
    message.payloads.push(
        ExplicitMessageEncryption {
            namespace: ns::LEGACY_OMEMO.to_string(),
            name: Some("OMEMO".to_string()),
        }
        .into(),
    );
    message.payloads.push(store_hint_element());

    Ok(Some(message))
}

async fn encrypted_room_recipients(
    client: &mut tokio_xmpp::Client,
    room_jid: &str,
    tx: &tokio::sync::mpsc::Sender<IncomingMessage>,
    config: &XmppConfig,
    reply_targets: &Arc<RwLock<LruCache<Uuid, String>>>,
    omemo: &Arc<OmemoManager>,
    muc_participants: &Arc<RwLock<HashMap<String, HashSet<String>>>>,
    encrypted_room_states: &Arc<RwLock<HashMap<String, EncryptedRoomState>>>,
    pairing_store: &PairingStore,
    outbound_tx: &mpsc::Sender<OutboundMessage>,
) -> Result<Vec<RemoteDeviceBundle>, ChannelError> {
    let should_refresh = {
        let states = encrypted_room_states.read().await;
        match states.get(room_jid) {
            Some(state) => !state.ready || state.members.is_empty(),
            None => true,
        }
    };
    if should_refresh {
        refresh_encrypted_room_state(
            client,
            room_jid,
            tx,
            config,
            reply_targets,
            omemo,
            muc_participants,
            encrypted_room_states,
            pairing_store,
            outbound_tx,
        )
        .await?;
    }

    let room_state = {
        let states = encrypted_room_states.read().await;
        states
            .get(room_jid)
            .cloned()
            .ok_or_else(|| ChannelError::SendFailed {
                name: "xmpp".into(),
                reason: format!("encrypted room {} has no runtime state", room_jid),
            })?
    };
    if !room_state.ready {
        return Err(ChannelError::SendFailed {
            name: "xmpp".into(),
            reason: room_state
                .last_error
                .unwrap_or_else(|| format!("encrypted room {} is not ready", room_jid)),
        });
    }

    let own_bare = bare_jid(&config.jid).to_string();
    let own_device_id = omemo.device_id().await;
    let mut recipients = Vec::new();
    for member in room_state.members {
        let exclude_device_id = (member == own_bare).then_some(own_device_id);
        recipients.extend(
            fetch_recipient_bundles(
                client,
                &member,
                exclude_device_id,
                tx,
                config,
                reply_targets,
                omemo,
                muc_participants,
                encrypted_room_states,
                pairing_store,
                outbound_tx,
            )
            .await?,
        );
    }
    Ok(recipients)
}

async fn build_outbound_groupchat_stanza(
    client: &mut tokio_xmpp::Client,
    to_jid: xmpp_parsers::jid::Jid,
    body: String,
    tx: &tokio::sync::mpsc::Sender<IncomingMessage>,
    config: &XmppConfig,
    reply_targets: &Arc<RwLock<LruCache<Uuid, String>>>,
    omemo: &Arc<OmemoManager>,
    muc_participants: &Arc<RwLock<HashMap<String, HashSet<String>>>>,
    encrypted_room_states: &Arc<RwLock<HashMap<String, EncryptedRoomState>>>,
    pairing_store: &PairingStore,
    outbound_tx: &mpsc::Sender<OutboundMessage>,
) -> Result<Option<xmpp_parsers::message::Message>, ChannelError> {
    if !omemo.diagnostics().await.bundle_published {
        publish_omemo_state(
            client,
            tx,
            config,
            reply_targets,
            omemo,
            muc_participants,
            encrypted_room_states,
            pairing_store,
            outbound_tx,
        )
        .await?;
    }

    let room_jid = bare_jid(&to_jid.to_string()).to_string();
    let recipients = encrypted_room_recipients(
        client,
        &room_jid,
        tx,
        config,
        reply_targets,
        omemo,
        muc_participants,
        encrypted_room_states,
        pairing_store,
        outbound_tx,
    )
    .await?;
    if recipients.is_empty() {
        let reason = format!("encrypted room {} has no OMEMO recipient devices", room_jid);
        record_encrypted_room_error(encrypted_room_states, &room_jid, reason.clone()).await;
        return Err(ChannelError::SendFailed {
            name: "xmpp".into(),
            reason,
        });
    }

    let encrypted =
        omemo
            .encrypt(&body, recipients)
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "xmpp".into(),
                reason: format!("OMEMO encrypt failed: {e}"),
            })?;

    let mut message = xmpp_parsers::message::Message::groupchat(to_jid);
    message.payloads.push(encrypted.into());
    message.payloads.push(
        ExplicitMessageEncryption {
            namespace: ns::LEGACY_OMEMO.to_string(),
            name: Some("OMEMO".to_string()),
        }
        .into(),
    );
    message.payloads.push(store_hint_element());
    Ok(Some(message))
}

async fn publish_omemo_state(
    client: &mut tokio_xmpp::Client,
    tx: &tokio::sync::mpsc::Sender<IncomingMessage>,
    config: &XmppConfig,
    reply_targets: &Arc<RwLock<LruCache<Uuid, String>>>,
    omemo: &Arc<OmemoManager>,
    muc_participants: &Arc<RwLock<HashMap<String, HashSet<String>>>>,
    encrypted_room_states: &Arc<RwLock<HashMap<String, EncryptedRoomState>>>,
    pairing_store: &PairingStore,
    outbound_tx: &mpsc::Sender<OutboundMessage>,
) -> Result<(), ChannelError> {
    let local_device_id = omemo.device_id().await;
    let bare_self = bare_jid(&config.jid).to_string();
    let mut devices = fetch_omemo_device_list(
        client,
        &bare_self,
        tx,
        config,
        reply_targets,
        omemo,
        muc_participants,
        encrypted_room_states,
        pairing_store,
        outbound_tx,
    )
    .await
    .unwrap_or_default();
    if !devices.contains(&local_device_id) {
        devices.push(local_device_id);
    }
    devices.sort_unstable();
    devices.dedup();

    publish_pep_payload(
        client,
        ns::LEGACY_OMEMO_DEVICELIST,
        DeviceList {
            devices: devices.into_iter().map(|id| Device { id }).collect(),
        },
        tx,
        config,
        reply_targets,
        omemo,
        muc_participants,
        encrypted_room_states,
        pairing_store,
        outbound_tx,
    )
    .await?;

    let bundle = omemo.bundle().await.map_err(|e| ChannelError::SendFailed {
        name: "xmpp".into(),
        reason: format!("OMEMO bundle build failed: {e}"),
    })?;

    publish_pep_payload(
        client,
        &format!("{}:{}", ns::LEGACY_OMEMO_BUNDLES, local_device_id),
        bundle,
        tx,
        config,
        reply_targets,
        omemo,
        muc_participants,
        encrypted_room_states,
        pairing_store,
        outbound_tx,
    )
    .await?;

    omemo.mark_bundle_published().await;
    Ok(())
}

async fn fetch_recipient_bundles(
    client: &mut tokio_xmpp::Client,
    bare_jid_str: &str,
    exclude_device_id: Option<u32>,
    tx: &tokio::sync::mpsc::Sender<IncomingMessage>,
    config: &XmppConfig,
    reply_targets: &Arc<RwLock<LruCache<Uuid, String>>>,
    omemo: &Arc<OmemoManager>,
    muc_participants: &Arc<RwLock<HashMap<String, HashSet<String>>>>,
    encrypted_room_states: &Arc<RwLock<HashMap<String, EncryptedRoomState>>>,
    pairing_store: &PairingStore,
    outbound_tx: &mpsc::Sender<OutboundMessage>,
) -> Result<Vec<RemoteDeviceBundle>, ChannelError> {
    let device_ids = fetch_omemo_device_list(
        client,
        bare_jid_str,
        tx,
        config,
        reply_targets,
        omemo,
        muc_participants,
        encrypted_room_states,
        pairing_store,
        outbound_tx,
    )
    .await?;

    let mut bundles = Vec::new();
    for device_id in device_ids {
        if Some(device_id) == exclude_device_id {
            continue;
        }
        if let Some(bundle) = fetch_omemo_bundle(
            client,
            bare_jid_str,
            device_id,
            tx,
            config,
            reply_targets,
            omemo,
            muc_participants,
            encrypted_room_states,
            pairing_store,
            outbound_tx,
        )
        .await?
        {
            bundles.push(RemoteDeviceBundle {
                jid: bare_jid_str.to_string(),
                device_id,
                bundle,
            });
        }
    }

    Ok(bundles)
}

async fn fetch_omemo_device_list(
    client: &mut tokio_xmpp::Client,
    bare_jid_str: &str,
    tx: &tokio::sync::mpsc::Sender<IncomingMessage>,
    config: &XmppConfig,
    reply_targets: &Arc<RwLock<LruCache<Uuid, String>>>,
    omemo: &Arc<OmemoManager>,
    muc_participants: &Arc<RwLock<HashMap<String, HashSet<String>>>>,
    encrypted_room_states: &Arc<RwLock<HashMap<String, EncryptedRoomState>>>,
    pairing_store: &PairingStore,
    outbound_tx: &mpsc::Sender<OutboundMessage>,
) -> Result<Vec<u32>, ChannelError> {
    let to = bare_jid_str
        .parse::<xmpp_parsers::jid::Jid>()
        .map_err(|e| ChannelError::SendFailed {
            name: "xmpp".into(),
            reason: format!("invalid JID '{}': {e}", bare_jid_str),
        })?;
    let payload = send_iq_request(
        client,
        Some(to),
        tokio_xmpp::IqRequest::Get(
            PubSub::Items(latest_pubsub_items_request(ns::LEGACY_OMEMO_DEVICELIST)).into(),
        ),
        tx,
        config,
        reply_targets,
        omemo,
        muc_participants,
        encrypted_room_states,
        pairing_store,
        outbound_tx,
    )
    .await?;

    let Some(payload) = payload else {
        return Ok(Vec::new());
    };
    let pubsub = PubSub::try_from(payload).map_err(|e| ChannelError::SendFailed {
        name: "xmpp".into(),
        reason: format!("failed to parse OMEMO device list response: {e}"),
    })?;
    let PubSub::Items(items) = pubsub else {
        return Ok(Vec::new());
    };
    let Some(payload) = latest_pubsub_payload(items) else {
        return Ok(Vec::new());
    };
    let device_list = DeviceList::try_from(payload).map_err(|e| ChannelError::SendFailed {
        name: "xmpp".into(),
        reason: format!("failed to parse OMEMO device list payload: {e}"),
    })?;
    let devices: Vec<u32> = device_list
        .devices
        .into_iter()
        .map(|device| device.id)
        .collect();
    let _ = omemo.save_remote_device_list(bare_jid_str, &devices).await;
    Ok(devices)
}

async fn fetch_omemo_bundle(
    client: &mut tokio_xmpp::Client,
    bare_jid_str: &str,
    device_id: u32,
    tx: &tokio::sync::mpsc::Sender<IncomingMessage>,
    config: &XmppConfig,
    reply_targets: &Arc<RwLock<LruCache<Uuid, String>>>,
    omemo: &Arc<OmemoManager>,
    muc_participants: &Arc<RwLock<HashMap<String, HashSet<String>>>>,
    encrypted_room_states: &Arc<RwLock<HashMap<String, EncryptedRoomState>>>,
    pairing_store: &PairingStore,
    outbound_tx: &mpsc::Sender<OutboundMessage>,
) -> Result<Option<Bundle>, ChannelError> {
    let to = bare_jid_str
        .parse::<xmpp_parsers::jid::Jid>()
        .map_err(|e| ChannelError::SendFailed {
            name: "xmpp".into(),
            reason: format!("invalid JID '{}': {e}", bare_jid_str),
        })?;
    let node = format!("{}:{}", ns::LEGACY_OMEMO_BUNDLES, device_id);
    let payload = send_iq_request(
        client,
        Some(to),
        tokio_xmpp::IqRequest::Get(PubSub::Items(latest_pubsub_items_request(&node)).into()),
        tx,
        config,
        reply_targets,
        omemo,
        muc_participants,
        encrypted_room_states,
        pairing_store,
        outbound_tx,
    )
    .await?;

    let Some(payload) = payload else {
        return Ok(None);
    };
    let pubsub = PubSub::try_from(payload).map_err(|e| ChannelError::SendFailed {
        name: "xmpp".into(),
        reason: format!("failed to parse OMEMO bundle response: {e}"),
    })?;
    let PubSub::Items(items) = pubsub else {
        return Ok(None);
    };
    let Some(payload) = latest_pubsub_payload(items) else {
        return Ok(None);
    };
    let bundle = Bundle::try_from(payload.clone()).map_err(|e| ChannelError::SendFailed {
        name: "xmpp".into(),
        reason: format!("failed to parse OMEMO bundle payload: {e}"),
    })?;
    let _ = omemo
        .save_remote_bundle(bare_jid_str, device_id, &bundle)
        .await;
    Ok(Some(bundle))
}

async fn publish_pep_payload<P>(
    client: &mut tokio_xmpp::Client,
    node: &str,
    payload: P,
    tx: &tokio::sync::mpsc::Sender<IncomingMessage>,
    config: &XmppConfig,
    reply_targets: &Arc<RwLock<LruCache<Uuid, String>>>,
    omemo: &Arc<OmemoManager>,
    muc_participants: &Arc<RwLock<HashMap<String, HashSet<String>>>>,
    encrypted_room_states: &Arc<RwLock<HashMap<String, EncryptedRoomState>>>,
    pairing_store: &PairingStore,
    outbound_tx: &mpsc::Sender<OutboundMessage>,
) -> Result<(), ChannelError>
where
    P: PubSubPayload + Clone,
{
    let publish = Publish {
        node: NodeName(node.to_string()),
        items: vec![PubSubItem::new(None, None, Some(payload))],
    };
    let publish_with_options = send_iq_request(
        client,
        None,
        tokio_xmpp::IqRequest::Set(
            PubSub::Publish {
                publish: publish.clone(),
                publish_options: Some(omemo_publish_options()),
            }
            .into(),
        ),
        tx,
        config,
        reply_targets,
        omemo,
        muc_participants,
        encrypted_room_states,
        pairing_store,
        outbound_tx,
    )
    .await;

    match publish_with_options {
        Ok(_) => Ok(()),
        Err(err) if is_xmpp_iq_error(&err) => {
            tracing::warn!(
                node,
                error = %err,
                "XMPP PEP publish with publish-options was rejected, retrying without publish-options"
            );
            send_iq_request(
                client,
                None,
                tokio_xmpp::IqRequest::Set(
                    PubSub::Publish {
                        publish,
                        publish_options: None,
                    }
                    .into(),
                ),
                tx,
                config,
                reply_targets,
                omemo,
                muc_participants,
                encrypted_room_states,
                pairing_store,
                outbound_tx,
            )
            .await?;
            Ok(())
        }
        Err(err) => Err(err),
    }
}

fn build_iq_request(
    id: String,
    to: Option<xmpp_parsers::jid::Jid>,
    request: tokio_xmpp::IqRequest,
) -> Iq {
    match request {
        tokio_xmpp::IqRequest::Get(payload) => Iq::Get {
            from: None,
            to,
            id,
            payload,
        },
        tokio_xmpp::IqRequest::Set(payload) => Iq::Set {
            from: None,
            to,
            id,
            payload,
        },
    }
}

fn matching_iq_response(
    iq: Iq,
    expected_id: &str,
) -> Option<Result<Option<Element>, ChannelError>> {
    match iq {
        Iq::Result { id, payload, .. } if id == expected_id => Some(Ok(payload)),
        Iq::Error { id, error, .. } if id == expected_id => Some(Err(ChannelError::SendFailed {
            name: "xmpp".into(),
            reason: format!("XMPP IQ returned error: {:?}", error),
        })),
        _ => None,
    }
}

fn omemo_publish_options() -> PublishOptions {
    PublishOptions {
        form: Some(DataForm::new(
            DataFormType::Submit,
            "http://jabber.org/protocol/pubsub#publish-options",
            vec![Field::new("pubsub#access_model", FieldType::ListSingle).with_value("open")],
        )),
    }
}

fn is_xmpp_iq_error(err: &ChannelError) -> bool {
    matches!(
        err,
        ChannelError::SendFailed { name, reason }
            if name == "xmpp" && reason.starts_with("XMPP IQ returned error:")
    )
}

fn latest_pubsub_items_request(node: &str) -> Items {
    let mut items = Items::new(node);
    items.max_items = Some(1);
    items
}

fn latest_pubsub_payload(items: Items) -> Option<Element> {
    items.items.into_iter().rev().find_map(|item| item.payload)
}

async fn send_iq_request(
    client: &mut tokio_xmpp::Client,
    to: Option<xmpp_parsers::jid::Jid>,
    request: tokio_xmpp::IqRequest,
    tx: &tokio::sync::mpsc::Sender<IncomingMessage>,
    config: &XmppConfig,
    reply_targets: &Arc<RwLock<LruCache<Uuid, String>>>,
    omemo: &Arc<OmemoManager>,
    muc_participants: &Arc<RwLock<HashMap<String, HashSet<String>>>>,
    encrypted_room_states: &Arc<RwLock<HashMap<String, EncryptedRoomState>>>,
    pairing_store: &PairingStore,
    outbound_tx: &mpsc::Sender<OutboundMessage>,
) -> Result<Option<Element>, ChannelError> {
    let request_id = Uuid::new_v4().to_string();
    let iq = build_iq_request(request_id.clone(), to, request);
    client
        .send_stanza(tokio_xmpp::Stanza::Iq(iq))
        .await
        .map_err(|err| ChannelError::SendFailed {
            name: "xmpp".into(),
            reason: format!("XMPP IQ request failed: {err}"),
        })?;

    let deadline = tokio::time::sleep(std::time::Duration::from_secs(OMEMO_SEND_TIMEOUT_SECS));
    tokio::pin!(deadline);

    loop {
        tokio::select! {
            _ = &mut deadline => {
                return Err(ChannelError::SendFailed {
                    name: "xmpp".into(),
                    reason: "timed out waiting for XMPP IQ response".into(),
                });
            }
            event = client.next() => {
                match event {
                    Some(tokio_xmpp::Event::Online { .. }) => {}
                    Some(tokio_xmpp::Event::Stanza(stanza)) => {
                        if let tokio_xmpp::Stanza::Iq(iq) = &stanza {
                            if let Some(response) = matching_iq_response(iq.clone(), &request_id) {
                                return response;
                            }
                        }
                        process_stanza(
                            stanza,
                            tx,
                            config,
                            reply_targets,
                            omemo,
                            muc_participants,
                            encrypted_room_states,
                            pairing_store,
                            outbound_tx,
                        )
                        .await?;
                    }
                    Some(tokio_xmpp::Event::Disconnected(reason)) => {
                        return Err(ChannelError::SendFailed {
                            name: "xmpp".into(),
                            reason: format!("XMPP disconnected while waiting for IQ response: {:?}", reason),
                        });
                    }
                    None => {
                        return Err(ChannelError::SendFailed {
                            name: "xmpp".into(),
                            reason: "XMPP stream ended while waiting for IQ response".into(),
                        });
                    }
                }
            }
        }
    }
}

fn store_hint_element() -> Element {
    "<store xmlns='urn:xmpp:hints'/>"
        .parse::<Element>()
        .expect("static store hint XML is valid")
}

fn metadata_is_groupchat(metadata: &serde_json::Value) -> bool {
    metadata.get("xmpp_type").and_then(|v| v.as_str()) == Some("groupchat")
}

fn xmpp_target_from_metadata(metadata: &serde_json::Value) -> Option<String> {
    metadata
        .get("xmpp_target")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .or_else(|| {
            metadata
                .get("xmpp_room")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            metadata
                .get("xmpp_from")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        })
}

fn xmpp_sender_device_id_from_metadata(metadata: &serde_json::Value) -> Option<u32> {
    metadata
        .get("xmpp_sender_device_id")
        .and_then(|value| value.as_u64())
        .and_then(|value| u32::try_from(value).ok())
}

fn jid_resource(jid: &str) -> Option<&str> {
    jid.split_once('/')
        .map(|(_, resource)| resource)
        .filter(|resource| !resource.is_empty())
}

/// Find an OMEMO `<encrypted>` element in the message's extension payloads.
fn find_omemo_payload(
    payloads: &[tokio_xmpp::minidom::Element],
) -> Option<&tokio_xmpp::minidom::Element> {
    payloads
        .iter()
        .find(|p| p.name() == "encrypted" && p.ns() == "eu.siacs.conversations.axolotl")
}

/// Extract plain text body from an xmpp-parsers Message.
fn extract_body_from_message(msg: &xmpp_parsers::message::Message) -> Option<String> {
    if let Some(body) = msg.bodies.get("").filter(|b| !b.is_empty()) {
        return Some(body.clone());
    }
    msg.bodies.values().find(|b| !b.is_empty()).cloned()
}

#[cfg(test)]
fn extract_sid(encrypted_xml: &str) -> Option<u32> {
    for pat_prefix in &["sid='", "sid=\""] {
        if let Some(pos) = encrypted_xml.find(pat_prefix) {
            let start = pos + pat_prefix.len();
            let quote_char = if pat_prefix.ends_with('\'') {
                '\''
            } else {
                '"'
            };
            let end = encrypted_xml[start..].find(quote_char)?;
            return encrypted_xml[start..start + end].parse().ok();
        }
    }
    None
}

#[cfg(test)]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use secrecy::SecretString;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_xml_escape() {
        assert_eq!(xml_escape("a&b<c>d"), "a&amp;b&lt;c&gt;d");
    }

    #[test]
    fn test_extract_sid() {
        assert_eq!(extract_sid("<header sid='12345'>"), Some(12345));
        assert_eq!(extract_sid("<header sid=\"99\">"), Some(99));
        assert_eq!(extract_sid("<header>"), None);
    }

    #[test]
    fn test_extract_body_from_message_empty() {
        let msg = xmpp_parsers::message::Message::chat(None);
        assert!(extract_body_from_message(&msg).is_none());
    }

    #[test]
    fn test_extract_body_from_message() {
        use xmpp_parsers::message::Lang;
        let msg = xmpp_parsers::message::Message::chat(None)
            .with_body(Lang::from(""), "Hello world".to_string());
        assert_eq!(
            extract_body_from_message(&msg).as_deref(),
            Some("Hello world")
        );
    }

    #[test]
    fn omemo_lookup_errors_can_fallback_to_plaintext_dm() {
        use xmpp_parsers::jid::Jid;

        let mut config = test_config();
        config.allow_plaintext_fallback = true;
        let to_jid = "alice@example.com/phone".parse::<Jid>().expect("valid jid");
        let err = ChannelError::SendFailed {
            name: "xmpp".into(),
            reason: "timed out waiting for XMPP IQ response".into(),
        };

        let stanza = maybe_fallback_plaintext_dm(
            &config,
            &to_jid,
            "hello",
            "alice@example.com",
            "recipient_devices",
            &err,
        )
        .expect("plaintext fallback stanza");

        assert_eq!(stanza.type_, MessageType::Chat);
        assert_eq!(extract_body_from_message(&stanza).as_deref(), Some("hello"));
    }

    #[test]
    fn omemo_publish_errors_can_fallback_to_plaintext_dm() {
        use xmpp_parsers::jid::Jid;

        let mut config = test_config();
        config.allow_plaintext_fallback = true;
        let to_jid = "alice@example.com/phone".parse::<Jid>().expect("valid jid");
        let err = ChannelError::SendFailed {
            name: "xmpp".into(),
            reason: "XMPP IQ returned error: conflict".into(),
        };

        let stanza = maybe_fallback_plaintext_dm(
            &config,
            &to_jid,
            "hello",
            "alice@example.com",
            "local_publish",
            &err,
        )
        .expect("plaintext fallback stanza");

        assert_eq!(stanza.type_, MessageType::Chat);
        assert_eq!(extract_body_from_message(&stanza).as_deref(), Some("hello"));
    }

    #[test]
    fn omemo_lookup_errors_do_not_fallback_when_disabled() {
        use xmpp_parsers::jid::Jid;

        let mut config = test_config();
        config.allow_plaintext_fallback = false;
        let to_jid = "alice@example.com/phone".parse::<Jid>().expect("valid jid");
        let err = ChannelError::SendFailed {
            name: "xmpp".into(),
            reason: "timed out waiting for XMPP IQ response".into(),
        };

        let stanza = maybe_fallback_plaintext_dm(
            &config,
            &to_jid,
            "hello",
            "alice@example.com",
            "recipient_devices",
            &err,
        );

        assert!(stanza.is_none());
    }

    #[test]
    fn initial_presence_is_available() {
        let presence = build_initial_presence();
        assert_eq!(presence.type_, PresenceType::None);
        assert!(presence.from.is_none());
        assert!(presence.to.is_none());
        assert!(presence.payloads.is_empty());
    }

    #[test]
    fn default_muc_nick_prefers_bound_resource() {
        let jid = "bot@example.com/pixel8pro"
            .parse::<xmpp_parsers::jid::Jid>()
            .expect("valid jid");
        assert_eq!(default_muc_nick(&jid), "pixel8pro");

        let bare = "bot@example.com"
            .parse::<xmpp_parsers::jid::Jid>()
            .expect("valid bare jid");
        assert_eq!(default_muc_nick(&bare), "bot");
    }

    #[test]
    fn muc_join_presence_targets_room_nick_and_requests_no_history() {
        let presence = build_muc_join_presence("room@conference.example.com", "ironclaw")
            .expect("valid room join presence");

        assert_eq!(presence.type_, PresenceType::None);
        assert_eq!(
            presence.to.as_ref().map(ToString::to_string).as_deref(),
            Some("room@conference.example.com/ironclaw")
        );

        let muc = Muc::try_from(
            presence
                .payloads
                .first()
                .cloned()
                .expect("muc payload present"),
        )
        .expect("payload parses as muc");
        assert_eq!(muc.password, None);
        assert_eq!(muc.history.and_then(|h| h.maxstanzas), Some(0));
    }

    #[test]
    fn matching_iq_response_accepts_result_without_from() {
        let iq = Iq::Result {
            from: None,
            to: Some("alice@example.com".parse().expect("valid jid")),
            id: "abc123".to_string(),
            payload: None,
        };

        let response = matching_iq_response(iq, "abc123").expect("matching response");
        assert!(response.expect("successful iq").is_none());
    }

    #[test]
    fn matching_iq_response_ignores_other_ids() {
        let iq = Iq::Result {
            from: None,
            to: Some("alice@example.com".parse().expect("valid jid")),
            id: "other".to_string(),
            payload: None,
        };

        assert!(matching_iq_response(iq, "abc123").is_none());
    }

    #[test]
    fn omemo_publish_options_use_open_access_model() {
        let options = omemo_publish_options();
        let form = options.form.expect("publish options form");
        assert_eq!(
            form.form_type().as_deref(),
            Some("http://jabber.org/protocol/pubsub#publish-options")
        );

        let access_model = form
            .fields
            .iter()
            .find(|field| field.var.as_deref() == Some("pubsub#access_model"))
            .expect("access model field");
        assert_eq!(access_model.type_, FieldType::ListSingle);
        assert_eq!(access_model.values, vec!["open".to_string()]);
    }

    #[test]
    fn latest_pubsub_items_request_limits_to_latest_item() {
        let items = latest_pubsub_items_request(ns::LEGACY_OMEMO_DEVICELIST);
        assert_eq!(items.max_items, Some(1));
        assert_eq!(items.node.0, ns::LEGACY_OMEMO_DEVICELIST);
    }

    #[test]
    fn latest_pubsub_payload_prefers_last_payload() {
        let first = "<first xmlns='urn:test'/>".parse::<Element>().unwrap();
        let second = "<second xmlns='urn:test'/>".parse::<Element>().unwrap();
        let items = Items {
            max_items: None,
            node: NodeName("urn:test:node".to_string()),
            subid: None,
            items: vec![
                PubSubItem {
                    id: None,
                    publisher: None,
                    payload: Some(first),
                },
                PubSubItem {
                    id: None,
                    publisher: None,
                    payload: Some(second.clone()),
                },
            ],
        };

        let payload = latest_pubsub_payload(items).expect("payload");
        assert_eq!(payload, second);
    }

    #[test]
    fn parse_muc_affiliation_jids_extracts_unique_bare_jids() {
        let payload = format!(
            "<query xmlns='{MUC_ADMIN_NS}'>\
                <item affiliation='member' jid='alice@example.com/laptop'/>\
                <item affiliation='member' jid='alice@example.com/phone'/>\
                <item affiliation='admin' jid='bob@example.com'/>\
            </query>"
        )
        .parse::<Element>()
        .expect("valid affiliation response");

        let jids = parse_muc_affiliation_jids(payload).expect("parsed affiliation jids");
        assert_eq!(jids, vec!["alice@example.com", "bob@example.com"]);
    }

    #[test]
    fn xmpp_sender_device_id_is_parsed_from_metadata() {
        let metadata = serde_json::json!({
            "xmpp_sender_device_id": 12345_u32,
        });

        assert_eq!(xmpp_sender_device_id_from_metadata(&metadata), Some(12345));
    }

    #[test]
    fn iq_error_is_retryable_for_publish_options() {
        let err = ChannelError::SendFailed {
            name: "xmpp".into(),
            reason: "XMPP IQ returned error: conflict".into(),
        };
        assert!(is_xmpp_iq_error(&err));
    }

    #[test]
    fn timeout_is_not_retryable_as_publish_options_error() {
        let err = ChannelError::SendFailed {
            name: "xmpp".into(),
            reason: "timed out waiting for XMPP IQ response".into(),
        };
        assert!(!is_xmpp_iq_error(&err));
    }

    fn test_config() -> XmppConfig {
        let omemo_store_dir =
            std::env::temp_dir().join(format!("ironclaw-xmpp-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&omemo_store_dir).expect("create test omemo dir");
        XmppConfig {
            jid: "bot@example.com".to_string(),
            password: SecretString::from("password".to_string()),
            allow_from: vec!["alice@example.com".to_string()],
            dm_policy: "allowlist".to_string(),
            allow_rooms: vec!["room@conference.example.com".to_string()],
            encrypted_rooms: vec![],
            device_id: 0,
            omemo_store_dir,
            allow_plaintext_fallback: true,
            max_messages_per_hour: 0,
        }
    }

    async fn recv_outbound(channel: &XmppChannel) -> OutboundMessage {
        let mut guard = channel.outbound_rx.lock().await;
        let rx = guard.as_mut().expect("receiver available");
        rx.recv().await.expect("outbound message available")
    }

    fn temp_pairing_store() -> (TempDir, PairingStore) {
        let dir = TempDir::new().expect("tempdir");
        let store = PairingStore::with_base_dir(dir.path().to_path_buf());
        (dir, store)
    }

    #[tokio::test]
    async fn respond_uses_explicit_xmpp_target_metadata() {
        let channel = XmppChannel::new(test_config())
            .await
            .expect("channel initializes");
        let msg = IncomingMessage::new("xmpp", "room@conference.example.com", "hi").with_metadata(
            serde_json::json!({
                "chat_type": "group",
                "xmpp_target": "room@conference.example.com",
                "xmpp_room": "room@conference.example.com",
                "xmpp_type": "groupchat",
            }),
        );

        channel
            .respond(&msg, OutgoingResponse::text("reply"))
            .await
            .expect("respond queues outbound message");

        let outbound = recv_outbound(&channel).await;
        assert_eq!(outbound.to, "room@conference.example.com");
        assert!(outbound.groupchat);
        assert_eq!(outbound.body, "reply");
        assert_eq!(outbound.preferred_device_id, None);
    }

    #[tokio::test]
    async fn respond_carries_sender_device_preference_for_direct_omemo_reply() {
        let channel = XmppChannel::new(test_config())
            .await
            .expect("channel initializes");
        let msg = IncomingMessage::new("xmpp", "alice@example.com", "hi").with_metadata(
            serde_json::json!({
                "chat_type": "private",
                "xmpp_from": "alice@example.com",
                "xmpp_target": "alice@example.com",
                "xmpp_type": "chat",
                "xmpp_sender_device_id": 4242_u32,
            }),
        );

        channel
            .respond(&msg, OutgoingResponse::text("reply"))
            .await
            .expect("respond queues outbound message");

        let outbound = recv_outbound(&channel).await;
        assert_eq!(outbound.to, "alice@example.com");
        assert!(!outbound.groupchat);
        assert_eq!(outbound.preferred_device_id, Some(4242));
    }

    #[tokio::test]
    async fn broadcast_uses_known_room_targets_as_groupchat() {
        let channel = XmppChannel::new(test_config())
            .await
            .expect("channel initializes");
        channel
            .muc_participants
            .write()
            .await
            .insert("room@conference.example.com".to_string(), HashSet::new());

        channel
            .broadcast(
                "room@conference.example.com",
                OutgoingResponse::text("proactive room message"),
            )
            .await
            .expect("broadcast queues outbound message");

        let outbound = recv_outbound(&channel).await;
        assert_eq!(outbound.to, "room@conference.example.com");
        assert!(outbound.groupchat);
    }

    #[tokio::test]
    async fn queue_outbound_enforces_hourly_message_limit() {
        let mut config = test_config();
        config.max_messages_per_hour = 1;
        let channel = XmppChannel::new(config).await.expect("channel initializes");

        channel
            .queue_outbound(
                "alice@example.com".to_string(),
                "first".to_string(),
                false,
                None,
            )
            .await
            .expect("first outbound message should queue");

        let err = channel
            .queue_outbound(
                "alice@example.com".to_string(),
                "second".to_string(),
                false,
                None,
            )
            .await
            .expect_err("second outbound message should hit the rate limit");

        match err {
            ChannelError::SendFailed { name, reason } => {
                assert_eq!(name, "xmpp");
                assert!(reason.contains("1 messages/hour"));
            }
            other => panic!("unexpected error: {other:?}"),
        }

        let outbound = recv_outbound(&channel).await;
        assert_eq!(outbound.body, "first");

        let mut guard = channel.outbound_rx.lock().await;
        let rx = guard.as_mut().expect("receiver available");
        assert!(
            rx.try_recv().is_err(),
            "rate-limited message should not queue"
        );
    }

    #[tokio::test]
    async fn live_outbound_rate_limit_override_can_disable_and_reset_counter() {
        let mut config = test_config();
        config.max_messages_per_hour = 1;
        let channel = XmppChannel::new(config).await.expect("channel initializes");

        channel
            .queue_outbound(
                "alice@example.com".to_string(),
                "first".to_string(),
                false,
                None,
            )
            .await
            .expect("first outbound message should queue");
        assert_eq!(
            channel.outbound_rate_limit_diagnostics().await,
            OutboundRateLimitDiagnostics {
                max_messages_per_hour: 1,
                messages_in_current_window: 1,
            }
        );

        assert!(
            channel
                .queue_outbound(
                    "alice@example.com".to_string(),
                    "second".to_string(),
                    false,
                    None,
                )
                .await
                .is_err(),
            "second message should be blocked before override"
        );

        let disabled = channel.set_outbound_rate_limit(0, false).await;
        assert_eq!(
            disabled,
            OutboundRateLimitDiagnostics {
                max_messages_per_hour: 0,
                messages_in_current_window: 1,
            }
        );

        channel
            .queue_outbound(
                "alice@example.com".to_string(),
                "third".to_string(),
                false,
                None,
            )
            .await
            .expect("override should allow outbound message");

        let reset = channel.set_outbound_rate_limit(2, true).await;
        assert_eq!(
            reset,
            OutboundRateLimitDiagnostics {
                max_messages_per_hour: 2,
                messages_in_current_window: 0,
            }
        );

        channel
            .queue_outbound(
                "alice@example.com".to_string(),
                "fourth".to_string(),
                false,
                None,
            )
            .await
            .expect("first post-reset message should queue");
        channel
            .queue_outbound(
                "alice@example.com".to_string(),
                "fifth".to_string(),
                false,
                None,
            )
            .await
            .expect("second post-reset message should queue");
        assert!(
            channel
                .queue_outbound(
                    "alice@example.com".to_string(),
                    "sixth".to_string(),
                    false,
                    None,
                )
                .await
                .is_err(),
            "third post-reset message should hit the new limit"
        );
    }

    #[tokio::test]
    async fn muc_encryption_diagnostics_reports_ready_room_counts() {
        let mut config = test_config();
        config.encrypted_rooms = vec!["room@conference.example.com".to_string()];

        let channel = XmppChannel::new(config).await.expect("channel initializes");
        channel.encrypted_room_states.write().await.insert(
            "room@conference.example.com".to_string(),
            EncryptedRoomState {
                ready: true,
                last_error: Some("last room error".to_string()),
                ..EncryptedRoomState::default()
            },
        );

        let diagnostics = channel.muc_encryption_diagnostics().await;
        assert_eq!(diagnostics.encrypted_rooms_total, 1);
        assert_eq!(diagnostics.encrypted_rooms_ready, 1);
        assert_eq!(
            diagnostics.last_room_error.as_deref(),
            Some("last room error")
        );
    }

    #[tokio::test]
    async fn muc_presence_diagnostics_reports_only_rooms_with_presence() {
        let channel = XmppChannel::new(test_config())
            .await
            .expect("channel initializes");
        channel.muc_participants.write().await.insert(
            "room@conference.example.com".to_string(),
            HashSet::from([String::from("ironclaw")]),
        );
        channel
            .muc_participants
            .write()
            .await
            .insert("empty@conference.example.com".to_string(), HashSet::new());

        let diagnostics = channel.muc_presence_diagnostics().await;
        assert_eq!(
            diagnostics.rooms_with_presence,
            vec!["room@conference.example.com"]
        );
    }

    #[tokio::test]
    async fn handle_presence_stanza_tracks_real_jids_for_encrypted_rooms() {
        let mut config = test_config();
        config.encrypted_rooms = vec!["room@conference.example.com".to_string()];
        let channel = XmppChannel::new(config.clone())
            .await
            .expect("channel initializes");

        let presence = Presence::try_from(
            "<presence xmlns='jabber:client' from='room@conference.example.com/alice'>\
                <x xmlns='http://jabber.org/protocol/muc#user'>\
                    <item affiliation='member' jid='alice@example.com/laptop' role='participant'/>\
                </x>\
            </presence>"
                .parse::<Element>()
                .expect("valid presence xml"),
        )
        .expect("valid presence");

        handle_presence_stanza(
            presence,
            &config,
            &channel.muc_participants,
            &channel.encrypted_room_states,
        )
        .await;

        let states = channel.encrypted_room_states.read().await;
        let state = states
            .get("room@conference.example.com")
            .expect("encrypted room state");
        assert_eq!(
            state.occupant_real_jids.get("alice").map(String::as_str),
            Some("alice@example.com")
        );
        assert!(state.members.contains("alice@example.com"));
    }

    #[tokio::test]
    async fn handle_presence_stanza_tracks_self_nick_for_plain_rooms() {
        let config = test_config();
        let channel = XmppChannel::new(config.clone())
            .await
            .expect("channel initializes");
        let presence = xmpp_parsers::presence::Presence::try_from(
            "<presence xmlns='jabber:client' from='room@conference.example.com/ruffles'>\
                <x xmlns='http://jabber.org/protocol/muc#user'>\
                    <item affiliation='none' role='participant' nick='ruffles'/>\
                    <status code='110'/>\
                </x>\
            </presence>"
                .parse::<Element>()
                .expect("valid presence xml"),
        )
        .expect("valid presence");

        handle_presence_stanza(
            presence,
            &config,
            &channel.muc_participants,
            &channel.encrypted_room_states,
        )
        .await;

        let states = channel.encrypted_room_states.read().await;
        assert_eq!(
            states
                .get("room@conference.example.com")
                .and_then(|state| state.self_nick.as_deref()),
            Some("ruffles")
        );
    }

    #[tokio::test]
    async fn handle_groupchat_message_sets_explicit_target_and_group_metadata() {
        use xmpp_parsers::jid::Jid;
        use xmpp_parsers::message::Lang;

        let config = test_config();
        let channel = XmppChannel::new(config.clone())
            .await
            .expect("channel initializes");
        let (tx, mut rx) = mpsc::channel(1);
        let (_pairing_dir, pairing_store) = temp_pairing_store();

        let mut msg = xmpp_parsers::message::Message::groupchat(
            "room@conference.example.com"
                .parse::<Jid>()
                .expect("valid room jid"),
        )
        .with_body(Lang::from(""), "hello room".to_string());
        msg.from = Some(
            "room@conference.example.com/alice"
                .parse::<Jid>()
                .expect("valid occupant jid"),
        );

        handle_message_stanza(
            msg,
            &tx,
            &config,
            &channel.reply_targets,
            &channel.omemo,
            &channel.muc_participants,
            &channel.encrypted_room_states,
            &pairing_store,
            &channel.outbound_tx,
        )
        .await
        .expect("message handled");

        let incoming = rx.recv().await.expect("incoming message emitted");
        assert_eq!(incoming.user_id, "room@conference.example.com");
        assert_eq!(incoming.owner_id, "bot@example.com");
        assert_eq!(incoming.sender_id, "room@conference.example.com/alice");
        assert_eq!(incoming.user_name.as_deref(), Some("alice"));
        assert_eq!(
            incoming
                .metadata
                .get("chat_type")
                .and_then(|value| value.as_str()),
            Some("group")
        );
        assert_eq!(
            incoming
                .metadata
                .get("xmpp_target")
                .and_then(|value| value.as_str()),
            Some("room@conference.example.com")
        );
        assert_eq!(
            incoming
                .metadata
                .get("xmpp_nick")
                .and_then(|value| value.as_str()),
            Some("alice")
        );
    }

    #[tokio::test]
    async fn handle_plain_groupchat_from_self_nick_is_dropped() {
        use xmpp_parsers::jid::Jid;
        use xmpp_parsers::message::Lang;

        let config = test_config();
        let channel = XmppChannel::new(config.clone())
            .await
            .expect("channel initializes");
        channel.encrypted_room_states.write().await.insert(
            "room@conference.example.com".to_string(),
            EncryptedRoomState {
                self_nick: Some("ruffles".to_string()),
                ..EncryptedRoomState::default()
            },
        );
        let (tx, mut rx) = mpsc::channel(1);
        let (_pairing_dir, pairing_store) = temp_pairing_store();

        let mut msg = xmpp_parsers::message::Message::groupchat(
            "room@conference.example.com"
                .parse::<Jid>()
                .expect("valid room jid"),
        )
        .with_body(Lang::from(""), "hello from self".to_string());
        msg.from = Some(
            "room@conference.example.com/ruffles"
                .parse::<Jid>()
                .expect("valid occupant jid"),
        );

        handle_message_stanza(
            msg,
            &tx,
            &config,
            &channel.reply_targets,
            &channel.omemo,
            &channel.muc_participants,
            &channel.encrypted_room_states,
            &pairing_store,
            &channel.outbound_tx,
        )
        .await
        .expect("message handled");

        assert!(
            rx.try_recv().is_err(),
            "self-authored room message should drop"
        );
    }

    #[tokio::test]
    async fn handle_plain_groupchat_in_encrypted_room_is_dropped() {
        use xmpp_parsers::jid::Jid;
        use xmpp_parsers::message::Lang;

        let mut config = test_config();
        config.encrypted_rooms = vec!["room@conference.example.com".to_string()];

        let channel = XmppChannel::new(config.clone())
            .await
            .expect("channel initializes");
        let (tx, mut rx) = mpsc::channel(1);
        let (_pairing_dir, pairing_store) = temp_pairing_store();

        let mut msg = xmpp_parsers::message::Message::groupchat(
            "room@conference.example.com"
                .parse::<Jid>()
                .expect("valid room jid"),
        )
        .with_body(Lang::from(""), "plaintext room message".to_string());
        msg.from = Some(
            "room@conference.example.com/alice"
                .parse::<Jid>()
                .expect("valid occupant jid"),
        );

        handle_message_stanza(
            msg,
            &tx,
            &config,
            &channel.reply_targets,
            &channel.omemo,
            &channel.muc_participants,
            &channel.encrypted_room_states,
            &pairing_store,
            &channel.outbound_tx,
        )
        .await
        .expect("message handled");

        assert!(
            rx.try_recv().is_err(),
            "encrypted room should drop plaintext"
        );
        let diagnostics = channel.omemo.diagnostics().await;
        assert!(
            diagnostics
                .last_omemo_error
                .as_deref()
                .is_some_and(|value| value.contains("arrived without an OMEMO payload"))
        );
    }

    #[tokio::test]
    async fn handle_direct_message_pairing_blocks_delivery_and_replies_with_code() {
        use xmpp_parsers::jid::Jid;
        use xmpp_parsers::message::Lang;

        let mut config = test_config();
        config.allow_from.clear();
        config.dm_policy = "pairing".to_string();

        let channel = XmppChannel::new(config.clone())
            .await
            .expect("channel initializes");
        let (tx, mut rx) = mpsc::channel(1);
        let (_pairing_dir, pairing_store) = temp_pairing_store();

        let mut msg = xmpp_parsers::message::Message::chat(
            "bot@example.com".parse::<Jid>().expect("valid bot jid"),
        )
        .with_body(Lang::from(""), "hello bot".to_string());
        msg.from = Some(
            "eve@example.com/phone"
                .parse::<Jid>()
                .expect("valid sender jid"),
        );

        handle_message_stanza(
            msg,
            &tx,
            &config,
            &channel.reply_targets,
            &channel.omemo,
            &channel.muc_participants,
            &channel.encrypted_room_states,
            &pairing_store,
            &channel.outbound_tx,
        )
        .await
        .expect("message handled");

        assert!(rx.try_recv().is_err(), "pairing-gated DM should be blocked");

        let pending = pairing_store
            .list_pending("xmpp")
            .expect("pending requests");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "eve@example.com");

        let outbound = recv_outbound(&channel).await;
        assert_eq!(outbound.to, "eve@example.com");
        assert!(!outbound.groupchat);
        assert!(outbound.body.contains("ironclaw pairing approve xmpp"));
    }

    #[tokio::test]
    async fn malformed_omemo_payload_records_bridge_diagnostic_error() {
        use xmpp_parsers::jid::Jid;

        let mut config = test_config();
        config.allow_plaintext_fallback = false;

        let channel = XmppChannel::new(config.clone())
            .await
            .expect("channel initializes");
        let (tx, mut rx) = mpsc::channel(1);
        let (_pairing_dir, pairing_store) = temp_pairing_store();

        let mut msg =
            xmpp_parsers::message::Message::chat(Some("bot@example.com".parse::<Jid>().unwrap()));
        msg.from = Some("alice@example.com/phone".parse::<Jid>().expect("valid jid"));
        msg.payloads.push(
            "<encrypted xmlns='eu.siacs.conversations.axolotl'><broken/></encrypted>"
                .parse::<Element>()
                .expect("valid xml"),
        );

        handle_message_stanza(
            msg,
            &tx,
            &config,
            &channel.reply_targets,
            &channel.omemo,
            &channel.muc_participants,
            &channel.encrypted_room_states,
            &pairing_store,
            &channel.outbound_tx,
        )
        .await
        .expect("message handled");

        assert!(
            rx.try_recv().is_err(),
            "malformed OMEMO should not emit an incoming message"
        );
        let diagnostics = channel.omemo.diagnostics().await;
        let error = diagnostics
            .last_omemo_error
            .expect("last_omemo_error should be set");
        assert!(error.contains("OMEMO payload parse failed from alice@example.com"));
    }

    #[tokio::test]
    async fn handle_direct_message_allows_preapproved_sender_from_pairing_store() {
        use xmpp_parsers::jid::Jid;
        use xmpp_parsers::message::Lang;

        let mut config = test_config();
        config.allow_from.clear();
        config.dm_policy = "pairing".to_string();

        let channel = XmppChannel::new(config.clone())
            .await
            .expect("channel initializes");
        let (tx, mut rx) = mpsc::channel(1);
        let (_pairing_dir, pairing_store) = temp_pairing_store();

        let request = pairing_store
            .upsert_request("xmpp", "eve@example.com", None)
            .expect("create pairing request");
        pairing_store
            .approve("xmpp", &request.code)
            .expect("approve pairing request");

        let mut msg = xmpp_parsers::message::Message::chat(
            "bot@example.com".parse::<Jid>().expect("valid bot jid"),
        )
        .with_body(Lang::from(""), "hello after pairing".to_string());
        msg.from = Some(
            "eve@example.com/phone"
                .parse::<Jid>()
                .expect("valid sender jid"),
        );

        handle_message_stanza(
            msg,
            &tx,
            &config,
            &channel.reply_targets,
            &channel.omemo,
            &channel.muc_participants,
            &channel.encrypted_room_states,
            &pairing_store,
            &channel.outbound_tx,
        )
        .await
        .expect("message handled");

        let incoming = rx.recv().await.expect("incoming message emitted");
        assert_eq!(incoming.user_id, "eve@example.com");
        assert_eq!(incoming.sender_id, "eve@example.com");
        assert_eq!(incoming.content, "hello after pairing");
    }
}
