//! In-memory vector cache keyed by document digest.
//!
//! A cache, not durable state: losing it only costs a re-embed. It is bounded
//! by entry count. Each entry remembers the last fit that used it; when an
//! insert pushes the cache over capacity, the entries used longest ago are
//! evicted first, and entries used by the fit in progress are never evicted.
//! The capacity is always at least the corpus limit, so a whole corpus fits.
//!
//! Keying by digest means a changed tool gets a new key (and a re-embed) while
//! every unchanged tool keeps its vector, and alternating between two
//! catalogs, as different profiles do, reuses both sets until capacity forces
//! a choice.

use std::collections::HashMap;
use std::sync::Arc;

use crate::document::DocumentDigest;

#[derive(Debug)]
struct CachedVector {
    vector: Arc<[f32]>,
    last_used_fit: u64,
}

#[derive(Debug)]
pub(crate) struct VectorCache {
    entries: HashMap<DocumentDigest, CachedVector>,
    capacity: usize,
    fit_generation: u64,
}

impl VectorCache {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            capacity,
            fit_generation: 0,
        }
    }

    /// Start a fit and return its generation, which marks every entry the fit
    /// touches.
    pub(crate) fn begin_fit(&mut self) -> u64 {
        self.fit_generation = self.fit_generation.wrapping_add(1);
        self.fit_generation
    }

    /// The generation of the latest fit, for inserts made outside a fit
    /// (background indexing): they count as used by that fit.
    pub(crate) fn current_generation(&self) -> u64 {
        self.fit_generation
    }

    /// The cached vector for `digest`, marking it as used by `generation`.
    pub(crate) fn get(&mut self, digest: &DocumentDigest, generation: u64) -> Option<Arc<[f32]>> {
        let entry = self.entries.get_mut(digest)?;
        entry.last_used_fit = entry.last_used_fit.max(generation);
        Some(Arc::clone(&entry.vector))
    }

    /// Store `vector` for `digest` as used by `generation`. Call
    /// [`Self::evict_to_capacity`] once the fit has inserted everything.
    pub(crate) fn insert(&mut self, digest: DocumentDigest, vector: Arc<[f32]>, generation: u64) {
        self.entries.insert(
            digest,
            CachedVector {
                vector,
                last_used_fit: generation,
            },
        );
    }

    /// Evict the least recently fitted entries until the cache is within
    /// capacity, never touching an entry `protected_generation` used.
    pub(crate) fn evict_to_capacity(&mut self, protected_generation: u64) {
        let excess = self.entries.len().saturating_sub(self.capacity);
        if excess == 0 {
            return;
        }
        let mut candidates: Vec<(u64, DocumentDigest)> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.last_used_fit < protected_generation)
            .map(|(digest, entry)| (entry.last_used_fit, *digest))
            .collect();
        // Oldest first; the digest breaks ties so eviction is deterministic.
        candidates.sort_unstable();
        for (_, digest) in candidates.into_iter().take(excess) {
            self.entries.remove(&digest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vector(value: f32) -> Arc<[f32]> {
        Arc::from(vec![value])
    }

    #[test]
    fn eviction_drops_the_least_recently_fitted_entries_first() {
        let mut cache = VectorCache::new(2);
        let first = cache.begin_fit();
        cache.insert([1; 32], vector(1.0), first);
        cache.insert([2; 32], vector(2.0), first);
        cache.evict_to_capacity(first);

        let second = cache.begin_fit();
        assert!(cache.get(&[2; 32], second).is_some());
        cache.insert([3; 32], vector(3.0), second);
        cache.evict_to_capacity(second);

        assert_eq!(cache.entries.len(), 2);
        assert!(
            cache.get(&[1; 32], second).is_none(),
            "oldest entry evicted"
        );
        assert!(cache.get(&[2; 32], second).is_some());
        assert!(cache.get(&[3; 32], second).is_some());
    }

    #[test]
    fn entries_of_the_fit_in_progress_are_never_evicted() {
        let mut cache = VectorCache::new(1);
        let generation = cache.begin_fit();
        cache.insert([1; 32], vector(1.0), generation);
        cache.insert([2; 32], vector(2.0), generation);
        cache.evict_to_capacity(generation);
        // Over capacity, but both belong to the current fit.
        assert_eq!(cache.entries.len(), 2);

        let next = cache.begin_fit();
        cache.insert([3; 32], vector(3.0), next);
        cache.evict_to_capacity(next);
        assert_eq!(cache.entries.len(), 1);
        assert!(cache.get(&[3; 32], next).is_some());
    }
}
