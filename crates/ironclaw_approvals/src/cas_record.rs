use std::{
    collections::HashMap,
    hash::Hash,
    sync::{Arc, Mutex, RwLock},
};

use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, RootFilesystem, ScopedFilesystem,
    VersionedEntry,
};
use ironclaw_host_api::{ResourceScope, ScopedPath};

pub(crate) struct FilesystemCasRecordStore<F, K>
where
    F: RootFilesystem,
    K: Clone + Eq + Hash,
{
    pub(crate) filesystem: Arc<ScopedFilesystem<F>>,
    pub(crate) path_cache: RwLock<HashMap<K, ScopedPath>>,
    pub(crate) mutation_locks: Mutex<HashMap<K, Arc<tokio::sync::Mutex<()>>>>,
    cache_max_entries: usize,
}

impl<F, K> FilesystemCasRecordStore<F, K>
where
    F: RootFilesystem,
    K: Clone + Eq + Hash,
{
    pub(crate) fn new(filesystem: Arc<ScopedFilesystem<F>>, cache_max_entries: usize) -> Self {
        Self {
            filesystem,
            path_cache: RwLock::new(HashMap::new()),
            mutation_locks: Mutex::new(HashMap::new()),
            cache_max_entries,
        }
    }

    pub(crate) fn mutation_lock(&self, key: &K) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .mutation_locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locks.retain(|_, lock| Arc::strong_count(lock) > 1);
        locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    pub(crate) fn cached_path<E>(
        &self,
        key: &K,
        derive_path: impl FnOnce(&K) -> Result<ScopedPath, E>,
    ) -> Result<ScopedPath, E> {
        if let Some(path) = self
            .path_cache
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key)
            .cloned()
        {
            return Ok(path);
        }

        let path = derive_path(key)?;
        let mut cache = self
            .path_cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(path) = cache.get(key).cloned() {
            return Ok(path);
        }
        if cache.len() >= self.cache_max_entries
            && let Some(evicted) = cache.keys().next().cloned()
        {
            cache.remove(&evicted);
        }
        cache.insert(key.clone(), path.clone());
        Ok(path)
    }

    pub(crate) async fn get(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.filesystem.get(scope, path).await
    }

    pub(crate) async fn put_json<E>(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        body: Vec<u8>,
        expectation: CasExpectation,
    ) -> Result<(), E>
    where
        E: From<FilesystemError>,
    {
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        match self.filesystem.put(scope, path, entry, expectation).await {
            Ok(_) => Ok(()),
            Err(error) => Err(E::from(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Barrier,
        atomic::{AtomicUsize, Ordering},
    };

    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

    use super::*;

    fn test_store() -> Arc<FilesystemCasRecordStore<InMemoryBackend, String>> {
        let backend = Arc::new(InMemoryBackend::new());
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/records").expect("mount alias"),
            VirtualPath::new("/engine/records").expect("virtual path"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view");
        let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts));
        Arc::new(FilesystemCasRecordStore::new(filesystem, 8))
    }

    #[test]
    fn cached_path_uses_first_insert_under_concurrent_miss() {
        let store = test_store();
        let key = "same-key".to_string();
        let barrier = Arc::new(Barrier::new(2));
        let derive_count = Arc::new(AtomicUsize::new(0));

        let first = {
            let store = Arc::clone(&store);
            let key = key.clone();
            let barrier = Arc::clone(&barrier);
            let derive_count = Arc::clone(&derive_count);
            std::thread::spawn(move || {
                store.cached_path(&key, |_| {
                    derive_count.fetch_add(1, Ordering::SeqCst);
                    barrier.wait();
                    ScopedPath::new("/records/first.json")
                })
            })
        };

        let second = {
            let store = Arc::clone(&store);
            let key = key.clone();
            let barrier = Arc::clone(&barrier);
            let derive_count = Arc::clone(&derive_count);
            std::thread::spawn(move || {
                store.cached_path(&key, |_| {
                    derive_count.fetch_add(1, Ordering::SeqCst);
                    barrier.wait();
                    ScopedPath::new("/records/second.json")
                })
            })
        };

        let first_path = first.join().expect("first task").expect("first path");
        let second_path = second.join().expect("second task").expect("second path");

        assert_eq!(
            second_path.as_str(),
            first_path.as_str(),
            "concurrent misses should converge on the first inserted path"
        );
        assert_eq!(
            derive_count.load(Ordering::SeqCst),
            2,
            "both tasks may derive before the write lock, but cache insertion must be stable"
        );
    }
}
