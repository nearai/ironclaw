//! Anti-slippage ratchet for the product-surface method contract (§5.2 /
//! §5.2.5 / §10 of
//! `docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`).
//!
//! §5.2's target: product consumers share the neutral
//! `ironclaw_host_api::ProductSurface` vocabulary (`invoke`, `query`, and
//! `stream_events`). Feature work adds a **capability descriptor** and/or a
//! **view descriptor**, never a product-local facade method.
//!
//! This test freezes that shared host API method set and asserts the
//! old product-local facade trait stays retired. The
//! ratchet fails on any subsequent change:
//!
//! - a **new** host `ProductSurface` method fails — the feature belongs in a
//!   capability/view descriptor, not a new facade method;
//! - a **removed** host method fails — callers share this exact three-word
//!   vocabulary unless §5.2 is intentionally revised;
//! - a **duplicate** method name in the block fails (defensive; a trait cannot
//!   legally declare two, but the scan is explicit about it).
//!
//! Scoped to the *method set*: the extractor reads only the host
//! `ProductSurface` trait block, at trait-declaration depth (a `fn` inside a
//! default-method body is ignored), with comments and string literals stripped (shared
//! [`ratchet_support::strip_comments_and_strings`]).
//!
//! Definition of done for this axis (§5.2.5 step 5 / §10): product consumers use
//! `host_api::ProductSurface` (`invoke`, `query`, `stream_events`) plus
//! descriptors, and the product crate does not reintroduce its own product
//! surface trait vocabulary.

// This ratchet uses only the shared stripper + workspace-root helper; the
// type-def scanners in the shared module are unreachable from this binary
// (each test binary compiles the whole module and uses a different subset).
#[allow(dead_code)]
mod ratchet_support;

use std::collections::BTreeSet;

use ratchet_support::{strip_comments_and_strings, workspace_root};

/// Path (relative to the workspace root) of the crate that defines the shared
/// product-surface contract.
const HOST_PRODUCT_SURFACE_SOURCE: &str = "crates/ironclaw_host_api/src/product_surface.rs";
/// Product implementation source that must not grow another local ProductSurface
/// trait.
const PRODUCT_REBORN_SERVICES_SOURCE: &str = "crates/ironclaw_product/src/reborn_services.rs";
const PRODUCT_SURFACE_TRAIT: &str = "ProductSurface";
const RETIRED_PROTO_FACADE_TRAIT: &str = "RebornServicesApi";

/// The frozen inventory of shared `host_api::ProductSurface` methods.
const EXPECTED_HOST_PRODUCT_SURFACE_METHODS: &[&str] = &["invoke", "query", "stream_events"];

/// Extract the method names declared **directly** in `trait <trait_name>`'s block
/// — i.e. at trait-declaration depth, so a `fn` nested inside a default-method
/// body is not a facade method. Operates on comment-/string-stripped source and
/// walks brace depth so multi-line signatures and default bodies are handled
/// uniformly. Returns names in source order (duplicates preserved).
fn extract_trait_methods(source: &str, trait_name: &str) -> Vec<String> {
    let stripped = strip_comments_and_strings(source);
    let decl = format!("trait {trait_name}");
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    // Word-boundary match so a rename that keeps the same method set —
    // `trait ProductSurfaceV2`, `ProductSurface_legacy`, or a `subtrait`-
    // like prefix — does NOT silently bind here and defeat the rename guard
    // (#6292 IronLoop/Gemini): `trait` must start a word and the char right
    // after the trait name must not be an identifier char.
    let mut decl_pos = None;
    let mut search_from = 0;
    while let Some(rel) = stripped[search_from..].find(&decl) {
        let pos = search_from + rel;
        let after = pos + decl.len();
        let before_ok = pos == 0
            || stripped[..pos]
                .chars()
                .next_back()
                .is_none_or(|c| !is_word(c));
        let after_ok = stripped[after..].chars().next().is_none_or(|c| !is_word(c));
        if before_ok && after_ok {
            decl_pos = Some(pos);
            break;
        }
        search_from = after;
    }
    let Some(decl_pos) = decl_pos else {
        return Vec::new();
    };
    let after_decl = &stripped[decl_pos..];
    let Some(brace_off) = after_decl.find('{') else {
        return Vec::new();
    };
    let chars: Vec<char> = after_decl[brace_off..].chars().collect();

    let mut methods = Vec::new();
    let mut depth: i32 = 0;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' {
            depth += 1;
            i += 1;
            continue;
        }
        if c == '}' {
            depth -= 1;
            i += 1;
            if depth == 0 {
                break; // end of the trait block
            }
            continue;
        }
        if is_word(c) {
            let start = i;
            while i < chars.len() && is_word(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            // A trait method is `fn NAME` seen at trait-declaration depth (1).
            // `async` reads as its own word and is skipped; the body `{` that
            // follows a default method pushes depth to 2, hiding inner `fn`s.
            if depth == 1 && word == "fn" {
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }
                let name_start = i;
                while i < chars.len() && is_word(chars[i]) {
                    i += 1;
                }
                let name: String = chars[name_start..i].iter().collect();
                if !name.is_empty() {
                    methods.push(name);
                }
            }
            continue;
        }
        i += 1;
    }
    methods
}

#[test]
fn host_product_surface_method_set_is_frozen() {
    let source_path = workspace_root().join(HOST_PRODUCT_SURFACE_SOURCE);
    let source = std::fs::read_to_string(&source_path)
        .unwrap_or_else(|e| panic!("failed to read product surface source {source_path:?}: {e}"));

    let found = extract_trait_methods(&source, PRODUCT_SURFACE_TRAIT);
    assert!(
        !found.is_empty(),
        "no `{PRODUCT_SURFACE_TRAIT}` methods were extracted from {HOST_PRODUCT_SURFACE_SOURCE}: \
         the trait was renamed, moved, or the extractor no longer recognizes its block — update \
         this ratchet to keep tracking the shared product-surface contract."
    );

    // Duplicate detection (defensive — a trait cannot legally declare two, but a
    // silent extractor bug would otherwise mask a swap).
    let mut seen = BTreeSet::new();
    let duplicated: Vec<&String> = found
        .iter()
        .filter(|m| !seen.insert((*m).clone()))
        .collect();
    assert!(
        duplicated.is_empty(),
        "`{PRODUCT_SURFACE_TRAIT}` block yielded duplicate method names {duplicated:?} — the \
         extractor or the trait is malformed."
    );

    let expected: BTreeSet<&str> = EXPECTED_HOST_PRODUCT_SURFACE_METHODS
        .iter()
        .copied()
        .collect();
    let found_set: BTreeSet<&str> = found.iter().map(String::as_str).collect();

    let added: Vec<&str> = found_set.difference(&expected).copied().collect();
    assert!(
        added.is_empty(),
        "New `{PRODUCT_SURFACE_TRAIT}` methods are banned (arch-simplification §5.2/§5.2.5/§10): \
         a new product operation is a matrix-declared capability descriptor or a view descriptor, \
         never a facade method. Offending new methods: {added:?}."
    );

    let removed: Vec<&str> = expected.difference(&found_set).copied().collect();
    assert!(
        removed.is_empty(),
        "Expected shared `{PRODUCT_SURFACE_TRAIT}` methods are missing: {removed:?}. Product \
         consumers share exactly `invoke`, `query`, and `stream_events` unless §5.2 is revised."
    );
}

#[test]
fn product_local_product_surface_traits_stay_retired() {
    let source_path = workspace_root().join(PRODUCT_REBORN_SERVICES_SOURCE);
    let source = std::fs::read_to_string(&source_path)
        .unwrap_or_else(|e| panic!("failed to read product source {source_path:?}: {e}"));
    let stripped = strip_comments_and_strings(&source);
    let product_surface_needle = format!("pub trait {PRODUCT_SURFACE_TRAIT}");
    let retired_needle = format!("trait {RETIRED_PROTO_FACADE_TRAIT}");

    assert!(
        !stripped.contains(&product_surface_needle),
        "`ironclaw_product::{PRODUCT_SURFACE_TRAIT}` was retired; use \
         `ironclaw_host_api::{PRODUCT_SURFACE_TRAIT}` and descriptors instead."
    );
    assert!(
        !stripped.contains(&retired_needle),
        "`{RETIRED_PROTO_FACADE_TRAIT}` was retired into `{PRODUCT_SURFACE_TRAIT}`; do not \
         reintroduce the proto-facade split."
    );
}

/// Self-test: the extractor takes only trait-declaration-depth `fn`s, tolerates
/// `async`, multi-line signatures, default-method bodies (with their own nested
/// `fn`s and braces), and ignores `fn`-shaped text in comments and strings.
#[test]
fn extract_trait_methods_self_test() {
    let sample = r##"
        // fn commented_out_before -> ignored
        pub trait SampleFacade: Send + Sync {
            async fn create_thread(
                &self,
                caller: Caller,
            ) -> Result<Resp, Err>;

            fn sync_method(&self) -> u8;

            /// doc: fn doc_comment_decoy
            async fn with_default(&self, _r: Req) -> Result<(), Err> {
                fn nested_helper() -> u8 { 0 } // nested fn at depth 2 -> ignored
                let _ = "fn string_literal_decoy";
                if true { let _x = 1; }        // inner braces must not close the trait
                Ok(())
            }

            async fn last_method(&self) -> u8 { 7 }
        }

        // Anything after the trait block must be ignored:
        fn free_fn_after() {}
        impl SampleFacade for Thing {
            async fn create_thread(&self, _c: Caller) -> Result<Resp, Err> { fn inner() {} unimplemented!() }
        }
    "##;

    let methods = extract_trait_methods(sample, "SampleFacade");
    assert_eq!(
        methods,
        vec![
            "create_thread",
            "sync_method",
            "with_default",
            "last_method"
        ],
        "extractor must yield exactly the trait-declaration-depth methods, in source order — \
         skipping nested/default-body fns, impl-block fns, free fns, and comment/string decoys"
    );
}

/// Self-test: a missing / renamed trait yields no methods (so the main test's
/// non-empty assertion fires loudly rather than silently passing on a rename).
#[test]
fn extract_trait_methods_missing_trait_self_test() {
    let sample = "pub trait Other { fn a(&self); }";
    assert!(extract_trait_methods(sample, "ProductSurface").is_empty());
}

/// #6292 IronLoop/Gemini: the trait lookup must be a WORD-boundary match, not a
/// substring match — otherwise a rename that keeps the same method set (e.g.
/// `ProductSurfaceV2` or `ProductSurface_legacy`) would silently bind here
/// and defeat the freeze's rename guard. A prefixed `subtrait`-like token must
/// not bind either. Only the exact `trait ProductSurface` block is picked up.
#[test]
fn extract_trait_methods_rejects_renamed_or_prefixed_trait_self_test() {
    for renamed in [
        "pub trait ProductSurfaceV2 { fn a(&self); }",
        "pub trait ProductSurface_legacy { fn a(&self); }",
        "pub subtrait ProductSurface { fn a(&self); }",
    ] {
        assert!(
            extract_trait_methods(renamed, "ProductSurface").is_empty(),
            "must not bind to a renamed/prefixed trait: {renamed}"
        );
    }
    // The exact trait (with a supertrait bound / generics right after the name)
    // still binds.
    assert_eq!(
        extract_trait_methods(
            "trait ProductSurface: Send { fn a(&self); }",
            "ProductSurface"
        ),
        vec!["a".to_string()],
    );
    assert_eq!(
        extract_trait_methods(
            "pub trait ProductSurface { fn b(&self); }",
            "ProductSurface"
        ),
        vec!["b".to_string()],
    );
}
