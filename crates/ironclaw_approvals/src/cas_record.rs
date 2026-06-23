use std::{
    collections::{HashMap, VecDeque},
    hash::Hash,
    sync::{Arc, Mutex},
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
    path_cache: Mutex<BoundedPathCache<K>>,
    pub(crate) mutation_locks: Mutex<HashMap<K, Arc<tokio::sync::Mutex<()>>>>,
}

struct BoundedPathCache<K>
where
    K: Clone + Eq + Hash,
{
    entries: HashMap<K, ScopedPath>,
    recency: VecDeque<K>,
    max_entries: usize,
}

impl<K> BoundedPathCache<K>
where
    K: Clone + Eq + Hash,
{
    fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            recency: VecDeque::new(),
            max_entries,
        }
    }

    fn get_or_insert_with<E>(
        &mut self,
        key: &K,
        derive_path: impl FnOnce(&K) -> Result<ScopedPath, E>,
    ) -> Result<ScopedPath, E> {
        if let Some(path) = self.entries.get(key).cloned() {
            self.touch(key);
            return Ok(path);
        }
        let path = derive_path(key)?;
        if self.max_entries == 0 {
            return Ok(path);
        }
        while self.entries.len() >= self.max_entries {
            let Some(evicted) = self.recency.pop_front() else {
                break;
            };
            self.entries.remove(&evicted);
        }
        self.entries.insert(key.clone(), path.clone());
        self.recency.push_back(key.clone());
        Ok(path)
    }

    fn touch(&mut self, key: &K) {
        self.recency.retain(|candidate| candidate != key);
        self.recency.push_back(key.clone());
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

impl<F, K> FilesystemCasRecordStore<F, K>
where
    F: RootFilesystem,
    K: Clone + Eq + Hash,
{
    pub(crate) fn new(filesystem: Arc<ScopedFilesystem<F>>, cache_max_entries: usize) -> Self {
        Self {
            filesystem,
            path_cache: Mutex::new(BoundedPathCache::new(cache_max_entries)),
            mutation_locks: Mutex::new(HashMap::new()),
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
        self.path_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_or_insert_with(key, derive_path)
    }

    #[cfg(test)]
    pub(crate) fn path_cache_len(&self) -> usize {
        self.path_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    #[cfg(test)]
    pub(crate) fn mutation_lock_count(&self) -> usize {
        self.mutation_locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
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
    use std::convert::Infallible;

    use super::*;

    fn path(name: &str) -> ScopedPath {
        ScopedPath::new(format!("/approvals/{name}.json")).expect("scoped path")
    }

    #[test]
    fn bounded_path_cache_evicts_least_recently_used_entry() {
        let mut cache = BoundedPathCache::new(2);
        cache
            .get_or_insert_with(&"a", |_| Ok::<_, Infallible>(path("a")))
            .unwrap();
        cache
            .get_or_insert_with(&"b", |_| Ok::<_, Infallible>(path("b")))
            .unwrap();
        cache
            .get_or_insert_with(&"a", |_| Ok::<_, Infallible>(path("a2")))
            .unwrap();
        cache
            .get_or_insert_with(&"c", |_| Ok::<_, Infallible>(path("c")))
            .unwrap();

        let a = cache
            .get_or_insert_with(&"a", |_| Ok::<_, Infallible>(path("a-miss")))
            .unwrap();
        let b = cache
            .get_or_insert_with(&"b", |_| Ok::<_, Infallible>(path("b-reloaded")))
            .unwrap();

        assert_eq!(a, path("a"));
        assert_eq!(b, path("b-reloaded"));
        assert_eq!(cache.len(), 2);
    }
}
