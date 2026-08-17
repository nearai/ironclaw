use super::*;

#[test]
fn session_bytes_are_bounded_at_both_ends() {
    assert!(matches!(
        SessionBytes::new(Vec::new()),
        Err(LinkedSessionError::EmptyBlob)
    ));
    assert!(matches!(
        SessionBytes::new(vec![0u8; MAX_LINKED_SESSION_BYTES + 1]),
        Err(LinkedSessionError::BlobTooLarge {
            bytes,
            max: MAX_LINKED_SESSION_BYTES,
        }) if bytes == MAX_LINKED_SESSION_BYTES + 1
    ));

    let blob = SessionBytes::new(vec![7u8; 32]).expect("in-bounds blob");
    assert_eq!(blob.len(), 32);
    assert!(!blob.is_empty());
    assert_eq!(blob.expose(), &[7u8; 32]);
}

/// The whole point of the wrapper: `Debug` reports a length, never a byte.
#[test]
fn session_bytes_debug_never_renders_the_credential() {
    let blob = SessionBytes::new(b"super-secret-auth-key".to_vec()).expect("blob");
    let rendered = format!("{blob:?}");
    assert_eq!(rendered, "SessionBytes(21 bytes, redacted)");
    assert!(!rendered.contains("super-secret"), "{rendered}");

    // And through the snapshot that carries it, which derives `Debug`.
    let snapshot = LinkedSessionSnapshot {
        blob,
        version: LinkedSessionVersion::absent(),
    };
    let rendered = format!("{snapshot:?}");
    assert!(!rendered.contains("super-secret"), "{rendered}");
    assert!(rendered.contains("21 bytes, redacted"), "{rendered}");
}

/// Two halves, because neither alone is proof: the type carries the
/// `ZeroizeOnDrop` marker (so the bound is checked by the compiler, the
/// same way `ironclaw_host_api::http` pins its egress request), and the
/// clearing `Drop` performs really does overwrite the buffer rather than
/// just dropping the handle.
#[test]
fn session_bytes_zeroize_on_drop() {
    fn require_zeroize_on_drop<T: ?Sized + ZeroizeOnDrop>(_: &T) {}

    let mut blob = SessionBytes::new(b"auth-key-material".to_vec()).expect("blob");
    require_zeroize_on_drop(&blob);

    let before = blob.0.clone();
    assert!(
        before.iter().any(|byte| *byte != 0),
        "the fixture must start non-zero or this proves nothing"
    );
    blob.0.zeroize();
    assert!(
        blob.0.is_empty(),
        "`Zeroize for Vec<u8>` overwrites every byte and then truncates"
    );
}

#[test]
fn version_tokens_are_bounded_and_absent_is_distinct() {
    let absent = LinkedSessionVersion::absent();
    assert!(absent.is_absent());
    assert_eq!(absent.as_str(), None);

    let token = LinkedSessionVersion::new("v-17").expect("valid token");
    assert!(!token.is_absent());
    assert_eq!(token.as_str(), Some("v-17"));
    assert_ne!(token, absent);

    assert!(LinkedSessionVersion::new("").is_err());
    assert!(LinkedSessionVersion::new("a\nb").is_err());
    assert!(LinkedSessionVersion::new("x".repeat(MAX_LINKED_SESSION_VERSION_BYTES + 1)).is_err());
}

#[test]
fn linked_account_refs_are_bounded_opaque_strings() {
    let account = LinkedAccountRef::new("acct-9f3c").expect("valid ref");
    assert_eq!(account.as_str(), "acct-9f3c");
    assert_eq!(account.to_string(), "acct-9f3c");

    assert!(LinkedAccountRef::new("").is_err());
    assert!(LinkedAccountRef::new("has space").is_err());
    assert!(LinkedAccountRef::new("has\ttab").is_err());
    assert!(LinkedAccountRef::new("x".repeat(MAX_LINKED_ACCOUNT_REF_BYTES + 1)).is_err());
}

/// The revision is part of the grant's identity: same account, different
/// revision is a different grant, which is what evicts a stale handle.
#[test]
fn grants_distinguish_link_revisions_for_the_same_account() {
    let account = LinkedAccountRef::new("acct-9f3c").expect("valid ref");
    let first = LinkedAccountGrant::new(account.clone(), 1);
    let second = LinkedAccountGrant::new(account.clone(), 2);

    assert_eq!(first.account(), &account);
    assert_eq!(first.link_revision(), 1);
    assert_ne!(first, second);
    assert_eq!(first, LinkedAccountGrant::new(account, 1));
}

/// A conflict hands back the current version so the caller reloads and
/// merges rather than retrying blind.
#[test]
fn custody_errors_carry_recovery_context_but_no_bytes() {
    let current = LinkedSessionVersion::new("v-18").expect("token");
    let error = LinkedSessionError::VersionConflict {
        current: current.clone(),
    };
    assert!(matches!(
        &error,
        LinkedSessionError::VersionConflict { current: seen } if seen == &current
    ));
    assert!(error.to_string().contains("compare-and-swap"));

    let unavailable = LinkedSessionError::Unavailable {
        reason: "custody backend is offline",
    };
    assert_eq!(
        unavailable.to_string(),
        "linked-session custody is unavailable: custody backend is offline"
    );
}
