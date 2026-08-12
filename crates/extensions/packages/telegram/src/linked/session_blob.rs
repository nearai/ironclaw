//! The persisted linked-session blob: what the opaque bytes behind
//! [`LinkedSessionPort`] actually contain, and the only code that may read or
//! write that shape.
//!
//! **Why the package owns this and the auth domain does not.** Custody stores
//! opaque bytes on purpose — the domain that holds the ciphertext must not be
//! able to parse a vendor session. But compare-and-swap conflicts have to be
//! *merged*, and merging needs the structure. PROPOSAL §5.1 splits it exactly
//! there: `ironclaw_auth` owns conflict **detection** (CAS reject plus the
//! current version), and this package owns the semantic **merge**, because only
//! it can read the blob. This module is the "can read the blob" half.
//!
//! **Encoding: JSON with base64 auth keys, and that is a deviation worth
//! stating.** PROPOSAL §7.2 sized the blob against bincode-1 fixint (~762 B
//! after a fresh login, ~33 B per cached peer). `bincode` is not in this
//! package's pinned dependency set, so the blob rides `serde_json` instead,
//! with the 256-byte per-DC auth keys base64'd rather than hex'd (grammers'
//! own `serde` impl emits a 512-character hex string for them in *every*
//! format, binary included — §7.2 says take the raw encoding, and this is the
//! closest a text format gets). The consequence is arithmetic, not
//! architecture: the blob is a few times larger than §7.2's figures, and
//! [`MAX_PEER_CACHE_ENTRIES`] at 1,000 peers lands around 100 KB, comfortably
//! inside the 256 KiB [`MAX_LINKED_SESSION_BYTES`] ceiling. Re-derive both
//! numbers before changing either bound.
//!
//! [`LinkedSessionPort`]: ironclaw_extension_contracts::linked_session::LinkedSessionPort
//! [`MAX_PEER_CACHE_ENTRIES`]: crate::linked::MAX_PEER_CACHE_ENTRIES
//! [`MAX_LINKED_SESSION_BYTES`]: ironclaw_extension_contracts::linked_session::MAX_LINKED_SESSION_BYTES

use std::net::{SocketAddrV4, SocketAddrV6};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use grammers_session::types::{DcOption, PeerInfo, UpdatesState};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize as _;

/// Length of an MTProto permanent authorization key, in bytes.
const AUTH_KEY_BYTES: usize = 256;

/// Blob schema version. Bumped only when the *shape* changes; a reader that
/// meets a version it does not know refuses rather than guessing, because a
/// misparsed session is a silently dead link.
const BLOB_SCHEMA_VERSION: u8 = 1;

/// Failures that can come out of encoding or decoding a session blob.
///
/// No variant carries key material, and no message interpolates it — a blob
/// byte must never reach a log line, an event, or a panic message.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionBlobError {
    /// `reason` is category and position only, never blob content — see
    /// [`bounded_serde_reason`].
    #[error("linked-session blob is not valid JSON ({reason})")]
    Malformed { reason: String },
    #[error("linked-session blob declares schema version {found}, expected {expected}")]
    UnsupportedVersion { found: u8, expected: u8 },
    #[error("linked-session blob carries an authorization key of the wrong length")]
    BadAuthKey,
    #[error("linked-session blob could not be encoded ({reason})")]
    Encode { reason: String },
}

/// A serde failure reduced to its category and position.
///
/// serde_json's own `Display` can echo rejected *content*
/// (``invalid value: integer `…`, expected …``), and this file's charter is
/// that no blob byte reaches a log line — so the attribution a corrupt blob
/// gets is "what kind of failure, where", which is what a repair needs
/// anyway.
fn bounded_serde_reason(error: &serde_json::Error) -> String {
    format!(
        "{:?} error at line {} column {}",
        error.classify(),
        error.line(),
        error.column()
    )
}

/// The on-disk shape. Field names are short because the blob is written on
/// every auth-key rotation and read on every cold connect.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PersistedSession {
    /// Schema version — see [`BLOB_SCHEMA_VERSION`].
    pub(crate) v: u8,
    /// The datacenter the logged-in user is homed to.
    pub(crate) home_dc: i32,
    /// Known datacenter options, including any permanent auth keys.
    pub(crate) dc_options: Vec<PersistedDcOption>,
    /// Cached peers. A `Vec`, not a map: `PeerId` is a bit-packed `i64` and
    /// JSON has no integer keys, and [`PeerInfo::id`] recovers the key anyway.
    pub(crate) peers: Vec<PeerInfo>,
    /// The update cursor.
    pub(crate) updates_state: UpdatesState,
}

/// One datacenter option as persisted.
///
/// Mirrors [`DcOption`] rather than reusing it because grammers' own `serde`
/// impl hex-encodes the auth key at twice the size, and because a mirror here
/// is what lets the blob schema version independently of the vendor crate.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PersistedDcOption {
    pub(crate) id: i32,
    pub(crate) ipv4: SocketAddrV4,
    pub(crate) ipv6: SocketAddrV6,
    /// Base64 of the 256-byte permanent authorization key, when one exists.
    ///
    /// **This is the credential.** Anything holding it can speak as the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) auth_key: Option<String>,
}

impl PersistedDcOption {
    /// Project a live [`DcOption`] into its persisted shape.
    pub(crate) fn from_dc_option(option: &DcOption) -> Self {
        Self {
            id: option.id,
            ipv4: option.ipv4,
            ipv6: option.ipv6,
            auth_key: option.auth_key.map(|key| BASE64_STANDARD.encode(key)),
        }
    }

    /// Rebuild a live [`DcOption`], wiping the intermediate key buffers.
    ///
    /// The base64 string and the decoded vector both hold the auth key; both
    /// are zeroized here rather than left for the allocator, which is the only
    /// place in this file where that is not already handled by
    /// [`SessionBytes`]' own `Drop`.
    ///
    /// [`SessionBytes`]: ironclaw_extension_contracts::linked_session::SessionBytes
    pub(crate) fn into_dc_option(mut self) -> Result<DcOption, SessionBlobError> {
        let auth_key = match self.auth_key.take() {
            None => None,
            Some(mut encoded) => {
                let decoded = BASE64_STANDARD.decode(encoded.as_bytes());
                encoded.zeroize();
                let mut decoded = decoded.map_err(|_| SessionBlobError::BadAuthKey)?;
                if decoded.len() != AUTH_KEY_BYTES {
                    decoded.zeroize();
                    return Err(SessionBlobError::BadAuthKey);
                }
                let mut key = [0u8; AUTH_KEY_BYTES];
                key.copy_from_slice(&decoded);
                decoded.zeroize();
                Some(key)
            }
        };
        Ok(DcOption {
            id: self.id,
            ipv4: self.ipv4,
            ipv6: self.ipv6,
            auth_key,
        })
    }
}

/// Encode a session for custody.
///
/// Returns owned bytes so the caller can hand them straight to
/// `SessionBytes::new`, which takes ownership and zeroizes on drop — there is
/// deliberately no borrowed intermediate for a caller to forget about.
pub(crate) fn encode(session: &PersistedSession) -> Result<Vec<u8>, SessionBlobError> {
    serde_json::to_vec(session).map_err(|error| SessionBlobError::Encode {
        reason: bounded_serde_reason(&error),
    })
}

/// Decode a session from custody.
///
/// Refuses an unknown schema version instead of best-effort parsing: a
/// half-understood session is a link that fails later, at a vendor call, with
/// no way to attribute the failure.
pub(crate) fn decode(bytes: &[u8]) -> Result<PersistedSession, SessionBlobError> {
    let session: PersistedSession =
        serde_json::from_slice(bytes).map_err(|error| SessionBlobError::Malformed {
            reason: bounded_serde_reason(&error),
        })?;
    if session.v != BLOB_SCHEMA_VERSION {
        return Err(SessionBlobError::UnsupportedVersion {
            found: session.v,
            expected: BLOB_SCHEMA_VERSION,
        });
    }
    Ok(session)
}

/// The schema version a freshly encoded blob declares.
pub(crate) fn current_schema_version() -> u8 {
    BLOB_SCHEMA_VERSION
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn a_malformed_blob_error_carries_category_and_position_only() {
        // Before the fix the error was a bare `Malformed`, making a corrupt
        // custody blob unattributable — the exact outcome `decode`'s own doc
        // says it exists to avoid.
        let error = decode(b"{\"v\": \"not-a-number\"}").expect_err("must reject");
        let SessionBlobError::Malformed { reason } = &error else {
            panic!("expected Malformed, got {error:?}");
        };
        assert!(
            reason.contains("line") && reason.contains("column"),
            "reason must attribute the failure: {reason}"
        );
        // The charter half: the rejected content itself never rides along.
        assert!(
            !reason.contains("not-a-number"),
            "no blob byte may reach the error: {reason}"
        );
    }

    use grammers_session::types::PeerInfo;

    use super::*;

    fn dc(id: i32, auth_key: Option<[u8; AUTH_KEY_BYTES]>) -> DcOption {
        DcOption {
            id,
            ipv4: SocketAddrV4::new(Ipv4Addr::new(149, 154, 167, 41), 443),
            ipv6: SocketAddrV6::new(
                Ipv6Addr::new(0x2001, 0x67c, 0x4e8, 0xf002, 0, 0, 0, 0xa),
                443,
                0,
                0,
            ),
            auth_key,
        }
    }

    fn sample() -> PersistedSession {
        PersistedSession {
            v: BLOB_SCHEMA_VERSION,
            home_dc: 2,
            dc_options: vec![
                PersistedDcOption::from_dc_option(&dc(1, None)),
                PersistedDcOption::from_dc_option(&dc(2, Some([7u8; AUTH_KEY_BYTES]))),
            ],
            peers: vec![PeerInfo::User {
                id: 42,
                auth: None,
                bot: Some(false),
                is_self: Some(true),
            }],
            updates_state: UpdatesState {
                pts: 11,
                qts: 12,
                date: 13,
                seq: 14,
                channels: Vec::new(),
            },
        }
    }

    #[test]
    fn round_trips_including_the_auth_key() {
        let encoded = encode(&sample()).expect("encode");
        let decoded = decode(&encoded).expect("decode");
        assert_eq!(decoded.home_dc, 2);
        assert_eq!(decoded.peers.len(), 1);
        assert_eq!(decoded.updates_state.pts, 11);

        let mut options = decoded.dc_options.into_iter();
        let first = options
            .next()
            .expect("dc 1")
            .into_dc_option()
            .expect("dc 1");
        assert_eq!(first.auth_key, None);
        let second = options
            .next()
            .expect("dc 2")
            .into_dc_option()
            .expect("dc 2");
        assert_eq!(second.auth_key, Some([7u8; AUTH_KEY_BYTES]));
    }

    #[test]
    fn refuses_an_unknown_schema_version() {
        let mut session = sample();
        session.v = BLOB_SCHEMA_VERSION + 1;
        let encoded = encode(&session).expect("encode");
        assert!(matches!(
            decode(&encoded),
            Err(SessionBlobError::UnsupportedVersion { .. })
        ));
    }

    #[test]
    fn refuses_a_short_auth_key() {
        let option = PersistedDcOption {
            id: 2,
            ipv4: SocketAddrV4::new(Ipv4Addr::new(149, 154, 167, 41), 443),
            ipv6: SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 443, 0, 0),
            auth_key: Some(BASE64_STANDARD.encode([1u8; 8])),
        };
        assert!(matches!(
            option.into_dc_option(),
            Err(SessionBlobError::BadAuthKey)
        ));
    }

    #[test]
    fn refuses_bytes_that_are_not_a_blob() {
        assert!(matches!(
            decode(b"not json"),
            Err(SessionBlobError::Malformed { .. })
        ));
    }

    /// The §7.2 sizing claim, re-derived for *this* encoding rather than
    /// inherited from the bincode figures the proposal computed.
    #[test]
    fn a_full_peer_cache_stays_inside_the_custody_ceiling() {
        use ironclaw_extension_contracts::linked_session::MAX_LINKED_SESSION_BYTES;

        let mut session = sample();
        session.dc_options = (1..=5)
            .map(|id| PersistedDcOption::from_dc_option(&dc(id, Some([9u8; AUTH_KEY_BYTES]))))
            .collect();
        session.peers = (1..=crate::linked::MAX_PEER_CACHE_ENTRIES as i64)
            .map(|id| PeerInfo::User {
                id: id * 1_000_000,
                auth: Some(grammers_session::types::PeerAuth::from_hash(id * 7)),
                bot: Some(false),
                is_self: Some(false),
            })
            .collect();

        let encoded = encode(&session).expect("encode");
        assert!(
            encoded.len() < MAX_LINKED_SESSION_BYTES,
            "a full peer cache encodes to {} bytes, over the {MAX_LINKED_SESSION_BYTES}-byte \
             custody ceiling — lower MAX_PEER_CACHE_ENTRIES or change the encoding",
            encoded.len()
        );
    }
}
