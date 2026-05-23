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

use crate::path::{MemoryDocumentPath, MemoryDocumentScope};
use crate::repo::MemoryDocumentRepository;
use crate::search::MemorySearchRequest;

/// Factory closure shape every contract takes.
///
/// Must return a fresh, empty repository — contracts assume nothing
/// leaks between calls.
pub type RepoFactory<R> = fn() -> R;

fn scope_a() -> MemoryDocumentScope {
    MemoryDocumentScope::new("tenant-a", "alice", Some("project-1")).expect("valid scope a")
}

fn scope_b() -> MemoryDocumentScope {
    MemoryDocumentScope::new("tenant-b", "bob", Some("project-1")).expect("valid scope b")
}

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

/// Contract: search_documents must not leak across tenant scopes.
///
/// This is the *exact* class of bug surfaced in #3890. The trait
/// allows impls to opt out of search by returning
/// `memory_backend_unsupported` (the default impl does this); the
/// contract asserts the **stronger of**: either search is unsupported,
/// or it is scope-isolated. Either is acceptable; a search that
/// returns cross-tenant hits is a bug.
pub async fn search_documents_isolated_across_scopes<R, F>(factory: F)
where
    R: MemoryDocumentRepository,
    F: Fn() -> R,
{
    let repo = factory();
    let scope_a = scope_a();
    let scope_b = scope_b();

    // Same query token in both tenants.
    repo.write_document(
        &path_in(&scope_a, "notes/needle.md"),
        b"the quick brown needle",
    )
    .await
    .unwrap();
    repo.write_document(
        &path_in(&scope_b, "notes/needle.md"),
        b"the quick brown needle",
    )
    .await
    .unwrap();

    let request = MemorySearchRequest::new("needle").expect("valid search request");

    match repo.search_documents(&scope_a, &request).await {
        Ok(hits) => {
            // If the impl supports search, every hit MUST be in scope A.
            for hit in &hits {
                assert_eq!(
                    hit.path.scope(),
                    &scope_a,
                    "search_documents leaked a cross-tenant hit: {:?}",
                    hit.path
                );
            }
        }
        Err(err) => {
            // Opting out of search is acceptable; the trait permits it.
            // We only enforce that the failure is the documented one
            // ("backend does not support search") and not a generic
            // panic-shaped error.
            assert!(
                matches!(err, FilesystemError::Unsupported { .. })
                    || err.to_string().to_lowercase().contains("not support"),
                "search_documents error must be the documented unsupported \
                 variant, got: {err:?}"
            );
        }
    }
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
            async fn search_documents_isolated_across_scopes() {
                $crate::contract_tests::search_documents_isolated_across_scopes($factory).await;
            }
        }
    };
}
