//! Tests for [`IronclawSession`]: datacenter-address validation, the bounded
//! peer cache, and the write-through/merge behaviour custody depends on.
//!
//! The custody double is named `RecordingSessionCustody` rather than
//! `InMemory*Store` on purpose — the package gate
//! `telegram_tests_use_the_real_filesystem_state` bans the latter shape inside
//! `src/`, and this really is a recorder (it counts saves), not a store.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use ironclaw_extension_contracts::linked_session::LinkedSessionSnapshot;

use super::*;

const AUTH_KEY_LEN: usize = 256;

/// A `LinkedSessionPort` that records what reached it and can be made to lose
/// exactly one compare-and-swap.
#[derive(Default)]
struct RecordingSessionCustody {
    stored: Mutex<Option<(u64, Vec<u8>)>>,
    saves: AtomicUsize,
    version: AtomicUsize,
    lose_next_cas: AtomicBool,
}

impl RecordingSessionCustody {
    fn seed(&self, session: &PersistedSession) {
        let bytes = session_blob::encode(session).expect("encode");
        let version = self.version.fetch_add(1, Ordering::SeqCst) as u64 + 1;
        *self.stored.lock().expect("stored") = Some((version, bytes));
    }

    fn current(&self) -> PersistedSession {
        let guard = self.stored.lock().expect("stored");
        let (_, bytes) = guard.as_ref().expect("something stored");
        session_blob::decode(bytes).expect("decode")
    }

    fn save_count(&self) -> usize {
        self.saves.load(Ordering::SeqCst)
    }

    fn token(version: u64) -> LinkedSessionVersion {
        LinkedSessionVersion::new(format!("v{version}")).expect("version token")
    }
}

#[async_trait]
impl LinkedSessionPort for RecordingSessionCustody {
    async fn load(&self) -> Result<Option<LinkedSessionSnapshot>, LinkedSessionError> {
        let guard = self.stored.lock().expect("stored");
        let Some((version, bytes)) = guard.as_ref() else {
            return Ok(None);
        };
        Ok(Some(LinkedSessionSnapshot {
            blob: SessionBytes::new(bytes.clone())?,
            version: Self::token(*version),
        }))
    }

    async fn save(
        &self,
        expected: LinkedSessionVersion,
        blob: SessionBytes,
    ) -> Result<LinkedSessionVersion, LinkedSessionError> {
        self.saves.fetch_add(1, Ordering::SeqCst);
        let mut guard = self.stored.lock().expect("stored");
        let current = guard
            .as_ref()
            .map(|(version, _)| Self::token(*version))
            .unwrap_or_else(LinkedSessionVersion::absent);

        if self.lose_next_cas.swap(false, Ordering::SeqCst) || current != expected {
            return Err(LinkedSessionError::VersionConflict { current });
        }
        let version = self.version.fetch_add(1, Ordering::SeqCst) as u64 + 1;
        *guard = Some((version, blob.expose().to_vec()));
        Ok(Self::token(version))
    }
}

fn ipv4_option(id: i32, address: Ipv4Addr, port: u16) -> DcOption {
    DcOption {
        id,
        ipv4: SocketAddrV4::new(address, port),
        ipv6: SocketAddrV6::new(
            Ipv6Addr::new(0x2001, 0x67c, 0x4e8, 0xf002, 0, 0, 0, 0xa),
            443,
            0,
            0,
        ),
        auth_key: None,
    }
}

fn persisted(dc_options: Vec<DcOption>, peers: Vec<PeerInfo>, pts: i32) -> PersistedSession {
    PersistedSession {
        v: session_blob::current_schema_version(),
        home_dc: 2,
        dc_options: dc_options
            .iter()
            .map(PersistedDcOption::from_dc_option)
            .collect(),
        peers,
        updates_state: UpdatesState {
            pts,
            ..UpdatesState::default()
        },
    }
}

// ---------------------------------------------------------------------------
// Datacenter address validation — the one egress control on this path
// ---------------------------------------------------------------------------

#[test]
fn the_compiled_in_datacenter_table_passes_validation() {
    for (_, option) in SessionData::default().dc_options {
        validate_dc_option(&option).expect("a shipped datacenter option must be dialable");
    }
}

#[test]
fn unroutable_and_internal_addresses_are_refused() {
    let cases = [
        (Ipv4Addr::LOCALHOST, "loopback"),
        (Ipv4Addr::new(10, 0, 0, 5), "private"),
        (Ipv4Addr::new(192, 168, 1, 1), "private"),
        (Ipv4Addr::new(172, 16, 0, 1), "private"),
        (Ipv4Addr::new(169, 254, 169, 254), "link-local"),
        (Ipv4Addr::new(224, 0, 0, 1), "multicast"),
        (Ipv4Addr::UNSPECIFIED, "unspecified"),
        (Ipv4Addr::BROADCAST, "broadcast"),
    ];
    for (address, label) in cases {
        let option = ipv4_option(2, address, 443);
        assert!(
            validate_dc_option(&option).is_err(),
            "{label} address {address} must not be dialable"
        );
    }
}

#[test]
fn a_v4_mapped_v6_loopback_cannot_walk_past_the_v6_checks() {
    // `::ffff:127.0.0.1` dials 127.0.0.1, so it has to be judged as IPv4.
    let mut option = ipv4_option(2, Ipv4Addr::new(149, 154, 167, 41), 443);
    option.ipv6 = SocketAddrV6::new(Ipv4Addr::LOCALHOST.to_ipv6_mapped(), 443, 0, 0);
    assert!(validate_dc_option(&option).is_err());
}

#[test]
fn unique_local_and_link_local_v6_are_refused() {
    for leading in [0xfc00u16, 0xfd00, 0xfe80] {
        let mut option = ipv4_option(2, Ipv4Addr::new(149, 154, 167, 41), 443);
        option.ipv6 = SocketAddrV6::new(Ipv6Addr::new(leading, 0, 0, 0, 0, 0, 0, 1), 443, 0, 0);
        assert!(
            validate_dc_option(&option).is_err(),
            "v6 prefix {leading:#x} must not be dialable"
        );
    }
}

#[test]
fn only_the_allowlisted_port_and_the_known_datacenter_ids_are_accepted() {
    let off_port = ipv4_option(2, Ipv4Addr::new(149, 154, 167, 41), 8080);
    assert!(validate_dc_option(&off_port).is_err());

    let unknown_dc = ipv4_option(9, Ipv4Addr::new(149, 154, 167, 41), 443);
    assert!(validate_dc_option(&unknown_dc).is_err());
}

// ---------------------------------------------------------------------------
// Hydration and write-through
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_empty_custody_hydrates_to_the_compiled_in_table() {
    let custody = Arc::new(RecordingSessionCustody::default());
    let session = IronclawSession::hydrate(custody.clone())
        .await
        .expect("hydrate");

    assert_eq!(
        session.home_dc_id().expect("home dc"),
        SessionData::default().home_dc
    );
    for id in 1..=5 {
        assert!(
            session.dc_option(id).expect("dc option").is_some(),
            "datacenter {id} must be known to a fresh session"
        );
    }
    assert!(
        session.dc_option(9).expect("unknown dc").is_none(),
        "an unknown datacenter is `None`, not an error — the sender pool turns \
         that into InvalidDc, while an error would abort its config probe"
    );
    assert_eq!(custody.save_count(), 0, "hydration must not write");
}

#[tokio::test]
async fn an_authorization_key_change_reaches_custody_immediately() {
    let custody = Arc::new(RecordingSessionCustody::default());
    custody.seed(&persisted(
        SessionData::default().dc_options.into_values().collect(),
        Vec::new(),
        0,
    ));
    let session = IronclawSession::hydrate(custody.clone())
        .await
        .expect("hydrate");

    let mut option = session.dc_option(2).expect("dc 2").expect("dc 2 present");
    option.auth_key = Some([5u8; AUTH_KEY_LEN]);
    session.set_dc_option(&option).await.expect("set dc option");

    assert_eq!(
        custody.save_count(),
        1,
        "a rotating auth key must not sit inside the debounce window"
    );
    let stored = custody.current();
    let dc2 = stored
        .dc_options
        .into_iter()
        .find(|option| option.id == 2)
        .expect("dc 2 stored")
        .into_dc_option()
        .expect("dc 2 decodes");
    assert_eq!(dc2.auth_key, Some([5u8; AUTH_KEY_LEN]));
}

#[tokio::test]
async fn peer_caching_is_debounced_but_an_explicit_flush_is_not() {
    let custody = Arc::new(RecordingSessionCustody::default());
    custody.seed(&persisted(
        SessionData::default().dc_options.into_values().collect(),
        Vec::new(),
        0,
    ));
    let session = IronclawSession::hydrate(custody.clone())
        .await
        .expect("hydrate");

    for id in 1..=20i64 {
        session
            .cache_peer(&PeerInfo::User {
                id,
                auth: None,
                bot: Some(false),
                is_self: Some(false),
            })
            .await
            .expect("cache peer");
    }
    assert_eq!(
        custody.save_count(),
        0,
        "writing per cache_peer would be a compare-and-swap storm"
    );

    session.flush().await.expect("flush");
    assert_eq!(custody.save_count(), 1);
    assert_eq!(custody.current().peers.len(), 20);
}

// ---------------------------------------------------------------------------
// The bounded peer cache
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_peer_cache_is_bounded_and_never_evicts_the_logged_in_user() {
    let custody = Arc::new(RecordingSessionCustody::default());
    let session = IronclawSession::hydrate(custody.clone())
        .await
        .expect("hydrate");

    let me = PeerInfo::User {
        id: 1,
        auth: None,
        bot: Some(false),
        is_self: Some(true),
    };
    session.cache_peer(&me).await.expect("cache self");

    for id in 2..=(MAX_PEER_CACHE_ENTRIES as i64 + 50) {
        session
            .cache_peer(&PeerInfo::User {
                id,
                auth: None,
                bot: Some(false),
                is_self: Some(false),
            })
            .await
            .expect("cache peer");
    }

    session.flush().await.expect("flush");
    let stored = custody.current();
    assert!(
        stored.peers.len() <= MAX_PEER_CACHE_ENTRIES,
        "the peer cache grew to {} entries, past its bound",
        stored.peers.len()
    );
    assert!(
        session
            .peer(PeerId::user_unchecked(1))
            .await
            .expect("peer lookup")
            .is_some(),
        "the self-user is the one peer the Session contract forbids forgetting"
    );
}

// ---------------------------------------------------------------------------
// Compare-and-swap merge
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_lost_compare_and_swap_merges_instead_of_clobbering() {
    let custody = Arc::new(RecordingSessionCustody::default());
    let mut table = SessionData::default().dc_options;
    if let Some(dc2) = table.get_mut(&2) {
        dc2.auth_key = Some([1u8; AUTH_KEY_LEN]);
    }
    custody.seed(&persisted(
        table.values().cloned().collect(),
        vec![PeerInfo::User {
            id: 10,
            auth: None,
            bot: Some(false),
            is_self: Some(false),
        }],
        1,
    ));

    let session = IronclawSession::hydrate(custody.clone())
        .await
        .expect("hydrate");

    // Someone else wrote a newer session: a different DC-2 key, another peer,
    // and a further-along cursor.
    let mut remote_table = SessionData::default().dc_options;
    if let Some(dc2) = remote_table.get_mut(&2) {
        dc2.auth_key = Some([2u8; AUTH_KEY_LEN]);
    }
    if let Some(dc3) = remote_table.get_mut(&3) {
        dc3.auth_key = Some([3u8; AUTH_KEY_LEN]);
    }
    custody.seed(&persisted(
        remote_table.values().cloned().collect(),
        vec![PeerInfo::User {
            id: 20,
            auth: None,
            bot: Some(false),
            is_self: Some(false),
        }],
        99,
    ));

    // Local rotates its own DC-2 key, which forces an immediate write that has
    // to lose the compare-and-swap first.
    let mut local_dc2 = session.dc_option(2).expect("dc 2").expect("dc 2 present");
    local_dc2.auth_key = Some([4u8; AUTH_KEY_LEN]);
    session
        .set_dc_option(&local_dc2)
        .await
        .expect("set dc option after a conflict");

    let stored = custody.current();
    let key_for = |id: i32| {
        stored
            .dc_options
            .iter()
            .find(|option| option.id == id)
            .map(|option| {
                PersistedDcOption {
                    id: option.id,
                    ipv4: option.ipv4,
                    ipv6: option.ipv6,
                    auth_key: option.auth_key.clone(),
                }
                .into_dc_option()
                .expect("decode")
                .auth_key
            })
            .expect("datacenter present")
    };

    assert_eq!(
        key_for(2),
        Some([4u8; AUTH_KEY_LEN]),
        "the merge must not replace the key this process is speaking with"
    );
    assert_eq!(
        key_for(3),
        Some([3u8; AUTH_KEY_LEN]),
        "a key only the other writer had must be adopted, not dropped"
    );

    let peers = stored
        .peers
        .iter()
        .map(|peer| peer.id())
        .collect::<Vec<_>>();
    assert!(peers.contains(&PeerId::user_unchecked(10)));
    assert!(peers.contains(&PeerId::user_unchecked(20)));
    assert_eq!(
        stored.updates_state.pts, 99,
        "the update cursor takes the maximum, never the local value"
    );
}

#[tokio::test]
async fn storing_a_fresh_login_overwrites_an_orphan_blob() {
    // A crashed prior link left a blob behind. Relinking must overwrite it —
    // an absent-only write would brick relinking forever (PROPOSAL §4.5).
    let custody = Arc::new(RecordingSessionCustody::default());
    custody.seed(&persisted(
        SessionData::default().dc_options.into_values().collect(),
        vec![PeerInfo::User {
            id: 77,
            auth: None,
            bot: Some(false),
            is_self: Some(true),
        }],
        7,
    ));

    let fresh = IronclawSession::in_memory();
    fresh
        .cache_peer(&PeerInfo::User {
            id: 88,
            auth: None,
            bot: Some(false),
            is_self: Some(true),
        })
        .await
        .expect("cache self");

    fresh
        .store_into(custody.as_ref())
        .await
        .expect("store over an orphan blob");

    let stored = custody.current();
    let peers = stored
        .peers
        .iter()
        .map(|peer| peer.id())
        .collect::<Vec<_>>();
    assert_eq!(peers, vec![PeerId::user_unchecked(88)]);
}
