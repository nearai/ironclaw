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
    let ca = SandboxCertificateAuthority::generate_with(ttl, DEFAULT_MAX_CACHE_ENTRIES).unwrap();
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
fn zero_cache_capacity_issues_without_retaining_entries() {
    let ca = SandboxCertificateAuthority::generate_with(Duration::from_secs(300), 0).unwrap();

    let first = ca.issue_leaf_for_host("api.example.com").unwrap();
    assert!(!first.cache_hit);
    // A zero-capacity cache must never retain what it just inserted —
    // `insert_and_evict`'s `while cache.len() > max_cache_entries` loop
    // must evict back down to empty immediately.
    assert_eq!(ca.cached_entry_count(), 0);

    let second = ca.issue_leaf_for_host("api.example.com").unwrap();
    assert!(
        !second.cache_hit,
        "a zero-capacity cache must never report a cache hit"
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
fn consecutive_dots_are_rejected_as_an_empty_label() {
    let ca = SandboxCertificateAuthority::generate().unwrap();
    // Well under `MAX_DNS_HOST_LEN`, so this exercises `validate_dns_host`'s
    // per-label `is_empty` branch specifically, not the total-length check
    // `oversized_host_is_rejected` already covers.
    let error = ca.issue_leaf_for_host("a..b.example.com");

    assert!(
        error.is_err(),
        "a host with an empty label (consecutive dots) must fail issuance"
    );
    assert_eq!(ca.cached_entry_count(), 0);
}

#[test]
fn oversized_dns_label_is_rejected() {
    let ca = SandboxCertificateAuthority::generate().unwrap();
    // A single 64-byte label exceeds `MAX_DNS_LABEL_LEN` (63) while the
    // total host stays well under `MAX_DNS_HOST_LEN` (253) — this
    // exercises `validate_dns_host`'s per-label length branch, which the
    // total-length test never reaches.
    let oversized_label_host = format!("{}.example.com", "a".repeat(64));

    let error = ca.issue_leaf_for_host(&oversized_label_host);

    assert!(
        error.is_err(),
        "a host with a single label over 63 bytes must fail issuance"
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
fn leaf_not_after_is_capped_at_the_root_expiry() {
    // 40 days comfortably exceeds `ROOT_VALIDITY` (30 days) while
    // staying well inside the `CertValidityDuration` range that
    // `oversized_leaf_ttl_fails_issuance_without_caching` proves fails —
    // this exercises `mint_leaf`'s `min(requested_not_after,
    // root_not_after)` cap actually taking effect, not just existing.
    let long_ttl = Duration::from_secs(40 * 24 * 60 * 60);
    let ca =
        SandboxCertificateAuthority::generate_with(long_ttl, DEFAULT_MAX_CACHE_ENTRIES).unwrap();
    let issued = ca.issue_leaf_for_host("api.example.com").unwrap();

    let root = parse(ca.root_certificate_pem());
    let leaf = parse(&issued.certificate.cert_pem);

    assert_eq!(
        leaf.validity().not_after.timestamp(),
        root.validity().not_after.timestamp(),
        "a leaf TTL longer than the root's own validity must be capped at the root's not_after"
    );
}

#[test]
fn cache_ttl_tracks_the_root_capped_expiry_not_the_requested_leaf_ttl() {
    // Same over-long TTL as `leaf_not_after_is_capped_at_the_root_expiry`:
    // regression for the cache returning an already-expired leaf when
    // `leaf_ttl` is much longer than the root-capped certificate
    // lifetime actually minted (see `CachedLeaf::expires_at`'s doc).
    let long_ttl = Duration::from_secs(40 * 24 * 60 * 60);
    let ca =
        SandboxCertificateAuthority::generate_with(long_ttl, DEFAULT_MAX_CACHE_ENTRIES).unwrap();
    let issued = ca.issue_leaf_for_host("api.example.com").unwrap();

    let leaf = parse(&issued.certificate.cert_pem);
    let cert_remaining_secs =
        leaf.validity().not_after.timestamp() - OffsetDateTime::now_utc().unix_timestamp();

    let cache_remaining = ca
        .cached_leaf_ttl_remaining("api.example.com")
        .expect("just-issued host must have a live cache entry");

    assert!(
        cache_remaining.as_secs() < long_ttl.as_secs(),
        "cache TTL must be capped well below the requested leaf_ttl once the root cap applies: {cache_remaining:?}"
    );
    assert!(
        (cache_remaining.as_secs() as i64 - cert_remaining_secs).abs() <= 2,
        "cache TTL must track the certificate's actual (root-capped) not_after, not the raw leaf_ttl: cache_remaining={cache_remaining:?} cert_remaining_secs={cert_remaining_secs}"
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
fn ca_debug_output_never_contains_private_key_material() {
    let ca = SandboxCertificateAuthority::generate().unwrap();
    // Issue a leaf too, so the CA's cache is non-empty when formatted —
    // a Debug impl that iterated `cache`'s contents (rather than only
    // reporting `max_cache_entries`) would leak leaf key material here.
    let issued = ca.issue_leaf_for_host("api.example.com").unwrap();

    let debug_ca = format!("{ca:?}");

    assert!(!debug_ca.contains("PRIVATE KEY"));
    assert!(!debug_ca.contains(&issued.certificate.key_pem));
    assert!(!debug_ca.contains(ca.root_certificate_pem()));
}

#[test]
fn issued_certificate_and_private_key_are_a_matching_pair() {
    let ca = SandboxCertificateAuthority::generate().unwrap();
    let issued = ca.issue_leaf_for_host("api.example.com").unwrap();

    let leaf = parse(&issued.certificate.cert_pem);
    let cert_spki = leaf.public_key().subject_public_key.data.to_vec();

    let key_pair = KeyPair::from_pem(&issued.certificate.key_pem)
        .expect("returned leaf key must parse as a valid key pair");

    assert_eq!(
        key_pair.public_key_raw(),
        cert_spki.as_slice(),
        "the returned private key's public component must match the returned certificate's subject public key"
    );
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
