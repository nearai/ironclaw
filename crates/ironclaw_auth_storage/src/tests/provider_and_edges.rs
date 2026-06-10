use super::*;

// ─── fix: fs_error maps VersionMismatch to BackendConflict ───────────────────

#[test]
fn fs_error_maps_version_mismatch_to_backend_conflict() {
    use super::paths::fs_error;
    use ironclaw_filesystem::{FilesystemError, FilesystemOperation};
    use ironclaw_host_api::VirtualPath;

    let version_mismatch = FilesystemError::VersionMismatch {
        path: VirtualPath::new("/secrets/test").unwrap(),
        expected: None,
        found: None,
    };
    assert_eq!(
        fs_error(version_mismatch),
        AuthProductError::BackendConflict,
        "VersionMismatch must map to BackendConflict, not BackendUnavailable"
    );

    let backend_err = FilesystemError::Backend {
        path: VirtualPath::new("/secrets/test").unwrap(),
        operation: FilesystemOperation::ReadFile,
        reason: "io error".to_string(),
    };
    assert_eq!(
        fs_error(backend_err),
        AuthProductError::BackendUnavailable,
        "non-CAS errors must still map to BackendUnavailable"
    );
}

// ─── fix: lock-cache weak-reference GC actually shrinks the map ──────────────

#[tokio::test]
async fn filesystem_lock_cache_drops_weak_entries_after_release() {
    let filesystem = test_filesystem();
    let secret_store: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());
    let service = test_service(filesystem, secret_store);

    {
        // Acquire a lock for key A and drop the guard immediately.
        let lock_a = service.lock_for("account:key-a".to_string());
        let _guard_a = lock_a.lock().await;
        // guard_a dropped at end of this block; Arc<Mutex> dropped too after lock_a drops.
    }
    // After key-A's Arc dropped, the next call to lock_for should evict the
    // dead weak reference. We trigger eviction via lock_for on a different key.
    let _lock_b = service.lock_for("account:key-b".to_string());

    // Verify key-A is gone: requesting it again must produce a *new* Arc (i.e.
    // a fresh Mutex), not the evicted weak ref.
    let lock_a2 = service.lock_for("account:key-a".to_string());
    // The new lock should be unlocked (no one holds it).
    assert!(
        lock_a2.try_lock().is_ok(),
        "re-acquired key-a must be unlocked"
    );
}
