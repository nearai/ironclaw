use std::{sync::Arc, time::Duration};

use ironclaw_filesystem::{InMemoryBackend, RecordVersion, ScopedFilesystem};
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

use super::*;

#[test]
fn cached_snapshot_freshness_is_bounded() {
    let snapshot = TurnPersistenceSnapshot::default();
    let fresh = CachedSnapshot::new(snapshot.clone(), None);
    assert!(fresh.is_fresh());

    let stale = CachedSnapshot {
        snapshot,
        version: None,
        loaded_at: Instant::now() - SNAPSHOT_READ_CACHE_TTL - Duration::from_millis(1),
    };
    assert!(!stale.is_fresh());
}

#[tokio::test]
async fn no_op_apply_populates_cache_with_default_snapshot() {
    // When the record is absent and the apply closure produces a default
    // snapshot (nothing to persist), `apply` uses `CasApply::no_op` so
    // `cas_update` skips the write. The outer match treats the no-op result
    // as a normal Ok and stores the default snapshot in the cache. A cached
    // default is observationally identical to re-reading an absent record.
    let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/turns").unwrap(),
            VirtualPath::new("/engine/turns").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap(),
    ));
    let store = FilesystemTurnStateStore::new(filesystem);
    // Pre-seed with version 99 so we can confirm it gets replaced, not kept.
    store.store_snapshot_cache((
        TurnPersistenceSnapshot::default(),
        Some(RecordVersion::from_backend(99)),
    ));

    store
        .apply(RunnerLeaseOverlay::None, |store| async move {
            (Ok::<_, TurnError>(()), store)
        })
        .await
        .unwrap();

    // Cache must be populated with the default snapshot at version None
    // (the stale version-99 entry was replaced).
    let cached = store
        .fresh_cached_snapshot()
        .expect("no-op apply must populate cache with default snapshot");
    assert_eq!(
        cached.0,
        TurnPersistenceSnapshot::default(),
        "cached snapshot must be the default"
    );
    assert_eq!(cached.1, None, "no-op write has no record version");
}
