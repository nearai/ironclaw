//! TLS termination seam for the sandbox egress proxy — W6 phase 1 (design
//! doc `docs/plans/2026-07-26-sandbox-credential-firewall-design.md` §4,
//! §3.4, §3.5).
//!
//! Carved out of `egress_proxy.rs` from day one per the design doc's own
//! guidance: that file already mixes DNS resolution, private-IP denial, and
//! plain-HTTP handling, and cert-minting + MITM termination is over the
//! thermo file-size/complexity ceiling for growing it further in place.
//!
//! **D1 (hard invariant):** an unbound host MUST stay an opaque
//! `copy_bidirectional` tunnel with NO leaf cert ever issued for it. This
//! module never mints or looks up a leaf certificate except from
//! [`terminate_and_forward`], and that function's `host` parameter is a
//! [`BoundHost`] — the only way to obtain one is
//! [`TlsInterceptConfig::bind`], which returns `None` for an unbound host —
//! so an unbound host cannot be *passed* to this module's cert-minting
//! path at all, not merely "shouldn't be" by caller convention.
//!
//! **Binding decision — phase 1 is a flat allowlist, not a binding model.**
//! [`TlsInterceptConfig`] carries a plain `HashSet<String>` of hosts this
//! proxy instance terminates TLS for. W12 (design doc §4) owns the real
//! binding model (provider-scoped child records, UI, validation); this phase
//! deliberately does not anticipate it — the design doc calls out per-command
//! or per-binding predicates as its own, separately-justified follow-up, not
//! something to build speculatively here.
//!
//! **Phase 1 scope: forward the decrypted stream unchanged.** No credential
//! injection, no body parsing — that is phase 2, gated on a `RuntimeKind::
//! Sandbox` variant that does not exist yet (design doc, W6 gating note).
//! Proving the interception mechanism works (real MITM, real fail-closed
//! behavior) stands on its own before any injection logic lands on top.
//!
//! **Fail closed.** Any failure — leaf mint, server handshake with the
//! client, origin dial, or origin handshake — closes the connection. There
//! is deliberately no code path from a [`TlsInterceptError`] back to a
//! plaintext `copy_bidirectional` fallback; `egress_proxy::handle_connect`
//! must not add one.
//!
//! **Not wired to a production caller yet.** [`TlsInterceptConfig`] has no
//! production constructor — nothing in this crate builds a
//! [`super::ca::SandboxCertificateAuthority`] or a real "trust the sandbox
//! egress network" `TlsConnector` today. `egress_proxy`'s proxy types carry
//! an `Option<Arc<TlsInterceptConfig>>` that production always leaves `None`
//! (see `EgressAllowlistProxy::new`), matching the same unwired-`Option<Arc<
//! ..>>` shape `attribution`'s resolver field already uses in this crate.

use std::{
    collections::HashSet,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use rustls::pki_types::ServerName;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf, copy_bidirectional},
    net::TcpStream,
};
use tokio_rustls::{TlsAcceptor, TlsConnector};

use super::ca::{LeafCertificate, SandboxCertificateAuthority, normalize_host};

/// This module builds every `rustls::ClientConfig`/`ServerConfig` through
/// `builder_with_provider(ring_crypto_provider())` — never the bare
/// `ClientConfig::builder()`/`ServerConfig::builder()`, which resolve
/// against whatever `CryptoProvider` happens to be installed as the
/// *process-global* default via `CryptoProvider::install_default`.
///
/// This module used to install a process-global default (`Once` +
/// `rustls::crypto::ring::default_provider().install_default()`, ignoring
/// the `Err` on a losing racer) on the premise that "every call path in
/// this process installs the same `ring` provider, so a second, ignored
/// install attempt is harmless." That premise does not hold: this binary
/// also links `reqwest`/`hyper-rustls` (via `ironclaw_network`), which pulls
/// in `aws-lc-rs` and can install *that* provider as the process-global
/// default before this module ever runs — at which point this module's own
/// install would silently lose the race, and its `ServerConfig`/
/// `ClientConfig::builder()` calls would resolve against `aws-lc-rs`
/// instead of `ring`, contrary to what this module's own (now-removed)
/// comment claimed. Not a security weakening (both providers are valid,
/// spec-compliant `rustls` backends), but exactly the "hidden
/// initialization order" hazard flagged in review: which provider ends up
/// active becomes a function of unrelated call ordering elsewhere in the
/// process. `builder_with_provider` sidesteps the whole class: no
/// process-global state is read or written, so this module's TLS configs
/// are deterministically `ring`-backed regardless of what else runs in the
/// same binary.
fn ring_crypto_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Errors from the TLS-termination seam. Every variant is a **fail-closed**
/// signal to the caller: `egress_proxy::handle_connect` treats any `Err`
/// here as "close the connection," never "fall back to a plaintext tunnel."
#[allow(dead_code)] // consumed by W6; not wired yet
#[derive(Debug, thiserror::Error)]
pub(crate) enum TlsInterceptError {
    #[error("sandbox tls intercept: failed to mint leaf certificate for {host}: {reason}")]
    LeafMintFailed { host: String, reason: String },
    #[error("sandbox tls intercept: failed to build server tls config: {0}")]
    ServerConfigFailed(String),
    #[error("sandbox tls intercept: client tls handshake failed: {0}")]
    ClientHandshakeFailed(String),
    #[error("sandbox tls intercept: failed to dial origin {dial_addr}: {reason}")]
    OriginDialFailed {
        dial_addr: SocketAddr,
        reason: String,
    },
    #[error("sandbox tls intercept: origin tls handshake failed: {0}")]
    OriginHandshakeFailed(String),
    #[error("sandbox tls intercept: invalid sni host {host:?}: {reason}")]
    InvalidSniHost { host: String, reason: String },
    #[error("sandbox tls intercept: relaying decrypted bytes failed: {0}")]
    RelayFailed(String),
    #[error("sandbox tls intercept: failed to load system trust roots: {0}")]
    TrustRootsUnavailable(String),
    #[error(
        "sandbox tls intercept: BoundHost for {host:?} was minted by a different \
         TlsInterceptConfig instance than the one passed to termination; refusing \
         to mint a leaf or dial the origin"
    )]
    ConfigMismatch { host: String },
}

/// A [`TlsConnector`] whose trust store is guaranteed to be the real
/// platform root-of-trust — never empty, never `dangerous()`, never a
/// custom verifier that skips or weakens certificate verification.
///
/// This wraps the invariant the struct-level `# WARNING` on
/// [`TlsInterceptConfig`] used to only document: `origin_connector` is what
/// the proxy uses to verify the origin it re-originates TLS to, on behalf
/// of a sandboxed container that is deliberately never given the real
/// secret. If that connector's trust store is ever empty or permissive, the
/// interception seam stops being a credential firewall and becomes a
/// working, silent MITM against our own users' egress traffic to every
/// bound host.
///
/// [`from_system_roots`](Self::from_system_roots) is the **only** door in a
/// production build — there is no way to build one from a caller-supplied
/// `TlsConnector`, `RootCertStore`, or verifier outside `#[cfg(test)]`. This
/// makes the mistake this type exists to prevent (an empty or permissive
/// connector reaching `TlsInterceptConfig::new`) a compile error for any
/// non-test caller, not merely a documented review requirement.
#[allow(dead_code)] // consumed by W6; not wired to a production caller yet
pub(crate) struct VerifiedOriginConnector(TlsConnector);

impl VerifiedOriginConnector {
    /// Builds the connector from the platform's real trust anchors via
    /// `rustls-native-certs` (the same crate and pattern
    /// `ironclaw_reborn_event_store::make_rustls_connector` already uses in
    /// this workspace for remote Postgres TLS). An empty or unreadable
    /// system trust store is a returned `Err`, never a silent `Ok` with
    /// zero roots — that empty-store case is exactly the bug this type
    /// exists to make unrepresentable, so it must fail closed rather than
    /// hand back a connector that verifies against nothing.
    #[allow(dead_code)] // consumed by W6; not wired to a production caller yet
    pub(crate) fn from_system_roots() -> Result<Self, TlsInterceptError> {
        let mut root_store = rustls::RootCertStore::empty();
        let native = rustls_native_certs::load_native_certs();
        for error in &native.errors {
            // `debug!`, not `warn!`/`info!`: this is an internal diagnostic on
            // a per-process background path, not intentionally user-facing
            // status, and a messy system trust store can emit one line per
            // unparseable root — `warn!`/`info!` here would corrupt the
            // REPL/TUI (see the crate logging-level rule). The empty-store
            // case still fails loudly via `TrustRootsUnavailable`.
            tracing::debug!("sandbox tls intercept: error loading system root certs: {error}");
        }
        for cert in native.certs {
            if let Err(error) = root_store.add(cert) {
                tracing::debug!(
                    "sandbox tls intercept: skipping invalid system root cert: {error}"
                );
            }
        }
        Self::from_root_store(root_store)
    }

    /// The fail-closed core `from_system_roots` delegates to: an empty root
    /// store — whether a genuinely bare system trust store or, in tests, a
    /// synthetic one — must never produce a connector that verifies against
    /// nothing. Split out so this branch is deterministically unit-testable
    /// without needing to fake `rustls_native_certs::load_native_certs`'s
    /// OS-level behavior.
    #[allow(dead_code)] // consumed by W6; not wired to a production caller yet
    fn from_root_store(root_store: rustls::RootCertStore) -> Result<Self, TlsInterceptError> {
        if root_store.is_empty() {
            return Err(TlsInterceptError::TrustRootsUnavailable(
                "system trust store yielded zero usable root certificates".to_string(),
            ));
        }
        // `builder_with_provider` (not the bare, process-default-resolving
        // `builder()`) — see `ring_crypto_provider`'s doc for why this
        // module never reads or writes process-global crypto-provider
        // state. `with_safe_default_protocol_versions` mirrors exactly what
        // `builder()` does internally, just with an explicit provider and a
        // propagated `Result` instead of an internal `.unwrap()`.
        let client_config = rustls::ClientConfig::builder_with_provider(ring_crypto_provider())
            .with_safe_default_protocol_versions()
            .map_err(|error| {
                TlsInterceptError::TrustRootsUnavailable(format!(
                    "failed to pair the ring crypto provider with safe default TLS protocol \
                     versions: {error}"
                ))
            })?
            .with_root_certificates(root_store)
            .with_no_client_auth();
        Ok(Self(TlsConnector::from(Arc::new(client_config))))
    }

    /// Test-only escape hatch: wrap an arbitrary connector (e.g. one
    /// trusting only a fake origin's root, or trusting nothing at all, to
    /// force the fail-closed path deterministically). `#[cfg(test)]` means
    /// this constructor does not exist in a production build — a
    /// production caller reaching for a permissive connector gets a
    /// compile error, not a review comment.
    #[cfg(test)]
    pub(crate) fn for_test(connector: TlsConnector) -> Self {
        Self(connector)
    }

    /// Named accessor for the wrapped connector, matching the crate's other
    /// newtypes (e.g. `IdempotencyKey::as_str`) instead of letting callers
    /// reach past the type via `.0` — the one call site
    /// ([`terminate_and_forward`]) goes through this, not the tuple field.
    fn connector(&self) -> &TlsConnector {
        &self.0
    }
}

/// Shared, per-proxy-instance TLS-interception configuration:
/// [`super::ca::SandboxCertificateAuthority`] to mint leaf certs from, the
/// flat set of hosts to terminate (see the module doc's "binding decision"),
/// and a [`VerifiedOriginConnector`] for re-originating a TLS connection to
/// the real upstream once decrypted.
///
/// # WARNING: `origin_connector`'s trust store is a production security
/// boundary, not a test convenience
///
/// This is now **type-enforced**, not just documented: [`new`](Self::new)
/// takes a [`VerifiedOriginConnector`], whose only production constructor
/// ([`VerifiedOriginConnector::from_system_roots`]) builds from the
/// platform's real trust anchors and fails closed on an empty or unreadable
/// store. The test-only escape hatch
/// ([`VerifiedOriginConnector::for_test`]) is `#[cfg(test)]`, so it does not
/// exist in a production build — there is no bare `TlsConnector` overload
/// for a production caller to reach for `dangerous()`,
/// `with_custom_certificate_verifier`, or an empty `RootCertStore` with.
///
/// The invariant this protects has not changed: this module re-originates a
/// TLS connection to the real upstream on behalf of the sandboxed
/// container, using the same host/port the container thought it was
/// dialing. If `origin_connector` ever fails to verify the origin's
/// certificate against a real root store, this seam stops being a
/// credential firewall and becomes a working, silent MITM against our own
/// users' egress traffic to every "bound" host — the exact opposite of what
/// W6 exists to build. `crates/ironclaw_architecture` also bans the escape
/// hatches (`dangerous(`, `with_custom_certificate_verifier`,
/// `RootCertStore::empty()`) from non-test code under
/// `sandbox_process/`, so a caller can no longer route around this type and
/// hand-roll a permissive connector either.
/// Process-wide counter minting a fresh [`ConfigIdentity`] for every
/// [`TlsInterceptConfig`] constructed. `Relaxed` ordering is sufficient — the
/// only property this counter needs is "every call returns a different
/// value than every other call within this process," not any ordering
/// relative to other memory, and identity comparison
/// ([`terminate_and_forward_core`]'s D1 check) only ever compares two
/// already-materialized `ConfigIdentity` values for equality.
static NEXT_CONFIG_IDENTITY: AtomicU64 = AtomicU64::new(0);

/// A [`TlsInterceptConfig`] instance's identity, minted once at
/// construction ([`TlsInterceptConfig::new`]) and never mutated. Exists
/// solely so a [`BoundHost`] can prove not just "some config allowed this
/// host" but "*this specific config instance* allowed this host" — see
/// `BoundHost`'s doc and [`terminate_and_forward_core`]'s D1 check for why
/// binding the proof to a bare host `String` alone was insufficient: two
/// configs can independently allowlist the identical host string, and a
/// `BoundHost` minted from one must still be rejected if passed to
/// termination alongside the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ConfigIdentity(u64);

impl ConfigIdentity {
    fn mint() -> Self {
        Self(NEXT_CONFIG_IDENTITY.fetch_add(1, Ordering::Relaxed))
    }
}

#[allow(dead_code)] // consumed by W6; not wired yet
pub(crate) struct TlsInterceptConfig {
    ca: SandboxCertificateAuthority,
    bound_hosts: HashSet<String>,
    origin_connector: VerifiedOriginConnector,
    identity: ConfigIdentity,
}

impl TlsInterceptConfig {
    #[allow(dead_code)] // constructed by this module's tests; a production caller is future wiring
    pub(crate) fn new(
        ca: SandboxCertificateAuthority,
        bound_hosts: HashSet<String>,
        origin_connector: VerifiedOriginConnector,
    ) -> Self {
        Self {
            ca,
            bound_hosts: bound_hosts
                .into_iter()
                .map(|host| normalize_host(&host))
                .collect(),
            origin_connector,
            identity: ConfigIdentity::mint(),
        }
    }

    /// D1's predicate: is `host` one this proxy instance terminates TLS for?
    /// Canonicalizes through the same [`normalize_host`] as
    /// [`super::ca::SandboxCertificateAuthority::issue_leaf_for_host`] and
    /// this module's own SNI conversion — case-insensitive (to match
    /// `egress_proxy::host_allowed`'s own normalization) and whitespace-
    /// insensitive, so this allowlist check, the leaf mint, and the SNI
    /// value sent to the origin can never disagree about which host is
    /// meant. Everything not in this set stays an opaque tunnel — see the
    /// module doc's D1 section.
    ///
    /// Kept as a free-standing `bool` predicate (rather than folded into
    /// [`bind`](Self::bind) alone) because `egress_proxy::handle_connect`'s
    /// routing decision — "does this CONNECT target get the TLS-termination
    /// path or the opaque tunnel path at all" — is a separate question from
    /// "hand me proof I can mint a leaf for it," and this module's own tests
    /// use it to assert the CA's leaf cache stays empty independent of ever
    /// calling [`terminate_and_forward`].
    #[allow(dead_code)] // consumed by W6; not wired yet
    pub(crate) fn is_bound(&self, host: &str) -> bool {
        self.bound_hosts.contains(&normalize_host(host))
    }

    /// D1 enforced by construction, not a runtime branch: the **only**
    /// production door to a [`BoundHost`]. Returns `None` for an unbound
    /// host instead of an `Err` a caller could `?`-propagate past — there is
    /// no `BoundHost` value [`terminate_and_forward`] could ever receive for
    /// a host this config does not terminate TLS for, so a future proxy
    /// implementation cannot forget the D1 check the way a redundant `if
    /// !config.is_bound(host) { return Err(..) }` guard could silently stop
    /// being reachable (e.g. if a caller upstream of it changes and no
    /// longer calls it) without anyone noticing. Same shape as
    /// [`VerifiedOriginConnector::from_system_roots`] above for the sibling
    /// invariant: one checked production constructor, a `#[cfg(test)]`-only
    /// escape hatch, no bare-`String` overload for a production caller to
    /// reach for instead.
    ///
    /// Routes through the identical [`normalize_host`] [`is_bound`](Self::
    /// is_bound) and [`terminate_and_forward_core`]'s leaf mint/SNI both
    /// use, so the allowlist check baked into the returned [`BoundHost`] and
    /// the value actually threaded through cert minting and SNI can never
    /// disagree about which host is meant.
    ///
    /// The returned [`BoundHost`] also carries `self`'s [`ConfigIdentity`] —
    /// D1's proof is scoped to *this specific config instance*, not merely
    /// "some config allowed this host": [`terminate_and_forward_core`]
    /// rejects a `BoundHost` whose identity does not match the config it is
    /// passed alongside, even if that other config's own allowlist happens
    /// to also contain the same host string.
    #[allow(dead_code)] // consumed by W6; not wired yet
    pub(crate) fn bind(&self, host: &str) -> Option<BoundHost> {
        let host = normalize_host(host);
        self.bound_hosts.contains(&host).then_some(BoundHost {
            host,
            config_identity: self.identity,
        })
    }

    /// Test/introspection seam: how many hosts this config's CA currently
    /// holds a cached leaf certificate for — D1's assertion surface for "an
    /// unbound host must never have a leaf minted for it," independent of
    /// whether traffic merely *looked* like it flowed correctly.
    #[cfg(test)]
    #[allow(dead_code)] // consumed by W6; not wired yet
    pub(crate) fn cached_leaf_count(&self) -> usize {
        self.ca.cached_entry_count()
    }

    /// This instance's [`ConfigIdentity`], minted once at construction.
    /// [`terminate_and_forward_core`]'s D1 check compares this against the
    /// identity carried by the [`BoundHost`] it was passed.
    fn identity(&self) -> ConfigIdentity {
        self.identity
    }
}

/// Proof that [`TlsInterceptConfig::bind`] confirmed a host is one this
/// proxy instance terminates TLS for (D1) — the wrapped `String` is already
/// the canonical [`normalize_host`] output. [`terminate_and_forward`] and
/// [`terminate_and_forward_with_timeout`] take this instead of `host: &str`
/// plus a separate `config: &TlsInterceptConfig`, so an unbound host cannot
/// be passed to either function at all: D1 becomes a compile-time property
/// of the call site instead of a runtime `if` those functions would have to
/// remember to check (and a future caller could forget to gate on) — see
/// [`TlsInterceptConfig::bind`]'s doc for why the type was chosen over an
/// inner guard.
///
/// [`TlsInterceptConfig::bind`] is the **only** door — there is no way to
/// build a `BoundHost` from a caller-supplied `&str` at all, not even under
/// `#[cfg(test)]`: every test in this module already has a
/// `TlsInterceptConfig` to `bind` through (unlike [`VerifiedOriginConnector`]
/// above, whose tests need a permissive connector `from_system_roots` can
/// never produce, `BoundHost` has no test scenario that needs to skip the
/// allowlist check, so it gets no escape hatch — the mirrored shape is "one
/// checked constructor, no bypass," not "always add a `#[cfg(test)]` door.")
///
/// Also carries the [`ConfigIdentity`] of the [`TlsInterceptConfig`] that
/// minted it. A bare `String` alone only proves "some config allowed this
/// host" — two independently-constructed configs can allowlist the exact
/// same host string, so a `BoundHost` minted from config A could otherwise
/// be passed to [`terminate_and_forward`] alongside a *different* config B,
/// which would mint/dial using B's CA and `origin_connector` even though B
/// never authorized this specific `BoundHost`. `config_identity` closes
/// that: [`terminate_and_forward_core`] rejects the call
/// (`TlsInterceptError::ConfigMismatch`, fail-closed, not a panic) unless it
/// matches the config actually passed in.
#[allow(dead_code)] // consumed by W6; not wired yet
pub(crate) struct BoundHost {
    host: String,
    config_identity: ConfigIdentity,
}

impl BoundHost {
    /// Named accessor for the wrapped host, matching this module's other
    /// newtype ([`VerifiedOriginConnector::connector`]) instead of letting
    /// callers reach past the type via `.0`.
    fn as_str(&self) -> &str {
        &self.host
    }

    /// The [`ConfigIdentity`] of the [`TlsInterceptConfig`] that minted this
    /// `BoundHost` via [`TlsInterceptConfig::bind`].
    fn config_identity(&self) -> ConfigIdentity {
        self.config_identity
    }
}

/// Bound applied to every handshake/dial leg of [`terminate_and_forward`]:
/// the client TLS accept, the origin TCP dial, and the origin TLS connect.
/// The client side of this seam is untrusted worker/container traffic — a
/// peer that opens the socket and then sends nothing (or half a
/// `ClientHello` and stalls) must not be able to pin this task and its
/// sockets open indefinitely. Deliberately does **not** bound
/// `copy_bidirectional`'s steady-state relay: an idle-timeout/byte-ceiling
/// policy for a live, decrypted proxy connection is a product decision (what
/// counts as "idle," whether a byte cap is even correct for a general HTTPS
/// relay that legitimately serves large downloads) that belongs with
/// whichever PR gives this seam a production caller and a concurrency/fan-out
/// policy to sit inside, not invented ad hoc here.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// Terminates TLS from `client` using a leaf certificate minted for `host`,
/// dials `dial_addr` and re-originates TLS to the real upstream (SNI =
/// `host`), then relays the decrypted bytes unmodified (phase 1: no parsing,
/// no injection — see the module doc). `leftover` is whatever bytes the
/// egress proxy's `BufReader` had already buffered past the CONNECT
/// request/`200` reply (the same "eager client" case `egress_proxy`'s own
/// tunnel path already has to handle) — fed to the TLS acceptor before any
/// further bytes are read off the socket, via [`LeadingBytes`].
///
/// `host` is a [`BoundHost`], not a bare `&str`: D1 ("an unbound host stays
/// an opaque tunnel, never a leaf mint") is enforced by this signature
/// itself — the only way to produce a `BoundHost` in a production build is
/// [`TlsInterceptConfig::bind`], so a caller can no longer forget to gate on
/// `is_bound` before reaching this function. See [`TlsInterceptConfig::
/// bind`]'s doc for why that is a type instead of an inner `if`.
///
/// Every failure path returns `Err` and touches neither `client` nor an
/// origin socket again — no code path here ever falls through to a
/// plaintext relay. `egress_proxy::handle_connect` must preserve that: log
/// and close, never retry unencrypted.
#[allow(dead_code)] // consumed by W6; not wired yet
pub(crate) async fn terminate_and_forward(
    client: TcpStream,
    leftover: Vec<u8>,
    host: BoundHost,
    dial_addr: SocketAddr,
    config: &TlsInterceptConfig,
) -> Result<(), TlsInterceptError> {
    terminate_and_forward_with_timeout(client, leftover, host, dial_addr, config, HANDSHAKE_TIMEOUT)
        .await
}

/// The timeout-parameterized core `terminate_and_forward` delegates to with
/// [`HANDSHAKE_TIMEOUT`] — split out so tests can drive the timeout branch
/// deterministically with a short real duration instead of either sleeping
/// [`HANDSHAKE_TIMEOUT`] wall-clock seconds or fighting tokio's paused/
/// advanceable virtual clock against a task that also does real loopback
/// socket I/O.
#[allow(dead_code)] // consumed by W6; not wired yet
async fn terminate_and_forward_with_timeout(
    client: TcpStream,
    leftover: Vec<u8>,
    host: BoundHost,
    dial_addr: SocketAddr,
    config: &TlsInterceptConfig,
    handshake_timeout: Duration,
) -> Result<(), TlsInterceptError> {
    terminate_and_forward_core(
        client,
        leftover,
        host,
        dial_addr,
        config,
        handshake_timeout,
        build_sni_server_name,
    )
    .await
}

/// Test-only entry point that reaches every step of
/// [`terminate_and_forward_core`] exactly like production — same
/// normalization, same leaf mint, same client handshake — but forces the
/// SNI-conversion step to fail instead of calling the real
/// [`build_sni_server_name`]. This exists to keep
/// `invalid_sni_host_fails_before_the_origin_is_dialed` honest once the
/// leaf-mint/SNI-dial canonicalization asymmetry it used to exercise is
/// fixed: with both steps sharing [`normalize_host`], no input can any
/// longer pass the leaf mint and still fail a real `ServerName::try_from`
/// (see `ca.rs::validate_dns_host`'s doc), so there is no remaining runtime
/// input that reproduces the original bug's *symptom*. This seam instead
/// pins the *ordering* directly: whatever makes the SNI step return `Err`,
/// that `Err` must still surface before `TcpStream::connect(dial_addr)`
/// ever runs, because both this function and production route through the
/// identical `terminate_and_forward_core` control flow — only the SNI
/// builder closure differs. `#[cfg(test)]` means this function does not
/// exist in a production build.
#[cfg(test)]
async fn terminate_and_forward_with_forced_sni_failure(
    client: TcpStream,
    leftover: Vec<u8>,
    host: BoundHost,
    dial_addr: SocketAddr,
    config: &TlsInterceptConfig,
    handshake_timeout: Duration,
) -> Result<(), TlsInterceptError> {
    terminate_and_forward_core(
        client,
        leftover,
        host,
        dial_addr,
        config,
        handshake_timeout,
        |host| {
            Err(TlsInterceptError::InvalidSniHost {
                host: host.to_string(),
                reason: "forced by test to pin the pre-dial ordering".to_string(),
            })
        },
    )
    .await
}

/// The shared core both [`terminate_and_forward_with_timeout`] (production,
/// via the real [`build_sni_server_name`]) and the test-only
/// [`terminate_and_forward_with_forced_sni_failure`] (a forced-failure
/// closure) drive. `sni_server_name` is the *only* thing that differs
/// between the two callers — every other step, and critically the
/// leaf-mint-then-SNI-then-dial ordering itself, is identical code, so a
/// test using the forced closure is exercising the real ordering, not a
/// reimplementation of it.
async fn terminate_and_forward_core<F>(
    client: TcpStream,
    leftover: Vec<u8>,
    host: BoundHost,
    dial_addr: SocketAddr,
    config: &TlsInterceptConfig,
    handshake_timeout: Duration,
    sni_server_name: F,
) -> Result<(), TlsInterceptError>
where
    F: FnOnce(&str) -> Result<ServerName<'static>, TlsInterceptError>,
{
    // D1, scoped to the specific config instance: `host` proves "some
    // config allowed this host," not "THIS config allowed this host" —
    // two independently-constructed configs can allowlist the identical
    // host string. Reject before any side effect (leaf mint, dial) if the
    // `BoundHost` was minted by a different `TlsInterceptConfig` than the
    // one passed here. Fail closed with an error, never a panic — see
    // `BoundHost`'s and `TlsInterceptError::ConfigMismatch`'s docs.
    if host.config_identity() != config.identity() {
        return Err(TlsInterceptError::ConfigMismatch {
            host: host.as_str().to_string(),
        });
    }

    // `host` arrives already canonicalized — `BoundHost` only exists via
    // `TlsInterceptConfig::bind`, which normalizes through the same
    // `normalize_host` this used to call directly here. Use that SAME
    // canonical value for both the leaf mint below and the SNI value
    // threaded to the origin dial further down: this is what closes the
    // asymmetry `normalize_host`'s own doc describes — `issue_leaf_for_host`
    // used to canonicalize its own copy of `host` while this function
    // passed the original, un-normalized parameter to `ServerName::
    // try_from` — a padded host could mint a leaf and complete the client
    // handshake, then fail SNI conversion because whitespace is not a
    // valid DNS byte. `issue_leaf_for_host` still normalizes again
    // internally (it must stay safe to call on its own), but that second
    // normalization is now idempotent on an already-canonical string, not
    // the only normalization in the whole path.
    let host = host.as_str().to_string();
    let issued = config.ca.issue_leaf_for_host(&host).map_err(|error| {
        TlsInterceptError::LeafMintFailed {
            host: host.clone(),
            reason: error.to_string(),
        }
    })?;
    tracing::debug!(
        host = %issued.certificate.host,
        cache_hit = issued.cache_hit,
        "sandbox tls intercept: leaf certificate ready"
    );

    let server_config = build_server_config(&issued.certificate)?;
    let acceptor = TlsAcceptor::from(Arc::new(server_config));
    let client_with_leftover = LeadingBytes::new(leftover, client);
    let mut client_tls =
        tokio::time::timeout(handshake_timeout, acceptor.accept(client_with_leftover))
            .await
            .map_err(|_| {
                TlsInterceptError::ClientHandshakeFailed(format!(
                    "client handshake timed out after {handshake_timeout:?}"
                ))
            })?
            .map_err(|error| TlsInterceptError::ClientHandshakeFailed(error.to_string()))?;

    // Only reachable once the client trusts our leaf and completed its
    // handshake — a client-side failure above never gets this far, so an
    // unbound/failed interception never opens an origin socket either.
    //
    // Validate the SNI host BEFORE dialing the origin. `host` here is the
    // same canonicalized value the leaf was minted for above (see the
    // normalization note at the top of this function), constructed via
    // `sni_server_name` (production: `build_sni_server_name`, delegating to
    // `ServerName::try_from`; test-only: a forced-failure closure — see
    // `terminate_and_forward_with_forced_sni_failure`). Validating before
    // dialing means an invalid host never causes any origin-directed
    // network activity at all: dialing first would open a real outbound
    // TCP connection to an attacker-influenced host that is about to be
    // rejected anyway.
    let server_name = sni_server_name(&host)?;
    let origin_stream = tokio::time::timeout(handshake_timeout, TcpStream::connect(dial_addr))
        .await
        .map_err(|_| TlsInterceptError::OriginDialFailed {
            dial_addr,
            reason: format!("dial timed out after {handshake_timeout:?}"),
        })?
        .map_err(|error| TlsInterceptError::OriginDialFailed {
            dial_addr,
            reason: error.to_string(),
        })?;
    let mut origin_tls = tokio::time::timeout(
        handshake_timeout,
        config
            .origin_connector
            .connector()
            .connect(server_name, origin_stream),
    )
    .await
    .map_err(|_| {
        TlsInterceptError::OriginHandshakeFailed(format!(
            "origin handshake timed out after {handshake_timeout:?}"
        ))
    })?
    .map_err(|error| TlsInterceptError::OriginHandshakeFailed(error.to_string()))?;

    copy_bidirectional(&mut client_tls, &mut origin_tls)
        .await
        .map_err(|error| TlsInterceptError::RelayFailed(error.to_string()))?;
    Ok(())
}

/// Builds the `ServerName` used for the origin TLS handshake from the
/// already-canonicalized `host` (see [`terminate_and_forward_core`]'s
/// normalization note). The only production implementation of the
/// `sni_server_name` step `terminate_and_forward_core` takes as a
/// parameter — split out to a named function (rather than an inline
/// closure at the one production call site) purely so its doc comment has
/// somewhere to live and so the test-only forced-failure variant in
/// `terminate_and_forward_with_forced_sni_failure` reads as an obvious,
/// deliberate substitution for this specific function, not a divergent
/// reimplementation.
fn build_sni_server_name(host: &str) -> Result<ServerName<'static>, TlsInterceptError> {
    ServerName::try_from(host.to_string()).map_err(|error| TlsInterceptError::InvalidSniHost {
        host: host.to_string(),
        reason: error.to_string(),
    })
}

/// Builds a single-host rustls server config serving exactly the leaf
/// certificate minted for one host — no SNI-keyed multi-cert resolver is
/// needed because a CONNECT tunnel already pins the intended host before
/// this is called (see [`terminate_and_forward`]); the client's SNI, if
/// present, is not consulted.
#[allow(dead_code)] // consumed by W6; not wired yet
pub(crate) fn build_server_config(
    leaf: &LeafCertificate,
) -> Result<rustls::ServerConfig, TlsInterceptError> {
    let chain = CertificateDer::pem_slice_iter(leaf.cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            TlsInterceptError::ServerConfigFailed(format!("parsing leaf cert pem: {error}"))
        })?;
    // `PrivateKeyDer::from_pem_slice` (unlike `rustls_pemfile::private_key`'s
    // `Result<Option<_>>`) already returns `Err(pem::Error::NoItemsFound)`
    // when the PEM contains no key — no separate `None` case to handle, and
    // still fails closed exactly the same as the explicit `ok_or_else` this
    // replaces.
    let key = PrivateKeyDer::from_pem_slice(leaf.key_pem.as_bytes()).map_err(|error| {
        TlsInterceptError::ServerConfigFailed(format!("parsing leaf key pem: {error}"))
    })?;

    // `builder_with_provider` (not the bare, process-default-resolving
    // `builder()`) — see `ring_crypto_provider`'s doc.
    rustls::ServerConfig::builder_with_provider(ring_crypto_provider())
        .with_safe_default_protocol_versions()
        .map_err(|error| {
            TlsInterceptError::ServerConfigFailed(format!(
                "failed to pair the ring crypto provider with safe default TLS protocol \
                 versions: {error}"
            ))
        })?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(|error| TlsInterceptError::ServerConfigFailed(error.to_string()))
}

/// Wraps an `AsyncRead + AsyncWrite` stream with bytes that must be replayed
/// to the first reader before any further bytes are read off the underlying
/// stream. Mirrors the same "bytes buffered alongside the CONNECT request"
/// case `egress_proxy::handle_connect`'s plaintext tunnel path already
/// handles (see that module's `connect_forwards_bytes_buffered_alongside_
/// the_connect_request` test) — a client that doesn't wait for the proxy's
/// `200 Connection Established` before starting its TLS handshake can have
/// the start of its `ClientHello` land in the same TCP segment as the
/// CONNECT request, which ends up sitting in the proxy's `BufReader` rather
/// than the socket. Writes always delegate straight to the inner stream —
/// only reads need the replay.
#[allow(dead_code)] // consumed by W6; not wired yet
struct LeadingBytes<S> {
    leftover: Vec<u8>,
    leftover_pos: usize,
    inner: S,
}

impl<S> LeadingBytes<S> {
    #[allow(dead_code)] // consumed by W6; not wired yet
    fn new(leftover: Vec<u8>, inner: S) -> Self {
        Self {
            leftover,
            leftover_pos: 0,
            inner,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for LeadingBytes<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.leftover_pos < this.leftover.len() {
            let remaining = &this.leftover[this.leftover_pos..];
            let take = remaining.len().min(buf.remaining());
            buf.put_slice(&remaining[..take]);
            this.leftover_pos += take;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for LeadingBytes<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests;
