//! Internal per-tenant CA for the sandbox egress proxy (W5 — design doc
//! `docs/internal/plans/2026-07-26-sandbox-credential-firewall-design.md` §4).
//!
//! Generates a root key/cert pair **in memory only**, at construction, and
//! signs short-lived leaf certificates for hosts the credential firewall
//! (W6) intercepts. The root private key never touches disk, is never
//! serialized anywhere this module returns to a caller, and is never part
//! of anything mounted into a container — only
//! [`SandboxCertificateAuthority::root_certificate_pem`] (the public trust
//! anchor) is meant to reach the container filesystem, as a read-only
//! bind mount (W5's remaining trust-distribution work, see the design
//! doc's `update-ca-certificates` note). This is the same "secret material
//! never enters the container, in any form, even transiently" invariant
//! the rest of the credential firewall enforces, applied to the CA itself.
//!
//! **W6 is the consumer, not built yet.** Nothing in this crate calls
//! [`SandboxCertificateAuthority`] today; the proxy's TLS termination for
//! bound hosts will call [`SandboxCertificateAuthority::issue_leaf_for_host`]
//! per intercepted CONNECT, per the design doc's D1/W6 gating.

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
    time::{Duration, Instant},
};

use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use time::{Duration as CertValidityDuration, OffsetDateTime};

use ironclaw_host_api::process::RuntimeProcessError;

/// Default validity window for a leaf certificate, and the default cache
/// TTL — short by design so a leaked leaf key (a mounted-into-container
/// artifact, unlike the root) is only useful for a bounded window. The
/// cache TTL intentionally matches the cert's own validity: caching a leaf
/// past its `not_after` would just hand back an already-expired cert.
#[allow(dead_code)] // consumed by W6; not wired yet
pub(crate) const DEFAULT_LEAF_TTL: Duration = Duration::from_secs(5 * 60);

/// Clock-skew allowance applied to `not_before` so a cert issued a moment
/// ago is never rejected as "not yet valid" by a container whose clock
/// runs slightly behind the host's.
const NOT_BEFORE_SKEW: CertValidityDuration = CertValidityDuration::minutes(5);

/// Root CA validity window. The root is regenerated fresh in memory on
/// every process start (see the module doc and [`SandboxCertificateAuthority::generate`]),
/// so this only has to comfortably outlive one process's lifetime — cross-restart
/// rotation is W6/operational wiring, not this unwired primitive's job.
const ROOT_VALIDITY: CertValidityDuration = CertValidityDuration::days(30);

/// Upper bound on the number of distinct hosts a CA instance caches leaf
/// certificates for at once. Bounded so a proxy terminating TLS for an
/// unbounded number of distinct SNI hosts cannot grow this cache without
/// limit; the oldest-issued entry is evicted first once the bound is hit.
#[allow(dead_code)] // consumed by W6; not wired yet
pub(crate) const DEFAULT_MAX_CACHE_ENTRIES: usize = 256;

/// A leaf certificate issued for exactly one host: PEM cert + PEM private
/// key, both the *leaf's* material only. The root private key never
/// appears in either field.
#[derive(Clone)]
#[allow(dead_code)] // consumed by W6; not wired yet
pub(crate) struct LeafCertificate {
    pub(crate) host: String,
    pub(crate) cert_pem: String,
    pub(crate) key_pem: String,
}

impl std::fmt::Debug for LeafCertificate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately omit `key_pem`: it is the leaf's private key material
        // and must never persist in logs or panic output, even though (unlike
        // the root key) it is a bounded, short-lived, container-scoped
        // artifact this design otherwise accepts.
        formatter
            .debug_struct("LeafCertificate")
            .field("host", &self.host)
            .field("cert_pem", &self.cert_pem)
            .finish_non_exhaustive()
    }
}

/// [`SandboxCertificateAuthority::issue_leaf_for_host`]'s result. Exposes
/// whether the leaf came from the bounded cache or was freshly minted —
/// useful to a future caller deciding whether a live connection needs a
/// newly-issued cert pushed to it, and to this module's own cache tests.
#[derive(Debug, Clone)]
#[allow(dead_code)] // consumed by W6; not wired yet
pub(crate) struct IssuedLeaf {
    pub(crate) certificate: LeafCertificate,
    pub(crate) cache_hit: bool,
}

struct CachedLeaf {
    // Not `Arc`-wrapped: nothing here shares this pointer between callers —
    // `cached_leaf` immediately dereferences and clones the value out, and
    // the fresh-issuance path already clones before inserting. Plain
    // ownership avoids indirection that solves no aliasing problem.
    leaf: LeafCertificate,
    inserted_at: Instant,
    /// When this cache entry stops being live, in this process's monotonic
    /// clock. Computed once at mint time from the leaf's *actual* (possibly
    /// root-capped, see [`SandboxCertificateAuthority::mint_leaf`]) `not_after`
    /// — never derived from `leaf_ttl` alone. A leaf minted late in the
    /// root's life can have a real certificate lifetime shorter than
    /// `leaf_ttl`; keying cache expiry off `leaf_ttl` directly would let the
    /// cache keep serving that leaf after its certificate (and root) have
    /// actually expired.
    expires_at: Instant,
}

/// In-memory root CA plus a bounded, TTL-scoped cache of per-host leaf
/// certificates. See the module doc for the "root key never leaves the
/// host" invariant this type exists to uphold: the root signing key lives
/// only in the private `issuer` field below, and no method on this type
/// returns it.
#[allow(dead_code)] // consumed by W6; not wired yet
pub(crate) struct SandboxCertificateAuthority {
    root_cert_pem: String,
    issuer: Issuer<'static, KeyPair>,
    /// The root's own `not_after`. A leaf's validity must never outlive its
    /// trust anchor — see [`Self::mint_leaf`]'s cap — so this is kept
    /// alongside `issuer` (whose `params` field, carrying the same value,
    /// is moved-from and no longer independently readable once wrapped in
    /// `Issuer`).
    root_not_after: OffsetDateTime,
    leaf_ttl: Duration,
    max_cache_entries: usize,
    cache: Mutex<HashMap<String, CachedLeaf>>,
}

impl std::fmt::Debug for SandboxCertificateAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately omit `issuer` (carries the root private key —
        // `rcgen::Issuer`'s own `Debug` already elides key material, but
        // this type never even calls that impl) and `cache` (leaf private
        // keys are also secret-shaped). Neither belongs in a log line.
        formatter
            .debug_struct("SandboxCertificateAuthority")
            .field("leaf_ttl", &self.leaf_ttl)
            .field("max_cache_entries", &self.max_cache_entries)
            .finish_non_exhaustive()
    }
}

impl SandboxCertificateAuthority {
    /// Generates a fresh root key + self-signed CA certificate **in
    /// memory** and returns a CA ready to issue leaf certs, using the
    /// production leaf TTL and cache bound. The root private key lives
    /// only in this process's memory for the lifetime of the returned
    /// value — it is never written to disk, never serialized, and never
    /// handed to a caller.
    #[allow(dead_code)] // consumed by W6; not wired yet
    pub(crate) fn generate() -> Result<Self, RuntimeProcessError> {
        Self::generate_with(DEFAULT_LEAF_TTL, DEFAULT_MAX_CACHE_ENTRIES)
    }

    /// Same as [`Self::generate`] with an explicit leaf TTL and cache
    /// bound — the seam this module's own tests use to exercise TTL
    /// expiry and eviction without waiting on the production defaults.
    #[allow(dead_code)] // exercised by this module's own tests; W6 may call it directly later
    pub(crate) fn generate_with(
        leaf_ttl: Duration,
        max_cache_entries: usize,
    ) -> Result<Self, RuntimeProcessError> {
        let mut params = CertificateParams::new(Vec::new()).map_err(ca_error)?;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params
            .distinguished_name
            .push(DnType::CommonName, "IronClaw Sandbox Egress CA");
        params
            .distinguished_name
            .push(DnType::OrganizationName, "IronClaw");
        params.key_usages.push(KeyUsagePurpose::KeyCertSign);
        params.key_usages.push(KeyUsagePurpose::CrlSign);
        params.key_usages.push(KeyUsagePurpose::DigitalSignature);
        let now = OffsetDateTime::now_utc();
        params.not_before = now
            .checked_sub(NOT_BEFORE_SKEW)
            .ok_or_else(|| ca_range_error("root not_before"))?;
        params.not_after = now
            .checked_add(ROOT_VALIDITY)
            .ok_or_else(|| ca_range_error("root not_after"))?;

        let root_not_after = params.not_after;
        let root_key = KeyPair::generate().map_err(ca_error)?;
        // `self_signed` only borrows `params` — signing happens before
        // `params` moves into `Issuer::new` just below.
        let root_cert = params.self_signed(&root_key).map_err(ca_error)?;
        let root_cert_pem = root_cert.pem();
        let issuer = Issuer::new(params, root_key);

        Ok(Self {
            root_cert_pem,
            issuer,
            root_not_after,
            leaf_ttl,
            max_cache_entries,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// The CA's public trust anchor — the only artifact of this CA meant
    /// to reach a container (read-only bind-mount + `SSL_CERT_FILE` and
    /// friends is W5's remaining trust-distribution work). Contains no
    /// private key material; see this module's
    /// `root_certificate_pem_never_contains_key_material` test.
    #[allow(dead_code)] // consumed by W6; not wired yet
    pub(crate) fn root_certificate_pem(&self) -> &str {
        &self.root_cert_pem
    }

    /// Returns a leaf certificate for `host`: a live cached one if present,
    /// or a freshly minted (and cached) one otherwise. Every leaf's only
    /// SAN is the requested host, and the cache is keyed by the
    /// canonicalized (trimmed, lowercased) host string — issuing for one
    /// host can never hand back a cert valid for another, and case/padding
    /// variants of the same host share one cache entry.
    #[allow(dead_code)] // consumed by W6; not wired yet
    pub(crate) fn issue_leaf_for_host(
        &self,
        host: &str,
    ) -> Result<IssuedLeaf, RuntimeProcessError> {
        // Bound the raw, untrimmed length before any allocation: a
        // network-controlled CONNECT host that is merely oversized should
        // fail without paying for a lowercase copy of it first. This is the
        // same bound `validate_dns_host` enforces on the canonicalized
        // string below; checking it here too just moves the rejection ahead
        // of the allocation for the common "just too long" case.
        if host.len() > MAX_DNS_HOST_LEN {
            return Err(RuntimeProcessError::ExecutionFailed(format!(
                "sandbox CA: host exceeds the maximum DNS name length of {MAX_DNS_HOST_LEN}"
            )));
        }
        // Normalize once, at the boundary: DNS names are case-insensitive
        // and an intercepted CONNECT's SNI/host can carry incidental
        // whitespace. Trimming only the emptiness *check* while minting and
        // caching on the untrimmed, original-case string (the prior
        // behavior) would bake padding into the SAN/CN and let case
        // variants of the same host multiply cache entries — on a
        // bounded, oldest-first-eviction cache, that lets a peer choosing
        // SNI case variants evict unrelated live entries. Every downstream
        // use (cache key, SAN, CN) shares this one canonical form.
        let host = host.trim().to_ascii_lowercase();
        if host.is_empty() {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox CA: host must not be empty".to_string(),
            ));
        }
        // `rcgen` accepts arbitrary ASCII strings as a DNS SAN — it does not
        // itself reject control characters, wildcards, or oversized input.
        // A network-controlled CONNECT host reaches this call, so the CA
        // must validate it is a plausible DNS name before spending a key
        // generation and signing pass on it. (The length bound above is
        // re-checked here too, post-trim, since trimming can only shrink
        // the string — this second check is what actually enforces the
        // bound on the canonical form.)
        validate_dns_host(&host)?;
        let now = Instant::now();
        if let Some(certificate) = self.cached_leaf(&host, now) {
            return Ok(IssuedLeaf {
                certificate,
                cache_hit: true,
            });
        }

        let (leaf, expires_at) = self.mint_leaf(&host, now)?;
        self.insert_and_evict(host, leaf.clone(), now, expires_at);
        Ok(IssuedLeaf {
            certificate: leaf,
            cache_hit: false,
        })
    }

    /// Test/introspection seam: how many hosts currently hold a cached
    /// leaf, without exposing the cache's contents.
    #[cfg(test)]
    pub(crate) fn cached_entry_count(&self) -> usize {
        self.lock_cache().len()
    }

    /// Test/introspection seam: how much longer a just-cached host's entry
    /// has to live, so a test can pin that the cache's own TTL clock tracks
    /// the leaf's actual (possibly root-capped) expiry rather than the raw
    /// `leaf_ttl` alone.
    #[cfg(test)]
    pub(crate) fn cached_leaf_ttl_remaining(&self, host: &str) -> Option<Duration> {
        let cache = self.lock_cache();
        let entry = cache.get(host)?;
        Some(
            entry
                .expires_at
                .saturating_duration_since(entry.inserted_at),
        )
    }

    fn cached_leaf(&self, host: &str, now: Instant) -> Option<LeafCertificate> {
        let mut cache = self.lock_cache();
        let entry = cache.get(host)?;
        if now >= entry.expires_at {
            cache.remove(host);
            return None;
        }
        Some(entry.leaf.clone())
    }

    fn mint_leaf(
        &self,
        host: &str,
        now_instant: Instant,
    ) -> Result<(LeafCertificate, Instant), RuntimeProcessError> {
        let mut params = CertificateParams::new(vec![host.to_string()]).map_err(ca_error)?;
        params.distinguished_name.push(DnType::CommonName, host);
        params.use_authority_key_identifier_extension = true;
        params.key_usages.push(KeyUsagePurpose::DigitalSignature);
        params
            .extended_key_usages
            .push(ExtendedKeyUsagePurpose::ServerAuth);
        let now = OffsetDateTime::now_utc();
        params.not_before = now
            .checked_sub(NOT_BEFORE_SKEW)
            .ok_or_else(|| ca_range_error("leaf not_before"))?;
        let leaf_ttl = CertValidityDuration::try_from(self.leaf_ttl).map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox CA: leaf ttl out of range: {error}"
            ))
        })?;
        let requested_not_after = now
            .checked_add(leaf_ttl)
            .ok_or_else(|| ca_range_error("leaf not_after"))?;
        // A leaf must never outlive its own trust anchor: the root is
        // regenerated fresh per process start (see the module doc) and has
        // its own fixed `ROOT_VALIDITY`, so a leaf minted late in the root's
        // life would otherwise carry a `not_after` past the point at which
        // nothing can validate it against the root anymore.
        params.not_after = requested_not_after.min(self.root_not_after);
        if params.not_after <= params.not_before {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox CA: root certificate has expired; cannot issue a leaf".to_string(),
            ));
        }

        // The cache's TTL clock must track this leaf's *actual* not_after
        // (post root-cap), not the raw `leaf_ttl` the caller/default
        // requested — see `CachedLeaf::expires_at`'s doc. Converts the
        // wall-clock `not_after` into a monotonic instant relative to this
        // call's own `now_instant`, so `cached_leaf` can compare like against
        // like.
        let remaining = if params.not_after > now {
            params.not_after - now
        } else {
            CertValidityDuration::ZERO
        };
        let remaining_std = Duration::try_from(remaining).unwrap_or(Duration::ZERO);
        let expires_at = now_instant
            .checked_add(remaining_std)
            .unwrap_or(now_instant);

        let leaf_key = KeyPair::generate().map_err(ca_error)?;
        let cert = params
            .signed_by(&leaf_key, &self.issuer)
            .map_err(ca_error)?;

        Ok((
            LeafCertificate {
                host: host.to_string(),
                cert_pem: cert.pem(),
                key_pem: leaf_key.serialize_pem(),
            },
            expires_at,
        ))
    }

    fn insert_and_evict(
        &self,
        host: String,
        leaf: LeafCertificate,
        now: Instant,
        expires_at: Instant,
    ) {
        let mut cache = self.lock_cache();
        cache.insert(
            host,
            CachedLeaf {
                leaf,
                inserted_at: now,
                expires_at,
            },
        );
        // Bounded eviction: drop the oldest-inserted entry until back
        // within budget, one at a time (cache sizes here are small, so an
        // O(n) scan per eviction is cheap and needs no extra ordering
        // structure to keep in sync with the map).
        while cache.len() > self.max_cache_entries {
            let oldest = cache
                .iter()
                .min_by_key(|(_, cached)| cached.inserted_at)
                .map(|(host, _)| host.clone());
            match oldest {
                Some(host) => {
                    cache.remove(&host);
                }
                None => break,
            }
        }
    }

    fn lock_cache(&self) -> MutexGuard<'_, HashMap<String, CachedLeaf>> {
        self.cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

fn ca_error(error: rcgen::Error) -> RuntimeProcessError {
    RuntimeProcessError::ExecutionFailed(format!("sandbox CA: {error}"))
}

fn ca_range_error(field: &str) -> RuntimeProcessError {
    RuntimeProcessError::ExecutionFailed(format!(
        "sandbox CA: {field} computation overflowed the valid date range"
    ))
}

/// Bound on total DNS name length (RFC 1035 §3.1): 253 visible characters
/// (255 octets on the wire, minus the length-prefix and root-label bytes).
const MAX_DNS_HOST_LEN: usize = 253;

/// Bound on one DNS label's length (RFC 1035 §3.1).
const MAX_DNS_LABEL_LEN: usize = 63;

/// Rejects hosts `rcgen` itself would accept as a DNS SAN but that are not
/// plausible DNS names: oversized input, wildcards, and anything outside
/// `[a-z0-9-.]` (which excludes control characters and other non-ASCII or
/// non-hostname bytes). `host` is expected to already be trimmed and
/// lowercased by the caller. This is deliberately a plausibility filter, not
/// full RFC 1035 label-syntax enforcement (e.g. leading/trailing hyphens
/// within a label) — the goal is bounding what a network-controlled CONNECT
/// host can force this CA to mint a certificate for, not general-purpose DNS
/// validation.
fn validate_dns_host(host: &str) -> Result<(), RuntimeProcessError> {
    if host.len() > MAX_DNS_HOST_LEN {
        return Err(RuntimeProcessError::ExecutionFailed(format!(
            "sandbox CA: host exceeds the maximum DNS name length of {MAX_DNS_HOST_LEN}"
        )));
    }
    if host.contains('*') {
        return Err(RuntimeProcessError::ExecutionFailed(
            "sandbox CA: wildcard hosts are not permitted".to_string(),
        ));
    }
    let is_valid_char = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.';
    if !host.bytes().all(is_valid_char) {
        return Err(RuntimeProcessError::ExecutionFailed(
            "sandbox CA: host contains characters outside the DNS hostname charset".to_string(),
        ));
    }
    for label in host.split('.') {
        if label.is_empty() || label.len() > MAX_DNS_LABEL_LEN {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox CA: host contains an empty or oversized DNS label".to_string(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
