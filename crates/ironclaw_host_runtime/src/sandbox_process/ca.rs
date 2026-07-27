//! Internal per-tenant CA for the sandbox egress proxy (W5 — design doc
//! `docs/plans/2026-07-26-sandbox-credential-firewall-design.md` §4).
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

use crate::RuntimeProcessError;

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

        let root_key = KeyPair::generate().map_err(ca_error)?;
        // `self_signed` only borrows `params` — signing happens before
        // `params` moves into `Issuer::new` just below.
        let root_cert = params.self_signed(&root_key).map_err(ca_error)?;
        let root_cert_pem = root_cert.pem();
        let issuer = Issuer::new(params, root_key);

        Ok(Self {
            root_cert_pem,
            issuer,
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
    /// SAN is the requested host, and the cache is keyed by exact host
    /// string — issuing for one host can never hand back a cert valid for
    /// another.
    #[allow(dead_code)] // consumed by W6; not wired yet
    pub(crate) fn issue_leaf_for_host(
        &self,
        host: &str,
    ) -> Result<IssuedLeaf, RuntimeProcessError> {
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
        // generation and signing pass on it.
        validate_dns_host(&host)?;
        let now = Instant::now();
        if let Some(certificate) = self.cached_leaf(&host, now) {
            return Ok(IssuedLeaf {
                certificate,
                cache_hit: true,
            });
        }

        let leaf = self.mint_leaf(&host)?;
        self.insert_and_evict(host, leaf.clone(), now);
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

    fn cached_leaf(&self, host: &str, now: Instant) -> Option<LeafCertificate> {
        let mut cache = self.lock_cache();
        let entry = cache.get(host)?;
        if now.saturating_duration_since(entry.inserted_at) >= self.leaf_ttl {
            cache.remove(host);
            return None;
        }
        Some(entry.leaf.clone())
    }

    fn mint_leaf(&self, host: &str) -> Result<LeafCertificate, RuntimeProcessError> {
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
        params.not_after = now
            .checked_add(leaf_ttl)
            .ok_or_else(|| ca_range_error("leaf not_after"))?;

        let leaf_key = KeyPair::generate().map_err(ca_error)?;
        let cert = params
            .signed_by(&leaf_key, &self.issuer)
            .map_err(ca_error)?;

        Ok(LeafCertificate {
            host: host.to_string(),
            cert_pem: cert.pem(),
            key_pem: leaf_key.serialize_pem(),
        })
    }

    fn insert_and_evict(&self, host: String, leaf: LeafCertificate, now: Instant) {
        let mut cache = self.lock_cache();
        cache.insert(
            host,
            CachedLeaf {
                leaf,
                inserted_at: now,
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
mod tests {
    use super::*;
    use std::sync::Arc;
    use x509_parser::prelude::*;

    fn parse<'a>(pem: &'a str) -> X509Certificate<'a> {
        let (_, parsed) = parse_x509_pem(pem.as_bytes()).expect("valid PEM");
        let cert = Box::leak(Box::new(parsed));
        cert.parse_x509().expect("valid X.509 DER")
    }

    fn dns_sans(cert: &X509Certificate<'_>) -> Vec<String> {
        cert.subject_alternative_name()
            .expect("SAN extension parses")
            .expect("leaf has a SAN extension")
            .value
            .general_names
            .iter()
            .filter_map(|name| match name {
                GeneralName::DNSName(dns) => Some((*dns).to_string()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn root_is_generated_once_and_reused() {
        let ca = SandboxCertificateAuthority::generate().unwrap();

        // Same in-memory root every call — not regenerated per access.
        assert_eq!(ca.root_certificate_pem(), ca.root_certificate_pem());
    }

    #[test]
    fn leaf_chains_to_root_and_validates() {
        let ca = SandboxCertificateAuthority::generate().unwrap();
        let issued = ca.issue_leaf_for_host("api.example.com").unwrap();

        let root = parse(ca.root_certificate_pem());
        let leaf = parse(&issued.certificate.cert_pem);

        leaf.verify_signature(Some(root.public_key()))
            .expect("leaf must validate against the root's public key");
    }

    #[test]
    fn leaf_does_not_validate_against_an_unrelated_root() {
        let ca = SandboxCertificateAuthority::generate().unwrap();
        let other_ca = SandboxCertificateAuthority::generate().unwrap();
        let issued = ca.issue_leaf_for_host("api.example.com").unwrap();

        let other_root = parse(other_ca.root_certificate_pem());
        let leaf = parse(&issued.certificate.cert_pem);

        assert!(
            leaf.verify_signature(Some(other_root.public_key()))
                .is_err(),
            "a leaf signed by one CA must not validate against a different CA's root"
        );
    }

    #[test]
    fn leaf_cn_and_san_match_requested_host() {
        let ca = SandboxCertificateAuthority::generate().unwrap();
        let issued = ca.issue_leaf_for_host("api.example.com").unwrap();

        let leaf = parse(&issued.certificate.cert_pem);

        assert_eq!(dns_sans(&leaf), vec!["api.example.com".to_string()]);
        assert_eq!(issued.certificate.host, "api.example.com");
    }

    #[test]
    fn leaf_ttl_is_short_and_expiry_is_honored() {
        // X.509 timestamps have one-second granularity, so the TTL and
        // sleep below are sized to cross a full second boundary with
        // margin rather than a few tens of milliseconds.
        let ttl = Duration::from_millis(1_200);
        let ca =
            SandboxCertificateAuthority::generate_with(ttl, DEFAULT_MAX_CACHE_ENTRIES).unwrap();
        let issued = ca.issue_leaf_for_host("api.example.com").unwrap();
        let leaf = parse(&issued.certificate.cert_pem);

        let now = OffsetDateTime::now_utc().unix_timestamp();
        let not_after = leaf.validity().not_after.timestamp();
        // Short-lived: expires within a small bound of "now", not the
        // multi-decade default `CertificateParams` would otherwise carry.
        assert!(
            not_after - now <= 5,
            "leaf TTL must be short: not_after={not_after} now={now}"
        );

        std::thread::sleep(Duration::from_millis(2_500));

        // The cache TTL is honored: after it elapses, the same host gets a
        // freshly minted leaf, not the stale cached one.
        let reissued = ca.issue_leaf_for_host("api.example.com").unwrap();
        assert!(
            !reissued.cache_hit,
            "expired cache entry must not be reused"
        );

        // And the cert's own validity window has genuinely lapsed by now.
        let past_now = OffsetDateTime::now_utc().unix_timestamp();
        assert!(
            past_now > not_after,
            "the first leaf's not_after must actually be in the past by now"
        );
    }

    #[test]
    fn cache_hit_avoids_reissuing_within_ttl() {
        let ca = SandboxCertificateAuthority::generate_with(
            Duration::from_secs(300),
            DEFAULT_MAX_CACHE_ENTRIES,
        )
        .unwrap();

        let first = ca.issue_leaf_for_host("api.example.com").unwrap();
        assert!(!first.cache_hit);

        let second = ca.issue_leaf_for_host("api.example.com").unwrap();
        assert!(second.cache_hit);
        assert_eq!(
            first.certificate.cert_pem, second.certificate.cert_pem,
            "a cache hit must return the same cert, not mint a new one"
        );
    }

    #[test]
    fn cache_evicts_the_oldest_entry_once_bounded() {
        let ca = SandboxCertificateAuthority::generate_with(Duration::from_secs(300), 2).unwrap();

        ca.issue_leaf_for_host("a.example.com").unwrap();
        std::thread::sleep(Duration::from_millis(10));
        ca.issue_leaf_for_host("b.example.com").unwrap();
        std::thread::sleep(Duration::from_millis(10));
        // Cache bound is 2; adding a third host must evict the oldest ("a").
        ca.issue_leaf_for_host("c.example.com").unwrap();

        assert_eq!(ca.cached_entry_count(), 2);

        // "a" was evicted: reissuing for it mints fresh, not a cache hit.
        let a_again = ca.issue_leaf_for_host("a.example.com").unwrap();
        assert!(!a_again.cache_hit, "evicted host must be freshly minted");

        // "c" (the most recently inserted before the check) is still live.
        let c_again = ca.issue_leaf_for_host("c.example.com").unwrap();
        assert!(
            c_again.cache_hit,
            "recently cached host must still be cached"
        );
    }

    #[test]
    fn issuing_for_host_a_never_returns_a_cert_valid_for_host_b() {
        let ca = SandboxCertificateAuthority::generate().unwrap();

        let a = ca.issue_leaf_for_host("a.example.com").unwrap();
        let b = ca.issue_leaf_for_host("b.example.com").unwrap();

        let leaf_a = parse(&a.certificate.cert_pem);
        let leaf_b = parse(&b.certificate.cert_pem);

        assert_eq!(dns_sans(&leaf_a), vec!["a.example.com".to_string()]);
        assert_eq!(dns_sans(&leaf_b), vec!["b.example.com".to_string()]);
        assert_ne!(a.certificate.cert_pem, b.certificate.cert_pem);
        assert_ne!(a.certificate.key_pem, b.certificate.key_pem);

        // Requesting "a" again after "b" was minted must still return "a"'s
        // own cert, never cross over to "b"'s.
        let a_again = ca.issue_leaf_for_host("a.example.com").unwrap();
        assert_eq!(a_again.certificate.cert_pem, a.certificate.cert_pem);
    }

    #[test]
    fn empty_host_is_rejected() {
        let ca = SandboxCertificateAuthority::generate().unwrap();

        let error = ca.issue_leaf_for_host("").unwrap_err();

        assert!(format!("{error}").contains("host must not be empty"));
    }

    #[test]
    fn whitespace_only_host_is_rejected() {
        let ca = SandboxCertificateAuthority::generate().unwrap();

        let error = ca.issue_leaf_for_host("   ").unwrap_err();

        assert!(format!("{error}").contains("host must not be empty"));
    }

    #[test]
    fn host_lookup_is_case_insensitive_and_ignores_padding() {
        let ca = SandboxCertificateAuthority::generate().unwrap();

        let first = ca.issue_leaf_for_host("api.example.com").unwrap();
        assert!(!first.cache_hit);

        // A case variant and a padded variant of the same host must both
        // hit the cache entry the canonical form created, not mint (and
        // cache) their own separate entries.
        let case_variant = ca.issue_leaf_for_host("API.Example.COM").unwrap();
        assert!(
            case_variant.cache_hit,
            "case variants of an already-cached host must be a cache hit"
        );

        let padded_variant = ca.issue_leaf_for_host("  api.example.com  ").unwrap();
        assert!(
            padded_variant.cache_hit,
            "padded variants of an already-cached host must be a cache hit"
        );

        assert_eq!(ca.cached_entry_count(), 1);
    }

    #[test]
    fn padded_host_input_produces_an_unpadded_san() {
        let ca = SandboxCertificateAuthority::generate().unwrap();
        let issued = ca.issue_leaf_for_host("  api.example.com  ").unwrap();

        let leaf = parse(&issued.certificate.cert_pem);

        assert_eq!(dns_sans(&leaf), vec!["api.example.com".to_string()]);
        assert_eq!(issued.certificate.host, "api.example.com");
    }

    #[test]
    fn malformed_host_fails_issuance_without_caching() {
        let ca = SandboxCertificateAuthority::generate().unwrap();

        // `rcgen` itself does not reject a control character in a DNS SAN
        // (confirmed: without `validate_dns_host` this mints a certificate
        // for it), so the CA's own boundary validation must reject it.
        let error = ca.issue_leaf_for_host("bad\u{0}host.example.com");

        assert!(
            error.is_err(),
            "a control-character host must fail issuance"
        );
        assert_eq!(
            ca.cached_entry_count(),
            0,
            "a failed issuance must not leave a cache entry behind"
        );
    }

    #[test]
    fn wildcard_host_is_rejected() {
        let ca = SandboxCertificateAuthority::generate().unwrap();

        let error = ca.issue_leaf_for_host("*.example.com").unwrap_err();

        assert!(format!("{error}").contains("wildcard"));
        assert_eq!(ca.cached_entry_count(), 0);
    }

    #[test]
    fn oversized_host_is_rejected() {
        let ca = SandboxCertificateAuthority::generate().unwrap();
        let oversized_host = format!("{}.example.com", "a".repeat(300));

        let error = ca.issue_leaf_for_host(&oversized_host);

        assert!(
            error.is_err(),
            "a host over the DNS length bound must fail issuance"
        );
        assert_eq!(ca.cached_entry_count(), 0);
    }

    #[test]
    fn oversized_leaf_ttl_fails_issuance_without_caching() {
        // `time::Duration` (used for cert validity) is bounded well below
        // `u64::MAX` seconds; a `std::time::Duration` this large must fail
        // the `CertValidityDuration::try_from` conversion in `mint_leaf`
        // rather than panicking or wrapping.
        let ca = SandboxCertificateAuthority::generate_with(
            Duration::from_secs(u64::MAX),
            DEFAULT_MAX_CACHE_ENTRIES,
        )
        .unwrap();

        let error = ca.issue_leaf_for_host("api.example.com");

        assert!(
            error.is_err(),
            "an out-of-range leaf TTL must fail issuance"
        );
        assert_eq!(
            ca.cached_entry_count(),
            0,
            "a failed issuance must not leave a cache entry behind"
        );
    }

    #[test]
    fn root_certificate_has_ca_and_key_cert_sign_constraints() {
        let ca = SandboxCertificateAuthority::generate().unwrap();
        let root = parse(ca.root_certificate_pem());

        let basic_constraints = root
            .basic_constraints()
            .expect("basic constraints extension parses")
            .expect("root has a basic constraints extension");
        assert!(basic_constraints.value.ca, "root must be a CA certificate");

        let key_usage = root
            .key_usage()
            .expect("key usage extension parses")
            .expect("root has a key usage extension");
        assert!(
            key_usage.value.key_cert_sign(),
            "root must be allowed to sign certificates"
        );
    }

    #[test]
    fn leaf_certificate_is_not_a_ca_and_has_server_auth_usage() {
        let ca = SandboxCertificateAuthority::generate().unwrap();
        let issued = ca.issue_leaf_for_host("api.example.com").unwrap();
        let leaf = parse(&issued.certificate.cert_pem);

        // A leaf must never itself be usable as an intermediate/CA cert.
        if let Some(basic_constraints) = leaf
            .basic_constraints()
            .expect("basic constraints extension parses")
        {
            assert!(
                !basic_constraints.value.ca,
                "a leaf certificate must not carry CA:true"
            );
        }

        let extended_key_usage = leaf
            .extended_key_usage()
            .expect("extended key usage extension parses")
            .expect("leaf has an extended key usage extension");
        assert!(
            extended_key_usage.value.server_auth,
            "leaf must be usable for TLS server authentication"
        );
    }

    #[test]
    fn root_certificate_pem_never_contains_key_material() {
        let ca = SandboxCertificateAuthority::generate().unwrap();
        let root_pem = ca.root_certificate_pem();

        // The only artifact meant to reach a container is a bare
        // certificate (public trust anchor) — it must parse as a valid
        // X.509 cert and must never carry a PEM-encoded key block.
        assert!(root_pem.starts_with("-----BEGIN CERTIFICATE-----"));
        assert!(!root_pem.contains("PRIVATE KEY"));
        assert!(!root_pem.contains("BEGIN EC PRIVATE KEY"));
        let _ = parse(root_pem); // panics (test-only) if the PEM doesn't parse as a cert
    }

    #[test]
    fn leaf_key_material_is_scoped_to_the_leaf_not_the_root() {
        let ca = SandboxCertificateAuthority::generate().unwrap();
        let issued = ca.issue_leaf_for_host("api.example.com").unwrap();

        // The leaf's own key is real key material (expected — the leaf key
        // is a bounded, short-lived, host-scoped artifact this design
        // accepts inside the container). It must not equal or embed the
        // root's certificate PEM, and the root's public artifact must not
        // contain the leaf's key.
        assert!(issued.certificate.key_pem.contains("PRIVATE KEY"));
        assert!(
            !ca.root_certificate_pem()
                .contains(&issued.certificate.key_pem)
        );
        assert_ne!(issued.certificate.key_pem, ca.root_certificate_pem());
    }

    #[test]
    fn leaf_certificate_debug_output_never_contains_private_key_material() {
        let ca = SandboxCertificateAuthority::generate().unwrap();
        let issued = ca.issue_leaf_for_host("api.example.com").unwrap();

        // Regression: `LeafCertificate`/`IssuedLeaf`'s `Debug` must redact
        // `key_pem` the same way the CA's own `Debug` redacts `issuer`/
        // `cache` — formatting either type must never persist a leaf
        // private key in logs or panic output.
        let debug_leaf = format!("{:?}", issued.certificate);
        let debug_issued = format!("{issued:?}");

        assert!(!debug_leaf.contains("PRIVATE KEY"));
        assert!(!debug_leaf.contains(&issued.certificate.key_pem));
        assert!(!debug_issued.contains("PRIVATE KEY"));
        assert!(!debug_issued.contains(&issued.certificate.key_pem));
    }

    #[test]
    fn concurrent_issuance_for_same_and_different_hosts_stays_correct() {
        let ca = Arc::new(SandboxCertificateAuthority::generate().unwrap());
        let mut handles = Vec::new();

        // 4 threads race on the same host, 4 threads each mint a distinct
        // host — a real contention exercise of the cache's Mutex, not the
        // sequential single-thread pattern the rest of this module uses.
        for _ in 0..4 {
            let ca = Arc::clone(&ca);
            handles.push(std::thread::spawn(move || {
                ca.issue_leaf_for_host("shared.example.com").unwrap()
            }));
        }
        for i in 0..4 {
            let ca = Arc::clone(&ca);
            handles.push(std::thread::spawn(move || {
                ca.issue_leaf_for_host(&format!("distinct-{i}.example.com"))
                    .unwrap()
            }));
        }

        let results: Vec<IssuedLeaf> = handles
            .into_iter()
            .map(|handle| handle.join().expect("issuance thread must not panic"))
            .collect();

        // Every distinct-host result must carry that host's own SAN — no
        // cross-host mixing under concurrent cache writes.
        for (i, result) in results.iter().skip(4).enumerate() {
            assert_eq!(result.certificate.host, format!("distinct-{i}.example.com"));
        }

        // All four racers on the shared host must agree on the *content*
        // that ultimately lives in the cache: whichever thread's mint won,
        // a subsequent lookup for that host must return exactly that cert.
        let settled = ca.issue_leaf_for_host("shared.example.com").unwrap();
        assert!(settled.cache_hit);
        assert!(
            results[..4]
                .iter()
                .any(|result| result.certificate.cert_pem == settled.certificate.cert_pem),
            "the cached cert must match one of the racing issuances"
        );
    }
}
