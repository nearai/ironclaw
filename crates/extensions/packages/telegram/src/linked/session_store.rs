//! [`IronclawSession`] — `grammers_session::Session` implemented over IronClaw
//! credential custody, and the home of **datacenter-address validation**.
//!
//! # Why this shape
//!
//! The `Session` trait mixes *synchronous* hot-path methods (`home_dc_id`,
//! `dc_option` — called on every connection creation) with `BoxFuture` ones.
//! Nothing async can serve the sync half, so this type is an in-memory mirror
//! behind a [`std::sync::RwLock`], hydrated once at connect, serving sync reads
//! from memory, with write-through to custody afterwards. Writing per
//! `cache_peer` would be pathological — a peer cache fill would become a
//! compare-and-swap storm — so peer and update-cursor writes are **debounced**
//! while auth-key and home-datacenter writes are **immediate** (losing either
//! of those is a dead link or an orphaned device authorization).
//!
//! # The validation seam, and why it is airtight in `=0.10.0`
//!
//! `Session::dc_option` is the *only* consumer-owned seam on grammers' dial
//! path: `create_connection → session.dc_option(dc_id) → connect_sender →
//! TcpStream::connect`. There is no connector, dialer, or address-callback
//! injection point anywhere in 0.10.0, and `NetStream::connect` is
//! `pub(crate)`. What makes that a *control* rather than best-effort: 0.10.0's
//! `SenderPoolRunner::update_config` parses Telegram's server-pushed DC list
//! into a local `DcOption` and then **discards it — it never calls
//! `set_dc_option`**. So every address the dialer can ever reach comes from the
//! compiled-in table or from a value this module wrote, and validating here
//! gates 100% of dials.
//!
//! Upstream commit `5f94e83` ("Fix update_config did not set_dc_option") lands
//! *after* this release. The moment the pin moves past it, server-pushed
//! addresses start flowing into the session and this claim must be re-verified
//! — which is why `=0.10.0` is pinned exactly, in the manifest, as a security
//! control (PROPOSAL §3.4).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, Instant};

use grammers_session::types::{
    ChannelState, DcOption, PeerId, PeerInfo, UpdateState, UpdatesState,
};
use grammers_session::{BoxFuture, Session, SessionData};
use ironclaw_extension_contracts::linked_session::{
    LinkedSessionError, LinkedSessionPort, LinkedSessionVersion, SessionBytes,
};
use tokio::sync::Mutex as AsyncMutex;

use crate::linked::MAX_PEER_CACHE_ENTRIES;
use crate::linked::session_blob::{self, PersistedDcOption, PersistedSession, SessionBlobError};

/// The highest datacenter id Telegram's primary set uses. Options outside
/// `1..=MAX_KNOWN_DC_ID` are refused rather than dialled: the sanity check
/// PROPOSAL §3.4 asks for, without hard-pinning addresses.
const MAX_KNOWN_DC_ID: i32 = 5;

/// The only port a datacenter option may name.
const ALLOWED_DC_PORT: u16 = 443;

/// How long debounced writes coalesce before the next one reaches custody.
const FLUSH_DEBOUNCE: Duration = Duration::from_secs(5);

/// How many times a write-through will reload-merge-retry before giving up.
///
/// Bounded on purpose: retrying a lost compare-and-swap forever against a
/// livelocked peer is worse than failing loudly, and last-writer-wins is not
/// an option — a clobbered auth key kills the link.
const MAX_CAS_ATTEMPTS: u32 = 4;

/// Typed failures from the session mirror and its custody write-through.
///
/// No variant carries session bytes, and no message interpolates them.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionStoreError {
    #[error("the linked-session mirror lock was poisoned")]
    Poisoned,
    #[error("linked-session custody failed")]
    Custody(#[from] LinkedSessionError),
    #[error("linked-session blob is unusable")]
    Blob(#[from] SessionBlobError),
    #[error("telegram datacenter option was refused: {reason}")]
    RefusedDcOption { reason: &'static str },
    #[error("linked-session write lost the compare-and-swap {attempts} times running")]
    CasExhausted { attempts: u32 },
}

/// Session state, mirrored in memory.
#[derive(Debug)]
struct SessionMirror {
    home_dc: i32,
    dc_options: BTreeMap<i32, DcOption>,
    peers: HashMap<PeerId, PeerInfo>,
    /// Insertion order, for the bounded peer cache's FIFO eviction.
    peer_order: VecDeque<PeerId>,
    updates_state: UpdatesState,
}

impl Default for SessionMirror {
    /// The compiled-in datacenter table, exactly as a fresh grammers session
    /// would start. Taken from [`SessionData::default`] rather than restated,
    /// so the table cannot drift from the vendor crate's.
    fn default() -> Self {
        let data = SessionData::default();
        Self {
            home_dc: data.home_dc,
            dc_options: data.dc_options.into_iter().collect(),
            peers: HashMap::new(),
            peer_order: VecDeque::new(),
            updates_state: UpdatesState::default(),
        }
    }
}

/// Custody-attached state: the port and the debounce/compare-and-swap cursor.
struct Custody {
    port: Arc<dyn LinkedSessionPort>,
    write: AsyncMutex<WriteState>,
}

struct WriteState {
    version: LinkedSessionVersion,
    last_flush: Instant,
}

/// Whether a write-through may be coalesced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Urgency {
    /// Reaches custody now. Used for auth-key and home-datacenter changes:
    /// losing an auth key is a dead link, and losing a home-DC move after a
    /// migration reconnects to the wrong datacenter.
    Immediate,
    /// May coalesce with later writes inside [`FLUSH_DEBOUNCE`]. Used for the
    /// peer cache and the update cursor, both of which are reconstructible.
    Debounced,
}

/// A grammers session backed by IronClaw credential custody.
///
/// **`custody` is genuinely optional, and this is not the
/// `Option<Arc<…>>`-that-production-always-sets smell.** A device link runs
/// *before* any credential account exists — there is nothing to be scoped
/// against yet — so the login path builds an in-memory session
/// ([`IronclawSession::in_memory`]) and hands the finished blob to the
/// flow-scoped port once, at completion ([`IronclawSession::store_into`]). The
/// pooled path is attached from birth ([`IronclawSession::hydrate`]). Two
/// constructors, no builder, no `with_*`.
pub(crate) struct IronclawSession {
    mirror: RwLock<SessionMirror>,
    custody: Option<Custody>,
}

impl IronclawSession {
    /// A detached session for an in-progress login.
    pub(crate) fn in_memory() -> Arc<Self> {
        Arc::new(Self {
            mirror: RwLock::new(SessionMirror::default()),
            custody: None,
        })
    }

    /// Load a stored session, or start a fresh one when nothing is stored.
    ///
    /// Every persisted datacenter option is validated here, not merely on
    /// dial: a blob that would dial somewhere unacceptable must fail at load,
    /// where the failure is attributable, rather than at the first request.
    pub(crate) async fn hydrate(
        port: Arc<dyn LinkedSessionPort>,
    ) -> Result<Arc<Self>, SessionStoreError> {
        let snapshot = port.load().await?;
        let (mirror, version) = match snapshot {
            None => (SessionMirror::default(), LinkedSessionVersion::absent()),
            Some(snapshot) => {
                let persisted = session_blob::decode(snapshot.blob.expose())?;
                (SessionMirror::from_persisted(persisted)?, snapshot.version)
            }
        };
        Ok(Arc::new(Self {
            mirror: RwLock::new(mirror),
            custody: Some(Custody {
                port,
                write: AsyncMutex::new(WriteState {
                    version,
                    last_flush: Instant::now(),
                }),
            }),
        }))
    }

    /// Persist a detached session into a flow-scoped port for the first time.
    ///
    /// **Load-then-compare-and-swap, never absent-only** (PROPOSAL §4.5):
    /// relinking over the orphan blob a crashed prior attempt left behind must
    /// overwrite it, or a single crash bricks relinking forever.
    pub(crate) async fn store_into(
        &self,
        port: &dyn LinkedSessionPort,
    ) -> Result<(), SessionStoreError> {
        let expected = match port.load().await? {
            Some(snapshot) => snapshot.version,
            None => LinkedSessionVersion::absent(),
        };
        let bytes = session_blob::encode(&self.snapshot()?)?;
        port.save(expected, SessionBytes::new(bytes)?).await?;
        Ok(())
    }

    /// Whether this session holds a permanent authorization key for any
    /// datacenter.
    ///
    /// The cheapest honest test for "is this account actually linked?". A
    /// session with no key is indistinguishable from a fresh one, and dialling
    /// with it would negotiate a *new* unauthenticated key rather than fail —
    /// so the check has to happen before the connection, not after.
    pub(crate) fn holds_authorization_key(&self) -> bool {
        self.lock_read()
            .map(|mirror| {
                mirror
                    .dc_options
                    .values()
                    .any(|option| option.auth_key.is_some())
            })
            .unwrap_or(false)
    }

    /// Force any pending debounced write to custody.
    ///
    /// Called before a pooled entry is dropped and before a generation swap,
    /// so an old adapter's coalesced write cannot land after a new one has
    /// hydrated.
    pub(crate) async fn flush(&self) -> Result<(), SessionStoreError> {
        self.write_through(Urgency::Immediate).await
    }

    /// The mirror as a persisted document.
    fn snapshot(&self) -> Result<PersistedSession, SessionStoreError> {
        let mirror = self.lock_read()?;
        Ok(PersistedSession {
            v: session_blob::current_schema_version(),
            home_dc: mirror.home_dc,
            dc_options: mirror
                .dc_options
                .values()
                .map(PersistedDcOption::from_dc_option)
                .collect(),
            peers: mirror.peers.values().cloned().collect(),
            updates_state: mirror.updates_state.clone(),
        })
    }

    fn lock_read(&self) -> Result<RwLockReadGuard<'_, SessionMirror>, SessionStoreError> {
        self.mirror.read().map_err(|_| SessionStoreError::Poisoned)
    }

    fn lock_write(&self) -> Result<RwLockWriteGuard<'_, SessionMirror>, SessionStoreError> {
        self.mirror.write().map_err(|_| SessionStoreError::Poisoned)
    }

    /// Write the mirror to custody, merging on a lost compare-and-swap.
    ///
    /// The mirror lock is never held across the `await`: the snapshot is taken
    /// and the guard dropped before the port is touched.
    async fn write_through(&self, urgency: Urgency) -> Result<(), SessionStoreError> {
        let Some(custody) = self.custody.as_ref() else {
            return Ok(());
        };
        let mut write = custody.write.lock().await;
        if urgency == Urgency::Debounced && write.last_flush.elapsed() < FLUSH_DEBOUNCE {
            return Ok(());
        }

        for _ in 0..MAX_CAS_ATTEMPTS {
            let bytes = session_blob::encode(&self.snapshot()?)?;
            let blob = SessionBytes::new(bytes)?;
            match custody.port.save(write.version.clone(), blob).await {
                Ok(version) => {
                    write.version = version;
                    write.last_flush = Instant::now();
                    return Ok(());
                }
                Err(LinkedSessionError::VersionConflict { .. }) => {
                    // Reload rather than trusting the conflict's version alone:
                    // the merge needs the *content* that version describes, and
                    // pairing a version with content we never read is how a
                    // rotating auth key gets clobbered.
                    match custody.port.load().await? {
                        Some(snapshot) => {
                            let remote = session_blob::decode(snapshot.blob.expose())?;
                            self.lock_write()?.merge_remote(remote)?;
                            write.version = snapshot.version;
                        }
                        None => write.version = LinkedSessionVersion::absent(),
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(SessionStoreError::CasExhausted {
            attempts: MAX_CAS_ATTEMPTS,
        })
    }
}

impl SessionMirror {
    fn from_persisted(persisted: PersistedSession) -> Result<Self, SessionStoreError> {
        let mut dc_options = BTreeMap::new();
        for option in persisted.dc_options {
            let option = option.into_dc_option()?;
            validate_dc_option(&option)?;
            dc_options.insert(option.id, option);
        }
        // A blob that lost its datacenter table would otherwise dial nothing;
        // fill the gaps from the compiled-in set rather than failing a link the
        // user can still use.
        for (id, option) in SessionData::default().dc_options {
            dc_options.entry(id).or_insert(option);
        }

        let mut mirror = Self {
            home_dc: persisted.home_dc,
            dc_options,
            peers: HashMap::new(),
            peer_order: VecDeque::new(),
            updates_state: persisted.updates_state,
        };
        for peer in persisted.peers {
            mirror.remember_peer(peer);
        }
        Ok(mirror)
    }

    /// Insert or extend one cached peer, evicting the oldest non-self entry
    /// when the cache is full.
    ///
    /// The `Session` contract explicitly permits forgetting everything except
    /// the user where `is_self` is `Some(true)`, which is what makes a capped
    /// cache contract-legal.
    fn remember_peer(&mut self, peer: PeerInfo) {
        let id = peer.id();
        if let Some(existing) = self.peers.get_mut(&id) {
            existing.extend_info(&peer);
            return;
        }
        while self.peers.len() >= MAX_PEER_CACHE_ENTRIES {
            let Some(victim) = self.pop_evictable_peer() else {
                break;
            };
            self.peers.remove(&victim);
        }
        self.peer_order.push_back(id);
        self.peers.insert(id, peer);
    }

    /// The oldest cached peer that is not the logged-in user.
    fn pop_evictable_peer(&mut self) -> Option<PeerId> {
        let mut skipped = Vec::new();
        let victim = loop {
            let candidate = self.peer_order.pop_front()?;
            let is_self = matches!(
                self.peers.get(&candidate),
                Some(PeerInfo::User {
                    is_self: Some(true),
                    ..
                })
            );
            if is_self {
                skipped.push(candidate);
                continue;
            }
            break candidate;
        };
        for id in skipped.into_iter().rev() {
            self.peer_order.push_front(id);
        }
        Some(victim)
    }

    /// Fold a concurrently-written session into this one (PROPOSAL §5.1).
    ///
    /// Union the peer cache, take the maximum update cursor, and **never
    /// remove or replace an existing datacenter auth key** — local deltas win
    /// everywhere else, because they are the ones this process is currently
    /// speaking with.
    fn merge_remote(&mut self, remote: PersistedSession) -> Result<(), SessionStoreError> {
        for option in remote.dc_options {
            let option = option.into_dc_option()?;
            validate_dc_option(&option)?;
            match self.dc_options.get_mut(&option.id) {
                None => {
                    self.dc_options.insert(option.id, option);
                }
                Some(local) if local.auth_key.is_none() => local.auth_key = option.auth_key,
                Some(_) => {}
            }
        }

        for peer in remote.peers {
            self.remember_peer(peer);
        }

        let cursor = &mut self.updates_state;
        cursor.pts = cursor.pts.max(remote.updates_state.pts);
        cursor.qts = cursor.qts.max(remote.updates_state.qts);
        cursor.date = cursor.date.max(remote.updates_state.date);
        cursor.seq = cursor.seq.max(remote.updates_state.seq);
        for channel in remote.updates_state.channels {
            match cursor.channels.iter_mut().find(|c| c.id == channel.id) {
                Some(local) => local.pts = local.pts.max(channel.pts),
                None => cursor.channels.push(channel),
            }
        }
        Ok(())
    }

    fn apply_update_state(&mut self, update: UpdateState) {
        match update {
            UpdateState::All(state) => self.updates_state = state,
            UpdateState::Primary { pts, date, seq } => {
                self.updates_state.pts = pts;
                self.updates_state.date = date;
                self.updates_state.seq = seq;
            }
            UpdateState::Secondary { qts } => self.updates_state.qts = qts,
            UpdateState::Channel { id, pts } => {
                match self.updates_state.channels.iter_mut().find(|c| c.id == id) {
                    Some(channel) => channel.pts = pts,
                    None => self.updates_state.channels.push(ChannelState { id, pts }),
                }
            }
        }
    }
}

/// Refuse a datacenter option that would dial somewhere unacceptable.
///
/// This is the whole of the MTProto egress control (PROPOSAL §3.4): there is
/// no manifest allowlist, no SSRF check, and no host mediation on this path,
/// so what a dial may reach is decided here or nowhere.
fn validate_dc_option(option: &DcOption) -> Result<(), SessionStoreError> {
    let refuse = |reason: &'static str| SessionStoreError::RefusedDcOption { reason };

    if !(1..=MAX_KNOWN_DC_ID).contains(&option.id) {
        return Err(refuse("datacenter id is outside Telegram's primary set"));
    }
    if option.ipv4.port() != ALLOWED_DC_PORT || option.ipv6.port() != ALLOWED_DC_PORT {
        return Err(refuse(
            "datacenter port is not the allowlisted MTProto port",
        ));
    }
    validate_ipv4(option.ipv4.ip()).map_err(refuse)?;
    validate_ipv6(option.ipv6.ip()).map_err(refuse)?;
    Ok(())
}

fn validate_ipv4(address: &Ipv4Addr) -> Result<(), &'static str> {
    if address.is_unspecified() {
        return Err("datacenter address is unspecified");
    }
    if address.is_loopback() {
        return Err("datacenter address is loopback");
    }
    if address.is_private() {
        return Err("datacenter address is in private space");
    }
    if address.is_link_local() {
        return Err("datacenter address is link-local");
    }
    if address.is_multicast() {
        return Err("datacenter address is multicast");
    }
    if address.is_broadcast() {
        return Err("datacenter address is broadcast");
    }
    if address.is_documentation() {
        return Err("datacenter address is in documentation space");
    }
    Ok(())
}

fn validate_ipv6(address: &Ipv6Addr) -> Result<(), &'static str> {
    // A v4-mapped address dials the embedded IPv4, so it is judged as IPv4 —
    // otherwise `::ffff:127.0.0.1` would walk straight past the v6 checks.
    if let Some(mapped) = address.to_ipv4_mapped() {
        return validate_ipv4(&mapped);
    }
    if address.is_unspecified() {
        return Err("datacenter address is unspecified");
    }
    if address.is_loopback() {
        return Err("datacenter address is loopback");
    }
    if address.is_multicast() {
        return Err("datacenter address is multicast");
    }
    let leading = address.segments()[0];
    // `Ipv6Addr::is_unique_local` / `is_unicast_link_local` are still unstable,
    // so the two prefixes are matched directly: fc00::/7 and fe80::/10.
    if leading & 0xfe00 == 0xfc00 {
        return Err("datacenter address is unique-local");
    }
    if leading & 0xffc0 == 0xfe80 {
        return Err("datacenter address is link-local");
    }
    Ok(())
}

impl Session for IronclawSession {
    type Error = SessionStoreError;

    fn home_dc_id(&self) -> Result<i32, Self::Error> {
        Ok(self.lock_read()?.home_dc)
    }

    fn set_home_dc_id(&self, dc_id: i32) -> BoxFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            {
                self.lock_write()?.home_dc = dc_id;
            }
            self.write_through(Urgency::Immediate).await
        })
    }

    fn dc_option(&self, dc_id: i32) -> Result<Option<DcOption>, Self::Error> {
        let mirror = self.lock_read()?;
        let Some(option) = mirror.dc_options.get(&dc_id) else {
            // `None` is the contract's "not known", which the sender pool turns
            // into `InvalidDc`. Returning an error instead would abort
            // `update_config`'s harmless probe of every advertised datacenter.
            return Ok(None);
        };
        validate_dc_option(option)?;
        Ok(Some(option.clone()))
    }

    fn set_dc_option(&self, dc_option: &DcOption) -> BoxFuture<'_, Result<(), Self::Error>> {
        let dc_option = dc_option.clone();
        Box::pin(async move {
            validate_dc_option(&dc_option)?;
            let key_changed = {
                let mut mirror = self.lock_write()?;
                let previous = mirror
                    .dc_options
                    .insert(dc_option.id, dc_option.clone())
                    .and_then(|option| option.auth_key);
                previous != dc_option.auth_key
            };
            // A changed authorization key is the credential. Coalescing that
            // write would mean a crash inside the debounce window leaves
            // Telegram holding a device this deployment can no longer address.
            let urgency = if key_changed {
                Urgency::Immediate
            } else {
                Urgency::Debounced
            };
            self.write_through(urgency).await
        })
    }

    fn peer(&self, peer: PeerId) -> BoxFuture<'_, Result<Option<PeerInfo>, Self::Error>> {
        Box::pin(async move { Ok(self.lock_read()?.peers.get(&peer).cloned()) })
    }

    fn cache_peer(&self, peer: &PeerInfo) -> BoxFuture<'_, Result<(), Self::Error>> {
        let peer = peer.clone();
        Box::pin(async move {
            {
                self.lock_write()?.remember_peer(peer);
            }
            self.write_through(Urgency::Debounced).await
        })
    }

    fn updates_state(&self) -> BoxFuture<'_, Result<UpdatesState, Self::Error>> {
        Box::pin(async move { Ok(self.lock_read()?.updates_state.clone()) })
    }

    fn set_update_state(&self, update: UpdateState) -> BoxFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            {
                self.lock_write()?.apply_update_state(update);
            }
            self.write_through(Urgency::Debounced).await
        })
    }
}

#[cfg(test)]
#[path = "../tests/linked_session_store.rs"]
mod tests;
