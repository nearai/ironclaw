//! Trait-level contract test harness for [`MemoryDocumentRepository`].
//!
//! This module establishes a **scaffolding pattern**: a single set of
//! invariants the trait promises is defined once here, and every impl
//! wires the suite once with a factory closure. The point is to move
//! coverage from "this mock has property X" to "every impl of this
//! trait has property X — by construction".
//!
//! ## Why this exists
//!
//! Across several PR reviews (#3890, #3887, #3908) the same shape
//! recurred: a trait has multiple impls, isolation/durability/CAS
//! invariants every impl must honor, but tests only cover one impl —
//! often a mock that quietly implements its own invariants. Per
//! `.claude/rules/testing.md` ("Test Through the Caller, Not Just the
//! Helper"), a contract test against one impl proves only that impl,
//! not the contract. #3890 in particular found a search-isolation gap
//! that would have been impossible if every impl was forced through
//! the same suite.
//!
//! ## Shape
//!
//! Each contract is a `pub async fn` taking a factory closure
//! `Fn() -> R`. The factory must produce a **fresh** repository per
//! call so suites cannot share state between contracts.
//!
//! Per-impl test files wire the suite via the [`contract_test!`]
//! macro, which expands to one `#[tokio::test]` per contract function
//! named `<impl_label>::<contract_name>` for clear failure
//! attribution.
//!
//! ## Non-goals
//!
//! This first scaffold intentionally covers a small surface
//! (isolation, round-trip, list filtering, search isolation). The
//! shape — not the breadth — is the point. Follow-up PRs can extend
//! the suite (CAS, metadata, append outcomes) and port other traits
//! (`IdempotencyLedger`, `CheckpointStateStore`, `ProcessStore`, …)
//! onto the same pattern.

use ironclaw_filesystem::FilesystemError;

use crate::chunking::{MemoryChunkWrite, content_sha256};
use crate::indexer::{MemoryChunkReplaceOutcome, MemoryDocumentIndexRepository};
use crate::path::{MemoryDocumentPath, MemoryDocumentScope};
use crate::repo::MemoryDocumentRepository;
use crate::search::MemorySearchRequest;

#[cfg(any(test, feature = "contract-tests"))]
fn scope_a() -> MemoryDocumentScope {
    MemoryDocumentScope::new("tenant-a", "alice", Some("project-1")).expect("valid scope a")
}

#[cfg(any(test, feature = "contract-tests"))]
fn scope_b() -> MemoryDocumentScope {
    MemoryDocumentScope::new("tenant-b", "bob", Some("project-1")).expect("valid scope b")
}

#[cfg(any(test, feature = "contract-tests"))]
fn path_in(scope: &MemoryDocumentScope, relative: &str) -> MemoryDocumentPath {
    MemoryDocumentPath::new(
        scope.tenant_id(),
        scope.user_id(),
        scope.project_id(),
        relative,
    )
    .expect("valid memory document path")
}

/// Contract: a put followed by a get returns the same bytes.
#[cfg(any(test, feature = "contract-tests"))]
pub async fn round_trip_returns_written_bytes<R, F>(factory: F)
where
    R: MemoryDocumentRepository,
    F: Fn() -> R,
{
    let repo = factory();
    let path = path_in(&scope_a(), "notes/round-trip.md");
    repo.write_document(&path, b"hello world").await.unwrap();
    let read = repo.read_document(&path).await.unwrap();
    assert_eq!(
        read.as_deref(),
        Some(&b"hello world"[..]),
        "round-trip must return exact bytes written"
    );
}

/// Contract: writes in scope A must not surface to reads in scope B.
///
/// This is the load-bearing invariant flagged in #3890 — search and
/// list isolation across tenants. Every impl must honor it; the
/// harness is the place to assert it once.
#[cfg(any(test, feature = "contract-tests"))]
pub async fn writes_isolated_across_scopes<R, F>(factory: F)
where
    R: MemoryDocumentRepository,
    F: Fn() -> R,
{
    let repo = factory();
    let a = path_in(&scope_a(), "notes/secret.md");
    let b = path_in(&scope_b(), "notes/secret.md");

    repo.write_document(&a, b"tenant-a secret").await.unwrap();

    // Scope B must not see scope A's write, even at the identical
    // relative path.
    let cross = repo.read_document(&b).await.unwrap();
    assert!(
        cross.is_none(),
        "scope B must not see scope A's bytes at the same relative path"
    );

    // Scope A still sees its own write.
    let same = repo.read_document(&a).await.unwrap();
    assert_eq!(same.as_deref(), Some(&b"tenant-a secret"[..]));
}

/// Contract: list_documents honors scope.
#[cfg(any(test, feature = "contract-tests"))]
pub async fn list_documents_honors_scope<R, F>(factory: F)
where
    R: MemoryDocumentRepository,
    F: Fn() -> R,
{
    let repo = factory();
    let scope_a = scope_a();
    let scope_b = scope_b();

    repo.write_document(&path_in(&scope_a, "notes/a1.md"), b"a1")
        .await
        .unwrap();
    repo.write_document(&path_in(&scope_a, "notes/a2.md"), b"a2")
        .await
        .unwrap();
    repo.write_document(&path_in(&scope_b, "notes/b1.md"), b"b1")
        .await
        .unwrap();

    let listed_a = repo.list_documents(&scope_a).await.unwrap();
    assert_eq!(
        listed_a.len(),
        2,
        "scope A must see exactly its own documents (got {listed_a:?})"
    );
    assert!(
        listed_a.iter().all(|p| p.scope() == &scope_a),
        "list_documents must not return cross-scope paths"
    );

    let listed_b = repo.list_documents(&scope_b).await.unwrap();
    assert_eq!(
        listed_b.len(),
        1,
        "scope B must see exactly its own documents (got {listed_b:?})"
    );
    assert!(
        listed_b.iter().all(|p| p.scope() == &scope_b),
        "list_documents must not return cross-scope paths"
    );
}

/// Contract: an impl that opts out of search must do so via the
/// documented unsupported error, not a panic-shaped one.
///
/// The trait lets impls opt out of search; the default
/// [`MemoryDocumentRepository::search_documents`] returns
/// `memory_backend_unsupported`. That helper emits
/// [`FilesystemError::Backend`] carrying a sanitized
/// `"...does not support search"` reason — **not**
/// [`FilesystemError::Unsupported`]. The two are distinct by design:
/// `Unsupported` is reserved for mount-time capability mismatches and
/// carries only `{ path, operation }` (no `reason` field), so it cannot
/// carry the human-readable opt-out message. Asserting on `Backend` here
/// reflects the real contract; the earlier `Unsupported`-shaped
/// assertion never fired and was caught only by the string fallback.
///
/// Impls that *do* support search are exercised by
/// [`search_documents_isolated_across_scopes`] instead, which seeds
/// chunk records so the FTS query returns real hits.
#[cfg(any(test, feature = "contract-tests"))]
pub async fn search_documents_unsupported_is_documented<R, F>(factory: F)
where
    R: MemoryDocumentRepository,
    F: Fn() -> R,
{
    let repo = factory();
    let scope_a = scope_a();
    repo.write_document(&path_in(&scope_a, "notes/needle.md"), b"needle")
        .await
        .unwrap();

    let request = MemorySearchRequest::new("needle").expect("valid search request");

    match repo.search_documents(&scope_a, &request).await {
        Ok(_) => {
            // This contract is for opt-out impls only; impls that
            // support search are covered by
            // `search_documents_isolated_across_scopes`.
        }
        Err(err) => {
            assert!(
                matches!(err, FilesystemError::Backend { ref reason, .. } if reason.to_lowercase().contains("not support")),
                "search_documents opt-out must surface the documented \
                 FilesystemError::Backend(\"...does not support search\") \
                 error, got: {err:?}"
            );
        }
    }
}

/// Contract: search_documents must not leak across tenant scopes.
///
/// This is the *exact* class of bug surfaced in #3890. Wired only for
/// impls that also implement [`MemoryDocumentIndexRepository`] (i.e.
/// impls that actually serve search). The contract **seeds chunk
/// records** in both tenants so the FTS search returns real hits — an
/// earlier version wrote only documents and no chunks, so
/// `search_documents` (which queries the `.chunks/` subtree) returned
/// `Ok([])` and the isolation assertion passed vacuously. With chunks
/// seeded, scope A's search must return its own hit and must NOT return
/// scope B's identical-content hit; if the impl ignored scope, this test
/// fails on the cross-tenant leak assertion.
#[cfg(any(test, feature = "contract-tests"))]
pub async fn search_documents_isolated_across_scopes<R, F>(factory: F)
where
    R: MemoryDocumentRepository + MemoryDocumentIndexRepository,
    F: Fn() -> R,
{
    let repo = factory();
    let scope_a = scope_a();
    let scope_b = scope_b();

    // Both tenants share the query token ("needle") but carry a
    // tenant-unique marker word in their chunk content. The marker is
    // what makes the leak observable: an impl that re-anchors result
    // *paths* to the requested scope (so a per-hit `path.scope()` check
    // cannot catch a prefix leak) still returns the leaked chunk's
    // *content*, so a stray "bravo" snippet in scope A's results exposes
    // the cross-tenant read.
    //
    // The two documents use *distinct* relative paths. They previously
    // shared `notes/needle.md`, but `fuse_memory_search_results` keys its
    // accumulator solely on `relative_path` (scope is not part of the
    // key). With identical paths a leaked scope-B hit would collapse into
    // the scope-A accumulator, and because fusion keeps the first-inserted
    // snippet (and_modify never replaces it), the "bravo" marker could be
    // discarded before the assertions run — masking the very leak this
    // test exists to catch. Distinct paths keep the leaked hit as its own
    // result so the marker survives fusion.
    let body_a = "alpha quick brown needle";
    let body_b = "bravo quick brown needle";
    let path_a = path_in(&scope_a, "notes/needle-a.md");
    let path_b = path_in(&scope_b, "notes/needle-b.md");
    repo.write_document(&path_a, body_a.as_bytes())
        .await
        .unwrap();
    repo.write_document(&path_b, body_b.as_bytes())
        .await
        .unwrap();

    // Seed chunk records for both documents — without these the FTS
    // search over the `.chunks/` subtree returns nothing and the
    // isolation assertion below would be vacuously satisfied.
    let chunk = |content: &str| {
        vec![MemoryChunkWrite {
            content: content.to_string(),
            embedding: None,
        }]
    };
    assert_eq!(
        repo.replace_document_chunks_if_current(&path_a, &content_sha256(body_a), &chunk(body_a))
            .await
            .unwrap(),
        MemoryChunkReplaceOutcome::Replaced,
        "seeding scope-A chunks must succeed for the search to have hits"
    );
    assert_eq!(
        repo.replace_document_chunks_if_current(&path_b, &content_sha256(body_b), &chunk(body_b))
            .await
            .unwrap(),
        MemoryChunkReplaceOutcome::Replaced,
        "seeding scope-B chunks must succeed"
    );

    let request = MemorySearchRequest::new("needle").expect("valid search request");

    let hits = repo
        .search_documents(&scope_a, &request)
        .await
        .expect("indexed impls must support search");

    // The search must actually return hits — proves we are exercising
    // the contract, not passing vacuously on an empty result set.
    assert!(
        !hits.is_empty(),
        "scope-A search returned no hits; the test is not exercising \
         search isolation (chunk seeding may have failed)"
    );
    // Every hit must belong to scope A by path...
    for hit in &hits {
        assert_eq!(
            hit.path.scope(),
            &scope_a,
            "search_documents leaked a cross-tenant hit: {:?}",
            hit.path
        );
    }
    // ...and no returned snippet may carry scope B's unique marker. This
    // is the leak-catching assertion: an impl that queries a
    // cross-tenant prefix surfaces scope B's "bravo" chunk content even
    // if it re-stamps the result path to scope A.
    assert!(
        hits.iter().all(|hit| !hit.snippet.contains("bravo")),
        "search_documents leaked scope-B chunk content into scope A: {hits:?}"
    );
}

/// Wire the standard [`MemoryDocumentRepository`] contract suite for a
/// concrete impl.
///
/// Usage (per-impl test file):
///
/// ```ignore
/// use ironclaw_memory::{InMemoryMemoryDocumentRepository, contract_test};
///
/// contract_test!(in_memory, || InMemoryMemoryDocumentRepository::new());
/// ```
///
/// The macro expands to one `#[tokio::test]` per contract, each named
/// `<impl_label>::<contract_name>`. This means a failure in the
/// filesystem impl's search-isolation contract shows up as
/// `filesystem::search_documents_isolated_across_scopes` in the test
/// output — clear attribution, no shared mutable state across tests.
///
/// `$factory` must be a closure (or `fn`) `Fn() -> R` returning a
/// fresh repository per call. Factories may capture (e.g. a `tempdir`
/// or an `Arc<RootFilesystem>` constructed inside the closure) but
/// must not share writable state across invocations — each contract
/// gets its own repository instance.
///
/// Use [`contract_test!`] for impls that opt out of search (only
/// implement [`MemoryDocumentRepository`]); use
/// [`contract_test_indexed!`] for impls that also implement
/// [`MemoryDocumentIndexRepository`] and therefore serve search — the
/// latter additionally wires the real (chunk-seeded) search-isolation
/// contract.
#[macro_export]
macro_rules! contract_test {
    ($label:ident, $factory:expr) => {
        mod $label {
            // Re-import here so callers don't have to drag in every
            // contract function name.
            use super::*;

            #[tokio::test]
            async fn round_trip_returns_written_bytes() {
                $crate::contract_tests::round_trip_returns_written_bytes($factory).await;
            }

            #[tokio::test]
            async fn writes_isolated_across_scopes() {
                $crate::contract_tests::writes_isolated_across_scopes($factory).await;
            }

            #[tokio::test]
            async fn list_documents_honors_scope() {
                $crate::contract_tests::list_documents_honors_scope($factory).await;
            }

            #[tokio::test]
            async fn search_documents_unsupported_is_documented() {
                $crate::contract_tests::search_documents_unsupported_is_documented($factory).await;
            }
        }
    };
}

/// Wire the contract suite for an impl that implements both
/// [`MemoryDocumentRepository`] and [`MemoryDocumentIndexRepository`]
/// (i.e. an impl that actually serves search).
///
/// Expands to every base contract from [`contract_test!`] plus
/// `search_documents_isolated_across_scopes`, which seeds chunk records
/// in two tenants and asserts a scope-A search returns only scope-A
/// hits. See the function docs for why chunk seeding is load-bearing
/// (without it the search returns `Ok([])` and the isolation assertion
/// is vacuous).
#[macro_export]
macro_rules! contract_test_indexed {
    ($label:ident, $factory:expr) => {
        mod $label {
            use super::*;

            #[tokio::test]
            async fn round_trip_returns_written_bytes() {
                $crate::contract_tests::round_trip_returns_written_bytes($factory).await;
            }

            #[tokio::test]
            async fn writes_isolated_across_scopes() {
                $crate::contract_tests::writes_isolated_across_scopes($factory).await;
            }

            #[tokio::test]
            async fn list_documents_honors_scope() {
                $crate::contract_tests::list_documents_honors_scope($factory).await;
            }

            #[tokio::test]
            async fn search_documents_isolated_across_scopes() {
                $crate::contract_tests::search_documents_isolated_across_scopes($factory).await;
            }
        }
    };
}
