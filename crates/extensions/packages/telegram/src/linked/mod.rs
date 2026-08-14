//! The **linked-device** half of the Telegram package: a user's own Telegram
//! account, reached over MTProto with a session credential the user granted by
//! scanning a code (or by typing a phone number and login code).
//!
//! This is deliberately a different thing from the rest of the package. The
//! `ChannelAdapter` in [`crate::channel`] speaks the *Bot API* over
//! host-mediated HTTP and never sees a token; the modules under here speak
//! *MTProto* over a raw socket and hold the account's own auth key in memory.
//! The design records that as an explicit carve-out with named compensating
//! controls (`docs/internal/design/telegram-linked-device/PROPOSAL.md` §3.4),
//! and the module split below *is* one of those controls:
//!
//! | Module | Owns | Never does |
//! | --- | --- | --- |
//! | [`transport`] | Every socket, the sender-pool runner, the read/write-aware retry wrapper | Decide what to send |
//! | [`session_store`] | `grammers_session::Session` over [`LinkedSessionPort`], **and DC-address validation** | Open a socket |
//! | [`session_blob`] | The persisted blob's encoding and merge shape | Anything else |
//! | [`login`] | The `DeviceLinkAdapter`: QR/phone handshakes and parked pending links | Hold the pool |
//! | [`pool`] | Live authenticated clients, keyed by credential account + link revision | Log in |
//! | [`mapping`] | Telegram shapes ↔ canonical op results | Invoke |
//!
//! **The socket confinement is checkable, not aspirational.** `transport.rs` is
//! the only file here that names `SenderPool`, `Client::new`, or
//! `tokio::spawn`; every other module reaches Telegram through
//! [`transport::MtprotoConnection`].
//!
//! [`LinkedSessionPort`]: ironclaw_extension_contracts::linked_session::LinkedSessionPort

use std::time::Duration;

use static_assertions::const_assert;

#[cfg(test)]
mod conformance;
pub(crate) mod login;
pub(crate) mod mapping;
pub(crate) mod ops;
pub(crate) mod pool;
pub(crate) mod session_blob;
pub(crate) mod session_store;
pub(crate) mod tools;
pub(crate) mod transport;

pub use login::{MtprotoAppIdentity, TelegramDeviceLinkAdapter};

/// `[admin_configuration]` field handle carrying the operator's MTProto
/// `api_id` (non-secret). `required = false`: a bot-only deployment keeps
/// activating and the device-link flow fails closed instead.
pub const TELEGRAM_API_ID_CONFIG: &str = "telegram_api_id";
/// `[admin_configuration]` field handle carrying the operator's MTProto
/// `api_hash` (`secret = true` — Telegram treats it as one).
pub const TELEGRAM_API_HASH_HANDLE: &str = "telegram_api_hash";
pub use pool::SessionPool;
pub use tools::TelegramLinkedToolAdapter;

// ---------------------------------------------------------------------------
// Bounds (PROPOSAL §7.2). Every constant here is a declared ceiling, not a
// tuning hint: an unbounded pool, an unbounded pending-link map, or an
// unbounded peer cache each turn a normal workload into a resource exhaustion,
// and the peer cache additionally grows the *persisted credential blob*.
// ---------------------------------------------------------------------------

/// Live authenticated MTProto sessions one process will hold at once.
///
/// Each costs a runner task plus its sockets, so this is the number the
/// hardening pass load-tests against — v1 targets 200 concurrent sessions per
/// process. At capacity the pool evicts its least-recently-used entry rather
/// than refusing: a miss reconnects from the stored blob, so eviction is a
/// latency cost, not a correctness one.
pub const MAX_POOLED_SESSIONS: usize = 200;

/// How long a pooled session may sit unused before the idle sweeper drops it.
pub const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// How often the idle sweeper runs. Deliberately coarse relative to
/// [`SESSION_IDLE_TIMEOUT`]: the sweep exists to release sockets, not to be
/// punctual.
pub const SESSION_IDLE_SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// In-progress device links one process will park at once.
///
/// A parked link holds a live connection and its runner task *before* any
/// credential exists, so this bound is what stops the flow from being a socket
/// amplifier.
pub const MAX_PENDING_LINKS: usize = 64;

/// How long a parked link survives without progress before it is reaped.
///
/// **Clock ordering matters** (PROPOSAL §4.3): `PENDING_LINK_TTL >= flow TTL >=
/// step TTL`. The adapter owns the innermost clock; the host owns the outer
/// two. Reaping a link whose flow the host still considers live turns a
/// recoverable poll into `Failed { restartable: true }`, which is why this is
/// the *longest* of the three.
pub const PENDING_LINK_TTL: Duration = Duration::from_secs(15 * 60);

/// Peers a session may cache before the oldest non-self entry is evicted.
///
/// `auto_cache_peers` defaults **true** in grammers and neither shipped storage
/// caps growth, so without this the persisted blob grows for the lifetime of
/// the link. The `Session` contract explicitly permits evicting everything
/// except the self user, so a capping implementation is contract-legal.
pub const MAX_PEER_CACHE_ENTRIES: usize = 1_000;

/// Wall-clock ceiling on one model-visible tool call against Telegram.
///
/// Consumed by the tool half; declared here because it is one of the §7.2
/// bounds and they are stated in one place. Note it does **not** bound unlink:
/// eviction never waits for an in-flight call — the generation fence
/// (`pool::LinkedAccountRevoker::revoke`) makes in-flight work fail at its next
/// RPC.
pub const TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(30);

// --- Content bounds (PROPOSAL §6.4, listed in §7.2) ------------------------
//
// The bounds above protect the process. These three protect the *transcript*.
// A personal Telegram account can be messaged by any Telegram user on earth,
// unsolicited and for free, so every byte a read returns is attacker-controlled
// text that lands verbatim in the durable turn transcript, the event log, and
// the model provider. Nothing upstream bounds that volume: the schema puts no
// `maxLength` on `text`, and the vendor decides how much it hands back.
//
// They are declared here, beside the other §7.2 bounds, because §7.2 is the
// one list that carries an each-tested rule — see the tests at the foot of this
// file. The enforcement sites are `mapping::sanitize_untrusted_text` (per
// message) and `mapping::bound_result_bytes` (per result); the per-page cap is
// applied by `ops::limit_arg`.

/// Ceiling on the serialized bytes of one read result's item list.
///
/// Applied by dropping trailing items — every list-shaped read here is
/// newest-first, so the oldest rows are the ones to lose. This is the
/// in-repo shape of #7397's channel-context clamp, which drops oldest lines
/// against its own declared byte ceiling.
pub const MAX_RESULT_BYTES: usize = 96 * 1024;

/// Ceiling on one message's (or one display name's) text, applied *before*
/// [`MAX_RESULT_BYTES`] so a single enormous message cannot evict a whole page
/// of context on its own.
pub const MAX_MESSAGE_TEXT_BYTES: usize = 4096;

/// Ceiling on the items one page of any list-shaped read may return, and the
/// clamp applied to a model-supplied `limit`.
pub const MAX_ITEMS_PER_PAGE: usize = 100;

/// How many vendor rows one call may walk while filling a page.
///
/// Bounds the pathological case where nearly every fetched row is filtered — a
/// chat of nothing but join/leave notices — and the loop would otherwise walk
/// the entire history looking for something to return.
pub const MAX_PAGES_PER_CALL: usize = 10;

// A zero bound is not a bound — it is a pool that never pools, a registry that
// never registers, and a peer cache that evicts the self-user. Checked at
// compile time rather than in a test, so it holds in every build.
const_assert!(MAX_POOLED_SESSIONS > 0);
const_assert!(MAX_PENDING_LINKS > 0);
const_assert!(MAX_PEER_CACHE_ENTRIES > 0);
const_assert!(MAX_RESULT_BYTES > 0);
const_assert!(MAX_MESSAGE_TEXT_BYTES > 0);
const_assert!(MAX_ITEMS_PER_PAGE > 0);
const_assert!(MAX_PAGES_PER_CALL > 0);

// The three content bounds are a set, not three independent numbers, and the
// ordering below is what makes each of them do work. Without it one of them is
// silently dead.
//
// One message must not be able to spend the whole result budget…
const_assert!(MAX_MESSAGE_TEXT_BYTES < MAX_RESULT_BYTES);
// …and a full page of maximum-length messages must exceed the byte ceiling,
// because otherwise the item cap already implies it and `bound_result_bytes`
// never runs — an unreachable clamp is one nobody notices breaking.
const_assert!(MAX_ITEMS_PER_PAGE * MAX_MESSAGE_TEXT_BYTES > MAX_RESULT_BYTES);
// The walk ceiling is a multiplier over the page size, so at 1 a page of
// filtered rows (a chat of nothing but join notices) returns empty rather than
// looking past them.
const_assert!(MAX_PAGES_PER_CALL > 1);

#[cfg(test)]
mod tests {
    use super::*;

    /// PROPOSAL §4.3's clock ordering, as a test rather than a comment: three
    /// clocks exist and they will disagree the moment nothing checks them.
    /// The adapter owns only the outermost, so the assertion here is that it
    /// is genuinely the outermost.
    #[test]
    fn pending_link_ttl_is_the_outermost_clock() {
        assert!(
            PENDING_LINK_TTL >= SESSION_IDLE_TIMEOUT,
            "a parked link must outlive an idle pooled session"
        );
        assert!(
            PENDING_LINK_TTL > TOOL_CALL_TIMEOUT,
            "a parked link must outlive a single tool call"
        );
    }

    #[test]
    fn sweep_interval_is_finer_than_the_idle_timeout() {
        assert!(
            SESSION_IDLE_SWEEP_INTERVAL < SESSION_IDLE_TIMEOUT,
            "a sweeper coarser than the timeout it enforces cannot enforce it"
        );
    }

    // The content bounds' *relationships* are the `const _: () = assert!(…)`
    // block above rather than tests down here: comparing two constants is not
    // a runtime fact, and a compile-time check holds in every build instead of
    // only where the test binary runs. What IS runtime about them — a caller
    // trying to raise the page cap, the per-message clamp on multi-byte text,
    // which rows the byte clamp keeps — is tested where the enforcement lives
    // (`ops::tests`, `mapping::tests`).
}
