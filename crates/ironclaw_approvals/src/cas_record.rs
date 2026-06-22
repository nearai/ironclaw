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
