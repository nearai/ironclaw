//! ERC-7730 clear-signing descriptors: the lookup key, the source port, and
//! the TTL cache (attested-signing §D3).
//!
//! ## Why a proxy exists at all
//!
//! The WebUI has a zero-remote-origins policy and a strict CSP, so the browser
//! cannot fetch descriptors from Ledger's context service directly. The backend
//! fetches them instead and serves them same-origin. That constraint turns out
//! to be a feature: it gives one audit point, one cache, one allowlist, and a
//! future pinning surface.
//!
//! ## Fail closed, and say so
//!
//! [`DescriptorLookup::NotAvailable`] is a first-class outcome, not an error to
//! be papered over. When no descriptor covers a transaction the device cannot
//! render its fields, and the only honest thing to show the human is that this
//! transaction cannot be clear-signed — never a blind-sign button. A cache miss
//! plus an upstream failure produce the SAME outcome as a genuine absence, so a
//! degraded context service cannot quietly downgrade the ceremony into blind
//! signing. Availability is deliberately traded for that.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

/// What a descriptor is looked up by.
///
/// The triple that decides which ERC-7730 descriptor applies: the chain, the
/// contract being called, and the function selector within it. A transaction
/// with no `to` (a deployment) or no selector (a bare value transfer) has no
/// contract call to describe, which is why both are optional here and why
/// [`Self::from_call`] refuses to invent one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DescriptorKey {
    /// CAIP-2 chain id.
    pub chain_id: String,
    /// Lowercase `0x`-prefixed contract address.
    pub contract: String,
    /// Lowercase `0x`-prefixed 4-byte selector.
    pub selector: String,
}

impl DescriptorKey {
    /// Build a key from a call's parts, or `None` when there is no contract
    /// call to describe.
    ///
    /// Case is normalized: a descriptor must not miss because an address
    /// arrived checksummed one time and lowercase the next — that miss would
    /// present to the user as "cannot be clear-signed" and train them to
    /// distrust the blocked state.
    pub fn from_call(chain_id: &str, to: Option<&str>, data: &[u8]) -> Option<Self> {
        let contract = to?;
        if data.len() < 4 {
            return None;
        }
        Some(Self {
            chain_id: chain_id.to_ascii_lowercase(),
            contract: contract.to_ascii_lowercase(),
            selector: format!("0x{}", hex::encode(&data[..4])),
        })
    }
}

/// The result of a descriptor lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescriptorLookup {
    /// A descriptor the device can render from.
    Available {
        /// The descriptor document, passed through verbatim. The backend does
        /// not interpret it — the device and the DMK context module do.
        descriptor: serde_json::Value,
    },
    /// No descriptor. The ceremony must block, visibly.
    NotAvailable,
}

/// Where descriptors come from.
///
/// A port so tests never touch the network, and so the allowlist/TTL policy
/// composes around whatever concrete source a deployment wires.
#[async_trait]
pub trait DescriptorSource: Send + Sync {
    /// Look one up. An upstream failure must surface as
    /// [`DescriptorLookup::NotAvailable`] rather than an error, so every path
    /// that cannot produce a descriptor converges on the same blocked UX.
    async fn lookup(&self, key: &DescriptorKey) -> DescriptorLookup;
}

/// Shared sources are sources too, so composition can hold the port behind an
/// `Arc<dyn DescriptorSource>` and still cache over it.
#[async_trait]
impl<T: DescriptorSource + ?Sized> DescriptorSource for std::sync::Arc<T> {
    async fn lookup(&self, key: &DescriptorKey) -> DescriptorLookup {
        (**self).lookup(key).await
    }
}

/// A source that has nothing, for deployments with clear signing unconfigured.
///
/// Not a degenerate stub: with no descriptor service wired, every Ledger
/// ceremony must block, and this is what makes that the default rather than
/// something a deployment opts into.
pub struct UnconfiguredDescriptorSource;

#[async_trait]
impl DescriptorSource for UnconfiguredDescriptorSource {
    async fn lookup(&self, _key: &DescriptorKey) -> DescriptorLookup {
        DescriptorLookup::NotAvailable
    }
}

/// Cache entry with its expiry.
struct CachedDescriptor {
    lookup: DescriptorLookup,
    expires_at_ms: i64,
}

/// A TTL cache in front of a [`DescriptorSource`].
///
/// Negative results are cached too, and deliberately for a shorter window: a
/// transient upstream outage should not pin "cannot be clear-signed" for the
/// full positive TTL, but an un-cached miss would let a page reload hammer the
/// context service.
pub struct TtlDescriptorCache<S> {
    source: S,
    entries: Mutex<HashMap<DescriptorKey, CachedDescriptor>>,
    hit_ttl_ms: i64,
    miss_ttl_ms: i64,
}

impl<S> TtlDescriptorCache<S> {
    /// Wrap a source with explicit TTLs.
    pub fn new(source: S, hit_ttl_ms: i64, miss_ttl_ms: i64) -> Self {
        Self {
            source,
            entries: Mutex::new(HashMap::new()),
            hit_ttl_ms,
            miss_ttl_ms,
        }
    }

    /// Entries currently held, for tests and diagnostics.
    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .map(|entries| entries.len())
            .unwrap_or(0)
    }

    /// Whether the cache holds nothing.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<S: DescriptorSource> TtlDescriptorCache<S> {
    /// Look up through the cache.
    ///
    /// `now_ms` is supplied by the caller — this type never reads the wall
    /// clock, so expiry is exactly testable.
    pub async fn lookup(&self, key: &DescriptorKey, now_ms: i64) -> DescriptorLookup {
        if let Ok(entries) = self.entries.lock()
            && let Some(entry) = entries.get(key)
            && entry.expires_at_ms > now_ms
        {
            return entry.lookup.clone();
        }

        let lookup = self.source.lookup(key).await;
        let ttl = match lookup {
            DescriptorLookup::Available { .. } => self.hit_ttl_ms,
            DescriptorLookup::NotAvailable => self.miss_ttl_ms,
        };
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(
                key.clone(),
                CachedDescriptor {
                    lookup: lookup.clone(),
                    expires_at_ms: now_ms.saturating_add(ttl),
                },
            );
        }
        lookup
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts calls so cache behaviour is observable, and can be scripted to
    /// fail — the outage case that must not become blind signing.
    struct ScriptedSource {
        result: DescriptorLookup,
        calls: AtomicUsize,
    }

    impl ScriptedSource {
        fn available() -> Self {
            Self {
                result: DescriptorLookup::Available {
                    descriptor: serde_json::json!({ "display": { "formats": {} } }),
                },
                calls: AtomicUsize::new(0),
            }
        }

        fn unavailable() -> Self {
            Self {
                result: DescriptorLookup::NotAvailable,
                calls: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::Relaxed)
        }
    }

    #[async_trait]
    impl DescriptorSource for ScriptedSource {
        async fn lookup(&self, _key: &DescriptorKey) -> DescriptorLookup {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.result.clone()
        }
    }

    fn key() -> DescriptorKey {
        DescriptorKey {
            chain_id: "eip155:1".to_string(),
            contract: "0xa0b8".to_string(),
            selector: "0xa9059cbb".to_string(),
        }
    }

    #[test]
    fn a_key_is_built_from_the_call_and_normalized() {
        let data = hex::decode("a9059cbb00000000").expect("hex");
        let built = DescriptorKey::from_call("EIP155:1", Some("0xAbCdEf"), &data).expect("a key");
        assert_eq!(built.chain_id, "eip155:1", "chain case is normalized");
        assert_eq!(built.contract, "0xabcdef", "address case is normalized");
        assert_eq!(built.selector, "0xa9059cbb");
    }

    /// A checksummed address and a lowercase one are the same contract. If they
    /// keyed differently, a descriptor would appear to be missing — presenting
    /// as "cannot be clear-signed" and teaching the user to distrust that state.
    #[test]
    fn address_casing_does_not_split_the_key() {
        let data = hex::decode("a9059cbb").expect("hex");
        assert_eq!(
            DescriptorKey::from_call("eip155:1", Some("0xAAbb"), &data),
            DescriptorKey::from_call("eip155:1", Some("0xaabb"), &data)
        );
    }

    /// A plain value transfer and a deployment have no contract call to
    /// describe. Inventing a key would look up a descriptor for something that
    /// is not a call.
    #[test]
    fn a_call_with_nothing_to_describe_has_no_key() {
        assert_eq!(DescriptorKey::from_call("eip155:1", None, &[0xa9; 8]), None);
        // Fewer than four bytes is not a selector.
        assert_eq!(
            DescriptorKey::from_call("eip155:1", Some("0xabcd"), &[0xa9, 0x05, 0x9c]),
            None
        );
        assert_eq!(
            DescriptorKey::from_call("eip155:1", Some("0xabcd"), &[]),
            None
        );
    }

    #[tokio::test]
    async fn a_hit_is_served_from_cache_until_it_expires() {
        let cache = TtlDescriptorCache::new(ScriptedSource::available(), 1_000, 100);

        let first = cache.lookup(&key(), 0).await;
        assert!(matches!(first, DescriptorLookup::Available { .. }));
        cache.lookup(&key(), 500).await;
        assert_eq!(cache.source.calls(), 1, "served from cache within the TTL");

        // At the boundary the entry is stale (expiry is exclusive).
        cache.lookup(&key(), 1_000).await;
        assert_eq!(cache.source.calls(), 2, "refetched once expired");
    }

    /// THE property of this module. An upstream outage and a genuine absence
    /// must be indistinguishable, so a degraded context service can never
    /// quietly downgrade the ceremony into blind signing.
    #[tokio::test]
    async fn an_unavailable_descriptor_is_never_an_error_the_caller_can_bypass() {
        let cache = TtlDescriptorCache::new(ScriptedSource::unavailable(), 1_000, 100);
        assert_eq!(
            cache.lookup(&key(), 0).await,
            DescriptorLookup::NotAvailable,
            "the blocked outcome is a value, not an Err a caller might unwrap_or into a bypass"
        );
    }

    /// Negative caching exists so a reload cannot hammer the context service,
    /// but on a SHORTER window so a transient outage does not pin the blocked
    /// state for the full positive TTL.
    #[tokio::test]
    async fn a_miss_is_cached_briefly_and_recovers_sooner_than_a_hit_expires() {
        let cache = TtlDescriptorCache::new(ScriptedSource::unavailable(), 1_000, 100);

        cache.lookup(&key(), 0).await;
        cache.lookup(&key(), 50).await;
        assert_eq!(cache.source.calls(), 1, "a miss is cached too");

        cache.lookup(&key(), 100).await;
        assert_eq!(
            cache.source.calls(),
            2,
            "and recovers at the shorter miss TTL, well before the hit TTL"
        );
    }

    #[tokio::test]
    async fn distinct_keys_do_not_share_an_entry() {
        let cache = TtlDescriptorCache::new(ScriptedSource::available(), 1_000, 100);
        let other = DescriptorKey {
            selector: "0x23b872dd".to_string(),
            ..key()
        };

        cache.lookup(&key(), 0).await;
        cache.lookup(&other, 0).await;
        assert_eq!(cache.source.calls(), 2);
        assert_eq!(cache.len(), 2);
    }

    /// With nothing wired, every ceremony blocks. Clear signing must be
    /// something a deployment turns ON, never something it forgets to turn off.
    #[tokio::test]
    async fn an_unconfigured_deployment_blocks_every_ceremony() {
        assert_eq!(
            UnconfiguredDescriptorSource.lookup(&key()).await,
            DescriptorLookup::NotAvailable
        );
    }
}
