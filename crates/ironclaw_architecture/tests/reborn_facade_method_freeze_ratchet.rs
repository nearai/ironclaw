//! Anti-slippage ratchet for the product-facade method surface (§5.2 / §5.2.5 /
//! §10 of `docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`).
//!
//! §5.2's target: the product surface is the *turn lifecycle* + two generic
//! conduits (`invoke` for commands, `query` for reads) that never grow. A feature
//! adds a **capability descriptor** and/or a **view descriptor**, never a facade
//! method. §5.2.5 step 1 is "freeze the facade now" — check in the current
//! `RebornServicesApi` method set so any *new* method fails CI and the migration
//! stops the bleeding before it starts.
//!
//! This test **freezes the current 88-method `RebornServicesApi` trait block**
//! (`crates/ironclaw_product_workflow/src/reborn_services.rs`) as a set-membership
//! allowlist (§10: compare set membership, never a count) and fails on any change:
//!
//! - a **new** trait method (not in [`FROZEN_REBORN_SERVICES_METHODS`]) fails —
//!   the feature belongs in a capability/view descriptor, not a new facade method;
//! - a **removed** method not trimmed from the allowlist fails — so the list
//!   shrinks in lock-step as each mutation migrates to a descriptor and reviewers
//!   watch it get shorter (the §10 monotonic-shrink contract);
//! - a **duplicate** method name in the block fails (defensive; a trait cannot
//!   legally declare two, but the scan is explicit about it).
//!
//! Scoped to the *method set*: the extractor reads only the `RebornServicesApi`
//! trait block, at trait-declaration depth (a `fn` inside a default-method body is
//! ignored), with comments and string literals stripped (shared
//! [`ratchet_support::strip_comments_and_strings`]).
//!
//! Definition of done for this axis (§5.2.5 step 5 / §10): the facade *is* the
//! turn lifecycle (`open_conversation`, `submit_turn`, `events`, `reply`,
//! `resolve_gate`, `cancel`) + `invoke` + `query` — the allowlist shrinks to that
//! ~8-method set and every other entry migrates to a matrix-declared capability
//! descriptor or a view descriptor. The follow-on §10 obligation ("every
//! capability descriptor declares an origin→gate matrix") lands with the
//! descriptor registry and is a separate ratchet — this one holds the method
//! surface from growing while that migration runs.

// This ratchet uses only the shared stripper + workspace-root helper; the
// type-def scanners in the shared module are unreachable from this binary
// (each test binary compiles the whole module and uses a different subset).
#[allow(dead_code)]
mod ratchet_support;

use std::collections::BTreeSet;

use ratchet_support::{strip_comments_and_strings, workspace_root};

/// Path (relative to the workspace root) of the crate that defines the facade
/// trait — the §-referenced contract owner (`type-placement.md` rule 3).
const FACADE_SOURCE: &str = "crates/ironclaw_product_workflow/src/reborn_services.rs";
const FACADE_TRAIT: &str = "RebornServicesApi";

/// The frozen inventory of `RebornServicesApi` methods, as of the §5.2.5 freeze.
/// Grouped by the product domain each method serves, so a reviewer can see which
/// cluster is migrating as entries disappear. Remove an entry in the same PR that
/// deletes its method (because the method became a capability/view descriptor);
/// never add one — a new product operation is a descriptor, not a facade method.
const FROZEN_REBORN_SERVICES_METHODS: &[&str] = &[
    // --- turn lifecycle (the irreducible core, §5.2.3) ---
    "create_thread",
    "submit_turn",
    "delete_thread",
    "get_timeline",
    "global_auto_approve_enabled",
    "read_attachment",
    "stream_events",
    "supports_stream_events_subscription",
    "subscribe_events",
    "cancel_run",
    "resolve_gate",
    "retry_run",
    "get_run_state",
    // --- filesystem / project browsing (→ view descriptors, §5.2.2) ---
    "list_project_dir",
    "stat_project_path",
    "read_project_file",
    "list_fs_mounts",
    "browse_fs_dir",
    "stat_fs_path",
    "read_fs_file",
    // --- projects + membership (→ capability + view descriptors) ---
    "list_projects",
    "create_project",
    "get_project",
    "update_project",
    "delete_project",
    "list_project_members",
    "add_project_member",
    "update_project_member_role",
    "remove_project_member",
    "list_threads",
    // --- automations (→ Automation-origin descriptors, §5.2.1) ---
    "list_automations",
    "pause_automation",
    "resume_automation",
    "rename_automation",
    "delete_automation",
    // --- trace / credits ---
    "trace_credits",
    "trace_account_traces",
    "trace_account_login_link",
    "authorize_trace_hold",
    // --- outbound + connectable channels ---
    "list_connectable_channels",
    "get_outbound_preferences",
    "set_outbound_preferences",
    "list_outbound_delivery_targets",
    // --- extensions + skills ---
    "list_extensions",
    "list_skills",
    "search_skills",
    "install_skill",
    "read_skill_content",
    "update_skill",
    "remove_skill",
    "set_skill_auto_activate",
    "set_auto_activate_learned",
    "list_extension_registry",
    "install_extension",
    "import_extension",
    "activate_extension",
    "remove_extension",
    "setup_extension",
    // --- LLM admin config ---
    "get_llm_config",
    "upsert_llm_provider",
    "delete_llm_provider",
    "set_active_llm",
    "test_llm_connection",
    "list_llm_models",
    "start_nearai_login",
    "complete_nearai_wallet_login",
    "start_codex_login",
    // --- operator setup / config / diagnostics ---
    "get_operator_setup",
    "run_operator_setup",
    "list_operator_config",
    "get_operator_config_key",
    "set_operator_config_key",
    "validate_operator_config",
    "get_operator_diagnostics",
    "get_operator_status",
    "query_logs",
    "query_operator_logs",
    "run_operator_service_lifecycle",
    // --- admin users + per-user secrets ---
    "list_admin_users",
    "get_admin_user",
    "create_admin_user",
    "update_admin_user",
    "set_admin_user_status",
    "set_admin_user_role",
    "delete_admin_user",
    "list_admin_user_secrets",
    "put_admin_user_secret",
    "delete_admin_user_secret",
];

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
    // `trait RebornServicesApiV2`, `RebornServicesApi_legacy`, or a `subtrait`-
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
fn reborn_facade_method_allowlist_is_frozen_and_only_shrinks() {
    let source_path = workspace_root().join(FACADE_SOURCE);
    let source = std::fs::read_to_string(&source_path)
        .unwrap_or_else(|e| panic!("failed to read facade source {source_path:?}: {e}"));

    let found = extract_trait_methods(&source, FACADE_TRAIT);
    assert!(
        !found.is_empty(),
        "no `{FACADE_TRAIT}` methods were extracted from {FACADE_SOURCE}: the trait was renamed, \
         moved, or the extractor no longer recognizes its block — update this ratchet to keep \
         tracking the facade surface."
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
        "`{FACADE_TRAIT}` block yielded duplicate method names {duplicated:?} — the extractor or \
         the trait is malformed."
    );

    let frozen: BTreeSet<&str> = FROZEN_REBORN_SERVICES_METHODS.iter().copied().collect();
    let found_set: BTreeSet<&str> = found.iter().map(String::as_str).collect();

    let added: Vec<&str> = found_set.difference(&frozen).copied().collect();
    assert!(
        added.is_empty(),
        "New `{FACADE_TRAIT}` methods are banned (arch-simplification §5.2/§5.2.5/§10): the product \
         surface is turn-lifecycle + `invoke`/`query`; a new product operation is a matrix-declared \
         capability descriptor or a view descriptor, never a facade method. Offending new methods: \
         {added:?}."
    );

    let removed: Vec<&str> = frozen.difference(&found_set).copied().collect();
    assert!(
        removed.is_empty(),
        "FROZEN_REBORN_SERVICES_METHODS lists methods that no longer exist on `{FACADE_TRAIT}`: \
         {removed:?}. A facade method was removed (good — §5.2 migration progress!) — trim it from \
         the allowlist in the same PR so the ratchet keeps shrinking toward the turn-lifecycle + \
         `invoke`/`query` end-state (§10)."
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
    assert!(extract_trait_methods(sample, "RebornServicesApi").is_empty());
}

/// #6292 IronLoop/Gemini: the trait lookup must be a WORD-boundary match, not a
/// substring match — otherwise a rename that keeps the same method set (e.g.
/// `RebornServicesApiV2` or `RebornServicesApi_legacy`) would silently bind here
/// and defeat the freeze's rename guard. A prefixed `subtrait`-like token must
/// not bind either. Only the exact `trait RebornServicesApi` block is picked up.
#[test]
fn extract_trait_methods_rejects_renamed_or_prefixed_trait_self_test() {
    for renamed in [
        "pub trait RebornServicesApiV2 { fn a(&self); }",
        "pub trait RebornServicesApi_legacy { fn a(&self); }",
        "pub subtrait RebornServicesApi { fn a(&self); }",
    ] {
        assert!(
            extract_trait_methods(renamed, "RebornServicesApi").is_empty(),
            "must not bind to a renamed/prefixed trait: {renamed}"
        );
    }
    // The exact trait (with a supertrait bound / generics right after the name)
    // still binds.
    assert_eq!(
        extract_trait_methods(
            "trait RebornServicesApi: Send { fn a(&self); }",
            "RebornServicesApi"
        ),
        vec!["a".to_string()],
    );
    assert_eq!(
        extract_trait_methods(
            "pub trait RebornServicesApi { fn b(&self); }",
            "RebornServicesApi"
        ),
        vec!["b".to_string()],
    );
}
