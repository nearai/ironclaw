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
async fn no_op_apply_clears_snapshot_cache_before_returning() {
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

    assert!(store.fresh_cached_snapshot().is_none());
}
