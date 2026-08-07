//! Concrete-name specificity gate for the unified extension runtime.
// arch-exempt: large_file, unified extension specificity ratchet stays centralized, plan #6175
//!
//! Goal (docs/reborn/extension-runtime/overview.md §1): no generic crate
//! contains a concrete product name, vendor id, or vendor API host. The
//! forbidden vocabulary is **derived from the bundled package inventory**
//! (`crates/extensions/packages/` plus the test fixture
//! inventory `tests/fixtures/extensions/`), so a future `discord` package is
//! caught without editing this scanner (checklist TEST-6).
//!
//! Scope: the `src/` tree and `Cargo.toml` of every generic Reborn workspace
//! crate, plus the WebUI frontend sources. Excluded, by category:
//!
//! - **Concrete extension crates** (`CONCRETE_EXTENSION_CRATES`) — they *are*
//!   the product code.
//! - **The package inventory crate** (`ironclaw_extension_support`) —
//!   it owns the concrete packages and their native executors.
//! - **Sanctioned assemblers** — `ironclaw` (the binary assembles
//!   the native factory registry; overview §4.0) and this architecture crate
//!   (names terms on purpose).
//! - **Legacy-layer / v1 crates** — the v1 enclave is being strangled
//!   wholesale, not policed term-by-term (same footing as
//!   `reborn_retired_taxonomy.rs`).
//! - **Test code** — tests may name concrete products (overview §8); `tests/`
//!   directories, `tests.rs` / `*_tests.rs` files, and `#[cfg(test)]` blocks
//!   inside `src/` are stripped before matching.
//!
//! Term-collision carve-outs (`TERM_COLLISIONS`, `PATH_TERM_COLLISIONS`):
//! some inventory-derived identifiers are *also* vocabulary of a different
//! product domain that legitimately and permanently lives in generic crates.
//! Exactly four such domains exist: LLM providers (`nearai`/`api.near.ai` is
//! the assistant's LLM+embeddings backend; Google is Gemini; GitHub is
//! Copilot), WebUI browser-login SSO providers (Google/GitHub OIDC), GitHub
//! as a skill/code host (skill installation from repositories), and
//! vendor-specific safety detection — leak/secret scanners must know
//! Slack/GitHub token shapes, and the trace payload-redaction/side-effect
//! classifier must know which tool-name keywords carry messaging/email/
//! issue-tracker payloads (a safety denylist that is a *superset* of the
//! inventory, so it cannot be sourced from the inventory without weakening
//! redaction). Bare `nearai` terms are carved out globally with the
//! compound extension forms (`nearai_mcp`, `private.near.ai`, …) still
//! scanned; the rest are path-scoped. Each carve-out names its collision and
//! fails when stale; adding one for any other reason is a violation of this
//! gate's purpose.
//!
//! Allowlist discipline (checklist TEST-7): `ALLOWLIST` enumerates today's
//! violations as exact `(path, term)` pairs. A new violating pair fails; a
//! stale pair (the file no longer matches the term) also fails, so the list
//! can only shrink. It must be **empty** by P7 (checklist DEL-8).
//!
//! Every remaining entry is **lane-4 residue** (grouped below by category with
//! a `// lane-4:` marker): a genuine generic branch, the deferred `nearai_mcp`
//! catalog slice, a one-time migration call site, the web-access assembly
//! module, an incidental doc/tool-string example, or the sanctioned DEL-7
//! dev-dependency. None is a first-party package-catalog name — Lane A cleared
//! those. Each is characterized in the PR #6065 lane-4 inventory and is the
//! owner's next decision: do NOT casually "fix" one — degenericize by routing
//! on a manifest capability, or carve deliberately with a one-line justification.

#[allow(dead_code)]
mod ratchet_support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

// Crate paths are spelled flat (`crates/ironclaw_x/...`) and RESOLVED through
// the crate inventory, so the family move (PROPOSAL section 5) repoints the
// ~215 literals below without a lockstep edit. On today's tree resolution is
// the identity - pinned by `reborn_crate_inventory.rs`.
use ratchet_support::{
    crate_dir, crate_directories, crate_path, strip_line_anchored_cfg_test_items,
    try_resolve_crate_relative, workspace_root,
};

/// Resolve a listed path through the crate inventory, falling back to the
/// literal when it names no crate that exists.
///
/// A crate that MOVED resolves, which is the point: the ~215 entries below keep
/// their readable flat spelling across the family move (PROPOSAL section 5). A
/// crate that was DELETED or RENAMED does not resolve, and keeping its literal
/// spelling makes it match nothing - which each list's own stale-entry
/// detection already reports, with a better message than a panic and with
/// exactly the behavior this gate had before.
fn resolve_listed_path(root: &Path, logical: &str) -> String {
    try_resolve_crate_relative(root, logical).unwrap_or_else(|_| logical.to_string())
}

// ---------------------------------------------------------------------------
// Inventory-derived forbidden terms
// ---------------------------------------------------------------------------

/// Package directories under `packages/` whose ids are NOT vendor vocabulary.
///
/// WS2 moved the two `[memory]` provider packages into `extensions/packages/`
/// beside the extension packages. Deriving the forbidden vocabulary from their
/// manifests as well would be wrong on both halves:
///
/// * `ironclaw.memory` and the `ironclaw.memory.*` tool ids are the
///   *provider-neutral* memory contract — the ids stay identical whichever
///   provider is bound, which is the whole point of
///   `MEMORY_PROVIDER_PACKAGE_IDS`. The kernel and the contracts crates name
///   them legitimately, so scanning them would flag the contract as vendor
///   specificity.
/// * `mem0` genuinely *is* a vendor name, and generic crates naming it
///   genuinely is debt — but that is the rule PROPOSAL §8.2 states separately
///   ("no crate outside the provider packages and the binary names a memory
///   provider"), enforced by
///   `reborn_dependency_boundaries.rs::only_the_sanctioned_residue_names_a_memory_provider`
///   with its own named, shrink-only residue. Folding it in here instead would
///   have meant raising this gate's shrink-only allowlist baseline by ~40
///   entries for debt that predates the move.
///
/// Excluding these two restores exactly the pre-move term set: before WS2 the
/// inventory was the twelve extension packages plus the fixtures, and it still
/// is.
const NON_VENDOR_PROVIDER_PACKAGE_DIRS: &[&str] = &["memory-native", "mem0"];

/// Directories whose `*/manifest.toml` files form the package inventory the
/// forbidden vocabulary derives from.
fn inventory_dirs(root: &Path) -> Vec<PathBuf> {
    vec![packages_root(root), root.join("tests/fixtures/extensions")]
}

/// The shipped package inventory, anchored on the crate that owns it rather
/// than on the literal `crates/extensions/packages`. A package directory is
/// owned by no crate (PROPOSAL section 5), so it cannot be resolved by name -
/// but its owning support crate can be, and `packages/` is that crate's
/// sibling. This is the hop `scripts/build-wasm-extensions.sh` and
/// `scripts/ci/ws12_workflow_contracts.py` already use.
fn packages_root(root: &Path) -> PathBuf {
    let support = crate_dir(root, "ironclaw_extension_support");
    let packages = support
        .parent()
        .expect("the package-support crate must have a parent directory")
        .join("packages");
    assert!(
        packages.is_dir(),
        "package inventory root {} does not exist - the specificity vocabulary would derive \
         from nothing. `packages/` is resolved as a sibling of the ironclaw_extension_support \
         crate; if the packages moved elsewhere, repoint this hop.",
        packages.display()
    );
    packages
}

/// Bare terms carved out of the derived set because they collide with a
/// non-extension taxonomy that legitimately lives in generic crates. Compound
/// forms derived from the same package (directory-name variants, specific
/// hosts) remain scanned.
const TERM_COLLISIONS: &[(&str, &str)] = &[
    (
        "nearai",
        "`nearai` is also the assistant's LLM backend id (crates/ironclaw_llm, \
         crates/ironclaw_config); the extension forms `nearai-mcp`/`nearai_mcp` \
         and the MCP host stay scanned",
    ),
    (
        "near.ai",
        "`api.near.ai` is the LLM/embeddings API host; the hosted-MCP endpoint \
         `private.near.ai` stays scanned",
    ),
];

/// Path-scoped permanent carve-outs for identifiers that belong to a
/// **different product domain** which legitimately owns the vocabulary.
/// Four classes only — LLM providers, WebUI browser-login SSO providers,
/// GitHub as a skill/code host, and vendor-specific safety detection
/// (credential-format leak/secret scanners plus the trace payload-redaction
/// classifier).
/// Every entry must keep matching (staleness fails) and the exact list is
/// pinned by a self-test; broadening it is a gate regression, not a fix.
const PATH_TERM_COLLISIONS: &[(&str, &str, &str)] = &[
    (
        "crates/ironclaw_host_api/src/credential_redaction.rs",
        "github",
        "GitHub token prefixes (ghp_/github_pat_/gho_/ghu_) in credential \
         redaction — vendor-specific safety detection, the documented \
         leak-scanner carve-out domain (sourcing from the inventory would weaken \
         the scan)",
    ),
    (
        "crates/ironclaw_llm/src/",
        "google",
        "Google is an LLM vendor (Gemini endpoints, google/gemma model ids) inside the \
         multi-provider LLM crate, independent of the google extensions vendor",
    ),
    (
        "crates/ironclaw_llm/src/",
        "github",
        "GitHub Copilot is an LLM provider (github_copilot*.rs), independent of the github \
         extension",
    ),
    (
        "crates/ironclaw_llm/src/",
        "private.near.ai",
        "the NEAR AI LLM API base URL (session/config defaults), independent of the \
         nearai-mcp extension endpoint",
    ),
    (
        "crates/ironclaw_llm/src/",
        "accounts.google.com",
        "Gemini OAuth endpoints inside the multi-provider LLM crate",
    ),
    (
        "crates/ironclaw_llm/src/",
        "oauth2.googleapis.com",
        "Gemini OAuth endpoints inside the multi-provider LLM crate",
    ),
    (
        "crates/ironclaw_webui/src/auth/",
        "google",
        "WebUI browser-login SSO provider (OIDC), not the extensions vendor",
    ),
    (
        "crates/ironclaw_webui/src/auth/",
        "github",
        "WebUI browser-login SSO provider, not the extensions vendor",
    ),
    (
        "crates/ironclaw_webui/src/lib.rs",
        "google",
        "re-export of the SSO login providers",
    ),
    (
        "crates/ironclaw_webui/src/lib.rs",
        "github",
        "re-export of the SSO login providers",
    ),
    (
        "crates/ironclaw_webui/src/auth/",
        "accounts.google.com",
        "WebUI browser-login SSO (OIDC) endpoints, not the extensions vendor",
    ),
    (
        "crates/ironclaw_webui/src/auth/",
        "oauth2.googleapis.com",
        "WebUI browser-login SSO (OIDC) endpoints, not the extensions vendor",
    ),
    // WS3 removed two carve-outs that used to live here — the skill-URL
    // installer's `github` and `api.github.com` entries. The installer moved to
    // `ironclaw_extension_support::skills::url_install`, and that crate is in
    // `SANCTIONED_SCAN_EXEMPT_CRATES` (it is the sanctioned vendor-name home),
    // so the scanner no longer reaches those files at all. Its two siblings
    // survive further down because their subjects stayed in `host_runtime`:
    // `first_party_tools/skill_management.rs` (the tool manifest description)
    // and `first_party_tools/schemas.rs` (the input schema) are host-owned
    // capability *declarations*, not the executor.
    (
        "crates/ironclaw_sandbox/src/sandbox_process/network_allowlist.rs",
        "github",
        "default sandboxed-shell egress allowlist includes github.com/\
         raw.githubusercontent.com/codeload.github.com for ordinary `git clone`/release-\
         archive workflows — GitHub as a code host, not the github extension",
    ),
    (
        "crates/ironclaw_sandbox/src/sandbox_process/network_allowlist.rs",
        "api.github.com",
        "default sandboxed-shell egress allowlist includes GitHub's content API host for \
         `gh`/archive-download workflows — GitHub as a code host, not the github extension",
    ),
    (
        "crates/ironclaw_host_runtime/src/first_party_tools/skill_management.rs",
        "github",
        "skill-install tool description names GitHub as a skill source",
    ),
    (
        "crates/ironclaw_extension_manager/src/ironhub/artifact_hosts.rs",
        "github",
        "IronHub's signed-package downloader pins GitHub release infrastructure as a code \
         artifact source, independent of the github extension",
    ),
    (
        "crates/ironclaw_host_runtime/src/first_party_tools/schemas.rs",
        "github",
        "skill-install input schema names GitHub as a skill source",
    ),
    (
        "crates/ironclaw_safety/src/leak_detector.rs",
        "github",
        "vendor token-format leak detection",
    ),
    (
        "crates/ironclaw_safety/src/leak_detector.rs",
        "google",
        "vendor token-format leak detection",
    ),
    (
        "crates/ironclaw_safety/src/leak_detector.rs",
        "slack",
        "vendor token-format leak detection",
    ),
    (
        "crates/ironclaw_config/src/secrets_guard.rs",
        "github",
        "vendor token-format inline-secret rejection",
    ),
    (
        "crates/ironclaw_config/src/secrets_guard.rs",
        "google",
        "vendor token-format inline-secret rejection",
    ),
    (
        "crates/ironclaw_config/src/secrets_guard.rs",
        "slack",
        "vendor token-format inline-secret rejection",
    ),
    (
        "crates/ironclaw_event_log/src/runtime_event.rs",
        "github",
        "credential-prefix redaction (github_pat_)",
    ),
    (
        "crates/ironclaw_loop_host/src/capability_port.rs",
        "github",
        "credential-prefix redaction (github_pat_)",
    ),
    (
        "crates/ironclaw_webui/frontend/src/pages/chat/lib/failureMessages.ts",
        "github",
        "credential-prefix redaction (github_pat_)",
    ),
    (
        "crates/ironclaw_loop_contracts/src/host/validate.rs",
        "github",
        "credential-prefix redaction (github_pat_) — relocated here when \
         run_profile/host.rs was decomposed (#6391), and again when WS1.2 moved \
         run_profile/** into ironclaw_loop_contracts",
    ),
    (
        "crates/ironclaw_loop_contracts/src/host/validate.rs",
        "google",
        "credential-prefix redaction (Google/GCP key shapes) at the \
         model-visible boundary — vendor-specific safety detection",
    ),
    (
        "crates/ironclaw_loop_host/src/model_visible_scrub.rs",
        "github",
        "model-visible Diagnostic scrub knows GitHub token prefixes \
         (ghp_/gho_/github_pat_) — the leak-scanner carve-out domain (#5965)",
    ),
    (
        "crates/ironclaw_loop_host/src/model_visible_scrub.rs",
        "google",
        "model-visible Diagnostic scrub knows Google/GCP credential shapes — \
         the leak-scanner carve-out domain (#5965)",
    ),
    (
        "crates/ironclaw_loop_host/src/model_visible_scrub.rs",
        "slack",
        "model-visible Diagnostic scrub knows Slack token shapes (xox…) — \
         the leak-scanner carve-out domain (#5965)",
    ),
    (
        "crates/ironclaw_loop_contracts/src/prompt_text.rs",
        "github",
        "credential-prefix redaction (github_pat_)",
    ),
    (
        "crates/ironclaw_auth/src/lib.rs",
        "gmail",
        "auth-engine OAuth provider-id vocabulary (persisted provider ids), not the extensions vendor",
    ),
    (
        "crates/ironclaw_auth/src/lib.rs",
        "google",
        "auth-engine OAuth provider-id vocabulary (persisted provider ids), not the extensions vendor",
    ),
    (
        "crates/ironclaw_auth/src/lib.rs",
        "google_calendar",
        "auth-engine OAuth provider-id vocabulary (persisted provider ids), not the extensions vendor",
    ),
    (
        "crates/ironclaw_auth/src/oauth.rs",
        "gmail",
        "auth-engine OAuth provider-id vocabulary (persisted provider ids), not the extensions vendor",
    ),
    (
        "crates/ironclaw_auth/src/oauth.rs",
        "google",
        "auth-engine OAuth provider-id vocabulary (persisted provider ids), not the extensions vendor",
    ),
    (
        "crates/ironclaw_auth/src/oauth.rs",
        "google_calendar",
        "auth-engine OAuth provider-id vocabulary (persisted provider ids), not the extensions vendor",
    ),
    (
        "crates/ironclaw_auth/src/oauth.rs",
        "slack",
        "auth-engine OAuth provider-id vocabulary (persisted provider ids), not the extensions vendor",
    ),
    (
        "crates/ironclaw_auth/src/oauth.rs",
        "www.googleapis.com",
        "auth-engine OAuth provider-id vocabulary (persisted provider ids), not the extensions vendor",
    ),
    (
        "crates/ironclaw_auth/src/scope.rs",
        "google",
        "auth-engine OAuth provider-id vocabulary (persisted provider ids), not the extensions vendor",
    ),
    (
        "crates/ironclaw_common/src/identity.rs",
        "github",
        "persisted credential-name / channel-id vocabulary in the shared identity crate (compat law: stored ids stay readable)",
    ),
    (
        "crates/ironclaw_common/src/identity.rs",
        "gmail",
        "persisted credential-name / channel-id vocabulary in the shared identity crate (compat law: stored ids stay readable)",
    ),
    (
        "crates/ironclaw_common/src/identity.rs",
        "google",
        "persisted credential-name / channel-id vocabulary in the shared identity crate (compat law: stored ids stay readable)",
    ),
    (
        "crates/ironclaw_common/src/identity.rs",
        "google-calendar",
        "persisted credential-name / channel-id vocabulary in the shared identity crate (compat law: stored ids stay readable)",
    ),
    (
        "crates/ironclaw_common/src/identity.rs",
        "google_calendar",
        "persisted credential-name / channel-id vocabulary in the shared identity crate (compat law: stored ids stay readable)",
    ),
    (
        "crates/ironclaw_common/src/identity.rs",
        "notion",
        "persisted credential-name / channel-id vocabulary in the shared identity crate (compat law: stored ids stay readable)",
    ),
    (
        "crates/ironclaw_common/src/identity.rs",
        "slack",
        "persisted credential-name / channel-id vocabulary in the shared identity crate (compat law: stored ids stay readable)",
    ),
    (
        "crates/ironclaw_llm/src/gemini_oauth.rs",
        "www.googleapis.com",
        "Gemini OAuth scope host in the multi-provider LLM crate",
    ),
    (
        "crates/ironclaw_identity/src/identity_store.rs",
        "slack",
        "credential-authority ProviderKind vocabulary (persisted identity keys), not the extensions vendor",
    ),
    (
        "crates/ironclaw_identity/src/key.rs",
        "github",
        "credential-authority ProviderKind vocabulary (persisted identity keys), not the extensions vendor",
    ),
    (
        "crates/ironclaw_identity/src/key.rs",
        "google",
        "credential-authority ProviderKind vocabulary (persisted identity keys), not the extensions vendor",
    ),
    (
        "crates/ironclaw_identity/src/key.rs",
        "slack",
        "credential-authority ProviderKind vocabulary (persisted identity keys), not the extensions vendor",
    ),
    (
        "crates/ironclaw_identity/src/lib.rs",
        "github",
        "credential-authority ProviderKind vocabulary (persisted identity keys), not the extensions vendor",
    ),
    (
        "crates/ironclaw_identity/src/lib.rs",
        "google",
        "credential-authority ProviderKind vocabulary (persisted identity keys), not the extensions vendor",
    ),
    (
        "crates/ironclaw_identity/src/lib.rs",
        "slack",
        "credential-authority ProviderKind vocabulary (persisted identity keys), not the extensions vendor",
    ),
    (
        "crates/ironclaw_common/src/attachment.rs",
        "telegram",
        "persisted credential-name / channel-id vocabulary in the shared identity crate (compat law: stored ids stay readable)",
    ),
    (
        "crates/ironclaw_common/src/identity.rs",
        "telegram",
        "persisted credential-name / channel-id vocabulary in the shared identity crate (compat law: stored ids stay readable)",
    ),
    (
        "crates/ironclaw_identity/src/key.rs",
        "telegram",
        "credential-authority ProviderKind vocabulary (persisted identity keys), not the extensions vendor",
    ),
    (
        "crates/ironclaw_identity/src/lib.rs",
        "telegram",
        "credential-authority ProviderKind vocabulary (persisted identity keys), not the extensions vendor",
    ),
    (
        "crates/ironclaw_safety/src/leak_detector.rs",
        "telegram",
        "vendor token-format leak detection (telegram_bot_token pattern)",
    ),
    // Trace payload-redaction / side-effect safety classifier
    // (`ironclaw_trace_commons`): `tool_payload_profile` selects the
    // payload-redaction profile and `classify_tool_side_effect` the
    // external-write side-effect off tool-name keywords. The vendor keywords
    // are a safety DENYLIST — a superset of the bundled inventory that must
    // also cover non-package messaging/issue-tracker tools (signal, discord,
    // gitlab) — so sourcing the set from the inventory would drop those and
    // weaken redaction. Not extension routing (the classifier has only the
    // tool-name string at trace-analysis time). Pinned by
    // `tool_payload_redaction_profile_is_a_safety_denylist_not_inventory_routing`.
    //
    // Scoped to two files since the `contribution.rs` split (WS6): the rule
    // tables live in `tool_payloads.rs` and only the external-write detector
    // (`classify_tool_side_effect`, `slack` alone) is in `classification.rs`.
    // Both carve-outs are narrower than the single whole-file one they
    // replace, so the gate now polices the rest of the module.
    (
        "crates/ironclaw_trace_commons/src/contribution/tool_payloads.rs",
        "slack",
        "trace payload-redaction safety classifier keyed off tool-name keywords \
         (messaging profile); a safety denylist, not extension routing",
    ),
    (
        "crates/ironclaw_trace_commons/src/contribution/classification.rs",
        "slack",
        "trace external-write side-effect detection keyed off tool-name keywords \
         (`classify_tool_side_effect`); a safety denylist, not extension routing",
    ),
    (
        "crates/ironclaw_trace_commons/src/contribution/tool_payloads.rs",
        "telegram",
        "trace payload-redaction safety classifier keyed off tool-name keywords \
         (messaging profile); a safety denylist, not extension routing",
    ),
    (
        "crates/ironclaw_trace_commons/src/contribution/tool_payloads.rs",
        "gmail",
        "trace payload-redaction safety classifier keyed off tool-name keywords \
         (email profile); a safety denylist, not extension routing",
    ),
    (
        "crates/ironclaw_trace_commons/src/contribution/tool_payloads.rs",
        "github",
        "trace payload-redaction safety classifier keyed off tool-name keywords \
         (issue-tracker profile); a safety denylist, not extension routing",
    ),
    // Manifest inline-secret guard (`looks_like_inline_secret`,
    // `ironclaw_extension_registry::host_api::product_adapter`): the vendor
    // token *prefixes* a manifest field value is rejected for carrying.
    //
    // **Carved, not allowlisted, and CHECKLIST WS5's "slack/telegram token
    // heuristics → the packages that own them" clause is REFUTED by the same
    // measurement.** Two independent reasons, both mechanical:
    //
    // 1. It is a **superset of the bundled inventory**, exactly like the
    //    trace-redaction classifier above. Of the eleven prefixes it carries,
    //    five belong to no installed package at all — `sk-` (OpenAI/Anthropic
    //    API keys), AWS `AKIA`/`ASIA`, JWTs (`eyJ…`), PEM private-key headers,
    //    and URI userinfo — so sourcing the set from the packages would
    //    *shrink* it and stop rejecting inline secrets the guard catches today.
    //    A guard that only knows the vendors currently installed is a guard
    //    that fails open on the next one.
    // 2. The move is **layer-illegal in the named direction**.
    //    `ironclaw_extension_registry` is `layer = "substrates"` and
    //    `crates/extensions/packages/{slack,telegram}` are `layer = "products"`,
    //    so `registry → package` is upward and needs a
    //    `LAYER_MATRIX_EXCEPTION` the ratchet forbids. The only legal shape is
    //    inversion — a package registers its prefixes with the parser — and
    //    that is strictly worse here: the guard runs while parsing an
    //    *arbitrary* manifest, including before any package is loaded, so a
    //    registration-sourced list is unpopulated exactly when it matters.
    //
    // Pinned by `inline_secret_guard_is_a_safety_denylist_not_package_inventory`
    // in `crates/extensions/ironclaw_extension_registry/tests/product_adapter_manifest_ingestion.rs`.
    (
        "crates/ironclaw_extension_registry/src/host_api/product_adapter.rs",
        "github",
        "manifest inline-secret guard token prefixes (`ghp_`/`gho_`/…); a safety \
         denylist that is a superset of the package inventory, not extension routing",
    ),
    (
        "crates/ironclaw_extension_registry/src/host_api/product_adapter.rs",
        "slack",
        "manifest inline-secret guard token prefixes (`xoxb-`/`xoxp-`/…); a safety \
         denylist that is a superset of the package inventory, not extension routing",
    ),
    (
        "crates/ironclaw_extension_registry/src/host_api/product_adapter.rs",
        "telegram",
        "manifest inline-secret guard bot-token shape (`looks_like_telegram_token`); a \
         safety denylist that is a superset of the package inventory, not extension routing",
    ),
    (
        "crates/ironclaw_webui/frontend/src/i18n/",
        "google",
        "Google named only as a NEAR AI browser-login SSO provider in localized \
         onboarding copy (onboarding.nearaiLocalSso: \"GitHub, Google, NEAR Wallet\"), \
         not the extensions vendor — localized UI copy must name the login providers",
    ),
    (
        "crates/ironclaw_webui/frontend/src/i18n/",
        "github",
        "GitHub named only in localized UI copy (all 11 locales), never the github \
         extension: the NEAR AI browser-login SSO provider name (onboarding.nearaiLocalSso, \
         the same key `google` is carved under), GitHub as a skill-install source \
         (tools.description.builtin.skill_install), and the user-facing display of the \
         HTTP-tool capability hint (tools.description.builtin.http/.http.save) — the \
         model-facing source of that hint stays tracked as lane-4 debt at \
         host_runtime/first_party_tools/http.rs; localized user copy must name the \
         provider/source it refers to",
    ),
    (
        "crates/ironclaw_webui/frontend/src/pages/login/components/oauth-provider-buttons.tsx",
        "github",
        "WebUI browser-login SSO provider button/route, not the extensions vendor",
    ),
    (
        "crates/ironclaw_webui/frontend/src/pages/login/components/oauth-provider-buttons.tsx",
        "google",
        "WebUI browser-login SSO provider button/route, not the extensions vendor",
    ),
    (
        "crates/ironclaw_webui/frontend/src/pages/login/hooks/useOAuthProviders.ts",
        "github",
        "WebUI browser-login SSO provider button/route, not the extensions vendor",
    ),
    (
        "crates/ironclaw_webui/frontend/src/pages/login/hooks/useOAuthProviders.ts",
        "google",
        "WebUI browser-login SSO provider button/route, not the extensions vendor",
    ),
    (
        "crates/ironclaw_webui/frontend/src/pages/onboarding/onboarding-page.tsx",
        "github",
        "WebUI browser-login SSO provider button/route, not the extensions vendor",
    ),
    (
        "crates/ironclaw_webui/frontend/src/pages/onboarding/onboarding-page.tsx",
        "google",
        "WebUI browser-login SSO provider button/route, not the extensions vendor",
    ),
    (
        "crates/ironclaw_webui/frontend/src/pages/settings/components/provider-card.tsx",
        "github",
        "WebUI browser-login SSO provider button/route, not the extensions vendor",
    ),
    (
        "crates/ironclaw_webui/frontend/src/pages/settings/components/provider-card.tsx",
        "google",
        "WebUI browser-login SSO provider button/route, not the extensions vendor",
    ),
    (
        "crates/ironclaw_webui/frontend/src/pages/settings/hooks/useProviderLogin.ts",
        "github",
        "WebUI browser-login SSO provider button/route, not the extensions vendor",
    ),
    (
        "crates/ironclaw_webui/frontend/src/pages/settings/hooks/useProviderLogin.ts",
        "google",
        "WebUI browser-login SSO provider button/route, not the extensions vendor",
    ),
    (
        "crates/ironclaw_webui/frontend/src/lib/browser-origin.ts",
        "github",
        "WebUI browser-login SSO provider origin validation, not the extensions vendor",
    ),
    (
        "crates/ironclaw_webui/frontend/src/lib/browser-origin.ts",
        "google",
        "WebUI browser-login SSO provider origin validation, not the extensions vendor",
    ),
    (
        "crates/ironclaw_webui/frontend/src/lib/browser-origin.ts",
        "private.near.ai",
        "WebUI browser-login SSO provider origin validation, not the extensions vendor",
    ),
];

/// Path fragments allowed to reference concrete extension identities for a
/// structural reason, mirroring `reborn_retired_taxonomy.rs`: the one-time
/// forward data migrations name what they fold forward.
const SANCTIONED_PATHS: &[&str] = &[
    "extension_host/extension_installation_store.rs",
    // One-release legacy webhook-path aliases (MIG-5): the compatibility
    // table names the concrete legacy paths it forwards; each entry carries
    // its own removal note.
    "product_auth/durable/",
];

/// Derive the forbidden term set from every `manifest.toml` under the given
/// inventory directories. Terms are lowercase; multi-part identifiers add
/// `-`, `_`, and compact variants so camel-case compounds are caught.
fn derive_forbidden_terms(inventory: &[PathBuf]) -> BTreeSet<String> {
    let mut terms = BTreeSet::new();
    for dir in inventory {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let package_dir = entry.path();
            if !package_dir.is_dir() {
                continue;
            }
            if NON_VENDOR_PROVIDER_PACKAGE_DIRS
                .iter()
                .any(|name| package_dir.file_name().is_some_and(|dir| dir == *name))
            {
                continue;
            }
            let manifest_path = package_dir.join("manifest.toml");
            let Ok(contents) = std::fs::read_to_string(&manifest_path) else {
                continue;
            };
            let manifest = contents.parse::<toml::Table>().unwrap_or_else(|error| {
                panic!(
                    "unparseable inventory manifest {} — fix the package before the scanner \
                     can derive from it: {error}",
                    manifest_path.display()
                )
            });
            let manifest = toml::Value::Table(manifest);
            let dir_name = entry.file_name().to_string_lossy().to_string();
            add_identifier_variants(&mut terms, &dir_name);
            collect_manifest_terms(&manifest, &mut terms);
        }
    }
    for (term, _reason) in TERM_COLLISIONS {
        terms.remove(*term);
    }
    terms.retain(|term| term.len() >= 4);
    terms
}

/// Walk a manifest value tree collecting extension ids, vendor ids, and
/// vendor API hosts. Key-path driven so it covers both the v2 and v3 schema
/// shapes without enumerating either:
///
/// - top-level `id` — the extension id;
/// - top-level `[auth.<vendor>]` table keys — v3 vendor ids;
/// - `provider` (v2) / `vendor` (v3) string values at any depth;
/// - `host` / `host_pattern` values and the hosts of URL-valued declarations
///   (`url`, `server`, recipe endpoints) at any depth.
fn collect_manifest_terms(value: &toml::Value, terms: &mut BTreeSet<String>) {
    collect_terms_inner(value, &mut Vec::new(), terms);
}

fn collect_terms_inner<'v>(
    value: &'v toml::Value,
    path: &mut Vec<&'v str>,
    terms: &mut BTreeSet<String>,
) {
    match value {
        toml::Value::Table(table) => {
            for (child_key, child) in table {
                if path.as_slice() == ["auth"] {
                    add_identifier_variants(terms, child_key);
                }
                path.push(child_key.as_str());
                collect_terms_inner(child, path, terms);
                path.pop();
            }
        }
        toml::Value::Array(items) => {
            for item in items {
                collect_terms_inner(item, path, terms);
            }
        }
        toml::Value::String(text) => match path.last().copied() {
            Some("id") if path.len() == 1 => {
                add_identifier_variants(terms, text);
            }
            Some("provider") | Some("vendor") => {
                add_identifier_variants(terms, text);
            }
            Some("host_pattern") | Some("host") => {
                add_host_term(terms, text);
            }
            Some("url")
            | Some("server")
            | Some("authorization_endpoint")
            | Some("token_endpoint")
            | Some("endpoint") => {
                if let Some(host) = url_host(text) {
                    add_host_term(terms, &host);
                }
            }
            _ => {}
        },
        _ => {}
    }
}

fn add_identifier_variants(terms: &mut BTreeSet<String>, identifier: &str) {
    let lower = identifier.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return;
    }
    terms.insert(lower.clone());
    if lower.contains('-') || lower.contains('_') {
        terms.insert(lower.replace('-', "_"));
        terms.insert(lower.replace('_', "-"));
        terms.insert(lower.replace(['-', '_'], ""));
    }
}

fn add_host_term(terms: &mut BTreeSet<String>, host: &str) {
    let host = host.trim().trim_start_matches("*.").to_ascii_lowercase();
    if host.is_empty() {
        return;
    }
    terms.insert(host);
}

fn url_host(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1)?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.split('@').next_back()?;
    let host = host.split(':').next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

// ---------------------------------------------------------------------------
// Scan scope
// ---------------------------------------------------------------------------

/// Reborn layers whose crates are "generic" for this gate. Legacy-layer and
/// unlayered (non-workspace) crates are the v1 enclave.
const REBORN_LAYERS: &[&str] = &[
    "contracts",
    "substrates",
    "runtimes",
    "kernel",
    "loops",
    "products",
    "app",
];

/// Crates that are the concrete product code (present or planned). A missing
/// directory is tolerated so the planned extension crates are covered from
/// the day they appear.
const CONCRETE_EXTENSION_CRATES: &[&str] =
    &["ironclaw_slack_extension", "ironclaw_telegram_extension"];

/// Generic-side crates excluded from the scan for a documented structural
/// reason (see the module header).
const SANCTIONED_SCAN_EXEMPT_CRATES: &[&str] = &[
    // The package inventory + native executors for bundled extensions.
    "ironclaw_extension_support",
    // The binary assembles the native factory registry (overview §4.0).
    "ironclaw",
    // The post-Tier-B workspace root: a test-only host for the Reborn
    // integration [[test]] suite (no lib/bin). Tests may name concrete
    // products (module header), and its manifest's workspace `exclude`
    // list names the WASM source dirs by path — assembly, not generic code.
    "ironclaw_integration_tests",
    // One-time forward migrations name what they fold forward.
    // This crate's tests name every term on purpose.
    "ironclaw_architecture_tests",
    // Load/stress tooling, not product code.
    "ironclaw_stress",
];

struct ScanRoot {
    crate_name: String,
    crate_dir: PathBuf,
}

fn generic_scan_roots(metadata: &Value, root: &Path) -> Vec<ScanRoot> {
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");
    let mut roots = Vec::new();
    for package in packages {
        let Some(name) = package["name"].as_str() else {
            continue;
        };
        if !(name == "ironclaw" || name.starts_with("ironclaw_")) {
            continue;
        }
        if CONCRETE_EXTENSION_CRATES.contains(&name)
            || SANCTIONED_SCAN_EXEMPT_CRATES.contains(&name)
        {
            continue;
        }
        let layer = package
            .get("metadata")
            .and_then(|metadata| metadata.get("ironclaw"))
            .and_then(|ironclaw| ironclaw.get("layer"))
            .and_then(|layer| layer.as_str());
        if !layer.is_some_and(|layer| REBORN_LAYERS.contains(&layer)) {
            continue;
        }
        let Some(manifest_path) = package["manifest_path"].as_str() else {
            continue;
        };
        let crate_dir = PathBuf::from(manifest_path)
            .parent()
            .expect("manifest has parent dir")
            .to_path_buf();
        // Only police crates inside this workspace checkout.
        if !crate_dir.starts_with(root) {
            continue;
        }
        roots.push(ScanRoot {
            crate_name: name.to_string(),
            crate_dir,
        });
    }
    roots.sort_by(|a, b| a.crate_name.cmp(&b.crate_name));
    roots
}

// ---------------------------------------------------------------------------
// File scanning
// ---------------------------------------------------------------------------

fn is_test_source_path(path: &Path) -> bool {
    let mut components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string());
    if components.any(|component| {
        component == "tests"
            || component == "__tests__"
            || component == "test-utils"
            // `test_support` modules are feature-gated fixtures/scripted
            // doubles, not product code; tests may name concrete products
            // (overview §8), and the fixtures they build (acme-*) legitimately
            // do.
            || component == "test_support"
    }) {
        return true;
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    name == "tests.rs"
        || name == "test_support.rs"
        || name.ends_with("_tests.rs")
        || name.contains(".test.")
        || name.contains(".spec.")
}

/// Mask non-extension references before matching:
///
/// - GitHub *repository URLs* (issue/PR citations, upstream repo links) so
///   the `github` term matches product/API references rather than routine
///   code citations;
/// - `metadata.google.internal` — the cloud metadata endpoint named by SSRF
///   guards, which is security vocabulary, not the google extensions vendor.
fn mask_non_extension_references(source: &str) -> String {
    const REPO_MARKER: &str = "github.com/";
    let mut masked = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(index) = rest.find(REPO_MARKER) {
        masked.push_str(&rest[..index]);
        let after = &rest[index + REPO_MARKER.len()..];
        let end = after
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | ')' | '>' | ','))
            .unwrap_or(after.len());
        rest = &after[end..];
    }
    masked.push_str(rest);
    masked.replace("metadata.google.internal", "")
}

fn scannable_kind(path: &Path) -> Option<FileKind> {
    let name = path.file_name()?.to_string_lossy();
    if name.ends_with(".rs") {
        Some(FileKind::Rust)
    } else if name.ends_with(".ts") || name.ends_with(".tsx") {
        Some(FileKind::Frontend)
    } else if name.ends_with(".toml") {
        Some(FileKind::Toml)
    } else {
        None
    }
}

#[derive(Clone, Copy, PartialEq)]
enum FileKind {
    Rust,
    Frontend,
    Toml,
}

fn scan_file(path: &Path, kind: FileKind, terms: &BTreeSet<String>) -> Vec<String> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let contents = match kind {
        FileKind::Rust => strip_line_anchored_cfg_test_items(&contents),
        FileKind::Frontend | FileKind::Toml => contents,
    };
    let haystack = mask_non_extension_references(&contents).to_ascii_lowercase();
    terms
        .iter()
        .filter(|term| haystack.contains(term.as_str()))
        .cloned()
        .collect()
}

fn scan_dir(
    root: &Path,
    dir: &Path,
    terms: &BTreeSet<String>,
    hits: &mut BTreeMap<String, BTreeSet<String>>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if name == "target" || name == "node_modules" || name == ".git" {
                continue;
            }
            if is_test_source_path(Path::new(&name)) {
                continue;
            }
            scan_dir(root, &path, terms, hits);
            continue;
        }
        if is_test_source_path(&path) {
            continue;
        }
        let Some(kind) = scannable_kind(&path) else {
            continue;
        };
        let matched = scan_file(&path, kind, terms);
        if matched.is_empty() {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        hits.entry(relative).or_default().extend(matched);
    }
}

fn collect_workspace_hits(root: &Path, terms: &BTreeSet<String>) -> BTreeSet<(String, String)> {
    let metadata = cargo_metadata(root);
    let mut hits: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for scan_root in generic_scan_roots(&metadata, root) {
        let src = scan_root.crate_dir.join("src");
        scan_dir(root, &src, terms, &mut hits);
        let manifest = scan_root.crate_dir.join("Cargo.toml");
        if manifest.exists() {
            let matched = scan_file(&manifest, FileKind::Toml, terms);
            if !matched.is_empty() {
                let relative = manifest
                    .strip_prefix(root)
                    .unwrap_or(&manifest)
                    .to_string_lossy()
                    .replace('\\', "/");
                hits.entry(relative).or_default().extend(matched);
            }
        }
    }
    // The WebUI frontend ships from a non-src directory of a generic crate.
    // `scan_dir` returns quietly on an unreadable directory, so a frontend that
    // moved out from under this literal would drop the whole SPA out of the
    // scan while the gate still passed - assert the root before scanning it.
    let frontend = crate_path(root, "crates/ironclaw_webui/frontend/src");
    assert!(
        frontend.is_dir(),
        "WebUI frontend scan root {} does not exist; the SPA would go unscanned while this \
         gate still reported success (docs/reborn/target-architecture/CHECKLIST.md WS10)",
        frontend.display()
    );
    scan_dir(root, &frontend, terms, &mut hits);

    // Resolved through the inventory: these fragments are matched against
    // paths discovered on disk, so a crate that moved makes every fragment
    // naming it stop carving - in the direction that ADDS violations.
    let sanctioned: Vec<String> = SANCTIONED_PATHS
        .iter()
        .map(|fragment| resolve_listed_path(root, fragment))
        .collect();
    hits.into_iter()
        .filter(|(path, _)| {
            !sanctioned
                .iter()
                .any(|fragment| path.contains(fragment.as_str()))
        })
        .flat_map(|(path, terms)| {
            terms
                .into_iter()
                .map(move |term| (path.clone(), term))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Split raw hits into (permanently carved-out, policed). Carve-outs that no
/// longer match anything are returned as stale so the list cannot rot.
fn apply_path_term_collisions(
    root: &Path,
    hits: BTreeSet<(String, String)>,
) -> (BTreeSet<(String, String)>, Vec<String>) {
    // The path fragments are matched against paths discovered on disk, so each
    // is resolved through the crate inventory first. The reported staleness
    // still names the LOGICAL spelling, which is what a reader would edit.
    let carve_outs: Vec<(String, &str, &str)> = PATH_TERM_COLLISIONS
        .iter()
        .map(|(fragment, term, reason)| (resolve_listed_path(root, fragment), *term, *reason))
        .collect();
    let mut used: BTreeSet<(&str, &str)> = BTreeSet::new();
    let policed: BTreeSet<(String, String)> = hits
        .into_iter()
        .filter(|(path, term)| {
            let carved = carve_outs.iter().find(|(fragment, carved_term, _)| {
                path.contains(fragment.as_str()) && term == carved_term
            });
            if let Some((fragment, carved_term, _)) = carved {
                used.insert((fragment.as_str(), carved_term));
                return false;
            }
            true
        })
        .collect();
    let stale = carve_outs
        .iter()
        .zip(PATH_TERM_COLLISIONS.iter())
        .filter(|((fragment, term, _), _)| !used.contains(&(fragment.as_str(), *term)))
        .map(|(_, (fragment, term, _))| format!("{fragment} :: {term}"))
        .collect();
    (policed, stale)
}

// ---------------------------------------------------------------------------
// Allowlist — today's violations, shrink-only, empty by P7 (DEL-8)
// ---------------------------------------------------------------------------

/// Exact `(path, term)` pairs allowed to match today. Every entry is existing
/// debt scheduled for deletion by the extension-runtime phases (P1–P7). Do
/// not add entries for new code — fix the code instead.
const ALLOWLIST: &[(&str, &str)] = &[
    // lane-4: branch — load-bearing generic logic that branches on / hardcodes a specific extension — degenericize by routing on a manifest-declared capability/vendor/effect (Ben's next decision; see PR #6065 lane-4 inventory)
    (
        "crates/ironclaw_host_runtime/src/document_output.rs",
        "google",
    ),
    (
        "crates/ironclaw_host_runtime/src/document_output.rs",
        "google-drive",
    ),
    (
        "crates/ironclaw_host_runtime/src/first_party_tools/http.rs",
        "github",
    ),
    // Presentation-only brand casing: the auth-gate card's display-name table
    // must key on the wire vendor id to function (the prior `git_hub` key
    // never matched and rendered "Github"). Same class as the table's
    // carved-out `nearai` row; behavior stays manifest-generic.
    (
        "crates/ironclaw_webui/frontend/src/pages/chat/components/auth-oauth-card.tsx",
        "github",
    ),
    // The inline-secret guard's vendor token prefixes moved OUT of this list
    // 2026-08-05 and into `PATH_TERM_COLLISIONS`, where the trace-redaction
    // classifier already sits. They are a safety denylist, not extension
    // routing, and CHECKLIST WS5's "slack/telegram token heuristics → the
    // packages that own them" clause is refuted rather than owed — see the
    // carve-out's own reason text for the measurement.
    (
        "crates/ironclaw_host_api/src/product_adapter/identity.rs",
        "slack",
    ),
    (
        "crates/ironclaw_host_api/src/product_adapter/identity.rs",
        "telegram",
    ),
    // Moved pre-existing extension-host fixture debt; these entries should
    // shrink as the old composition-hosted tests become manifest-driven.
    ("crates/ironclaw_extension_host/Cargo.toml", "slack"),
    (
        "crates/ironclaw_extension_contracts/src/auth_prompt.rs",
        "github",
    ),
    (
        "crates/ironclaw_extension_contracts/src/auth_prompt.rs",
        "google",
    ),
    (
        "crates/ironclaw_extension_contracts/src/auth_prompt.rs",
        "notion",
    ),
    (
        "crates/ironclaw_extension_contracts/src/auth_prompt.rs",
        "telegram",
    ),
    ("crates/ironclaw_assistant/Cargo.toml", "telegram"),
    // `conversation_binding.rs` was carved for a vendor example in
    // `ProductConversationRouteKey`'s doc. WS2.2 moved that type to
    // `ironclaw_product_contracts::subject_route`, where a vendor name is
    // forbidden outright, so the example was rewritten generically rather than
    // re-carved. The entry is deleted, not repointed — the allowlist shrinks.
    ("crates/ironclaw_assistant/src/lib.rs", "telegram"),
    // WS5 port inversion: these three wire-DTO sites moved to the contracts
    // crate with their code (`NearAiAuthProvider`'s OAuth identity providers and
    // the project-metadata doc example). Same terms, same debt, new file.
    //
    // Repointed, not deleted and not added: the two entries moved with the
    // `LlmConfigService` port from `ironclaw_assistant` to
    // `ironclaw_product_contracts::operator_llm` (WS5 operator row). Same two
    // terms, same count — the baseline is untouched.
    //
    // The terms are `NearAiAuthProvider::{Github, Google}`, the two SSO
    // identity providers NEAR AI accepts, whose `as_path()` builds
    // `/v1/auth/github` and `/v1/auth/google`. They are not extension package
    // references; they are LLM-vendor admin vocabulary, which PROPOSAL §8.2's
    // vendor rule sanctions for `ironclaw_operator` ("*this* is the LLM-vendor
    // admin layer", §6.9.2).
    //
    // **Recorded finding, not resolved here.** §8.2's sanctioned-vendor list
    // names `packages/*`, `llm` providers, `operator`, `webui::auth` login
    // providers, and recipes-as-data — it does **not** name the contracts
    // family. Declaring the port at the boundary therefore carries vendor
    // vocabulary somewhere the rule does not cover, and this pair is only the
    // visible tip: `LlmConfigService` has three vendor-named *methods*
    // (`start_nearai_login`, `complete_nearai_wallet_login`,
    // `start_codex_login`) and six vendor-named DTOs the scanner's inventory
    // does not know to flag. Generifying that is a design change — NEAR AI SSO,
    // NEAR wallet NEP-413, and OpenAI Codex device-code are three different
    // protocols behind one port — and PLAN's operating principle 2 keeps
    // semantic changes out of a move PR. The finding is in the WS5 row.
    (
        "crates/ironclaw_product_contracts/src/operator_llm.rs",
        "github",
    ),
    (
        "crates/ironclaw_product_contracts/src/operator_llm.rs",
        "google",
    ),
    (
        "crates/ironclaw_product_contracts/src/workspace_views.rs",
        "github",
    ),
    ("crates/ironclaw_assistant/src/workflow.rs", "slack"),
    ("crates/ironclaw_host_api/src/dispatch.rs", "slack"),
    (
        "crates/ironclaw_host_runtime/src/first_party_tools/schemas.rs",
        "slack",
    ),
    (
        "crates/ironclaw_host_runtime/src/services/wasm_execution.rs",
        "slack",
    ),
    (
        "crates/ironclaw_turn_runner/src/loop_driver_host.rs",
        "slack",
    ),
    ("crates/ironclaw_loop_host/src/tool_disclosure.rs", "google"),
    (
        "crates/ironclaw_loop_host/src/tool_disclosure.rs",
        "google-calendar",
    ),
    (
        "crates/ironclaw_loop_host/src/tool_disclosure.rs",
        "web-access",
    ),
    (
        "crates/ironclaw_loop_host/src/tool_disclosure_port.rs",
        "google",
    ),
    (
        "crates/ironclaw_loop_host/src/tool_disclosure_port.rs",
        "google-calendar",
    ),
    (
        "crates/ironclaw_extension_host/src/available_extensions.rs",
        "google",
    ),
    (
        "crates/ironclaw_assistant/src/projection/display_preview.rs",
        "web-access",
    ),
    // DEL-7: the manifest-egress policy builder's comment names the GSuite
    // (Google API hosts) and web-access (Exa MCP) egress it no longer
    // special-cases — the code routes purely on manifest-declared targets.
    (
        "crates/ironclaw_extension_host/src/capability_surface.rs",
        "google",
    ),
    (
        "crates/ironclaw_extension_host/src/capability_surface.rs",
        "web-access",
    ),
    // lane-4: nearai-slice — the last catalog package (nearai_mcp) is still
    // patched from LLM-admin bootstrap config; it now lives with the generic
    // available-extension catalog in ironclaw_extension_host.
    //
    // The `nearai-mcp` *asset-directory* carve-out for this file is gone (WS2
    // include_str kills): the package's embeds moved to the inventory module
    // `ironclaw_extension_support::packages::nearai`, which is a sanctioned
    // scan-exempt crate, so this file no longer names the directory. The
    // `nearai_mcp` / `nearaimcp` entries below stay — the endpoint patch and
    // the `nearai_mcp` module it reads are still here, by §6.8.2 (the fork is
    // what prevents an `extension_host -> operator` upward edge).
    (
        "crates/ironclaw_extension_host/src/available_extensions.rs",
        "nearai_mcp",
    ),
    (
        "crates/ironclaw_extension_host/src/available_extensions.rs",
        "nearaimcp",
    ),
    ("crates/ironclaw_extension_host/src/lib.rs", "nearai_mcp"),
    ("crates/ironclaw_extension_host/src/lib.rs", "nearaimcp"),
    (
        "crates/ironclaw_extension_host/src/nearai_mcp.rs",
        "nearai_mcp",
    ),
    (
        "crates/ironclaw_extension_host/src/nearai_mcp.rs",
        "nearaimcp",
    ),
    (
        "crates/ironclaw_composition/src/factory/auth_engine_assembly.rs",
        "nearai_mcp",
    ),
    (
        "crates/ironclaw_composition/src/factory/production_backend_assembly.rs",
        "nearai_mcp",
    ),
    (
        "crates/ironclaw_composition/src/factory/production_backend_assembly.rs",
        "nearaimcp",
    ),
    (
        "crates/ironclaw_composition/src/factory/production_build_assembly.rs",
        "nearai_mcp",
    ),
    (
        "crates/ironclaw_composition/src/factory/production_build_assembly.rs",
        "nearaimcp",
    ),
    ("crates/ironclaw_composition/src/input.rs", "nearai_mcp"),
    ("crates/ironclaw_composition/src/input.rs", "nearaimcp"),
    (
        "crates/ironclaw_composition/src/deployment.rs",
        "nearai_mcp",
    ),
    ("crates/ironclaw_composition/src/deployment.rs", "nearaimcp"),
    (
        "crates/ironclaw_composition/src/llm_admin/mod.rs",
        "nearai_mcp",
    ),
    (
        "crates/ironclaw_operator/src/llm_admin/mod.rs",
        "nearai_mcp",
    ),
    (
        "crates/ironclaw_composition/src/llm_admin/nearai_mcp.rs",
        "nearai_mcp",
    ),
    (
        "crates/ironclaw_composition/src/llm_admin/nearai_mcp.rs",
        "nearaimcp",
    ),
    (
        "crates/ironclaw_operator/src/llm_admin/nearai_mcp.rs",
        "nearai_mcp",
    ),
    (
        "crates/ironclaw_operator/src/llm_admin/nearai_mcp.rs",
        "nearaimcp",
    ),
    (
        "crates/ironclaw_auth/src/product_auth/api/auth.rs",
        "nearai_mcp",
    ),
    ("crates/ironclaw_composition/src/runtime.rs", "nearai_mcp"),
    // lane-4: migration — one-time forward-migration call sites naming the v1 vocabulary they fold forward — correct-by-design (same pattern the retired-taxonomy gate sanctions); would become a SANCTIONED_PATHS carve if the sites move into a dedicated migration module
    // lane-4: doc-str — incidental doc-comment / error-string / tool-description examples that NAME an extension but branch on nothing — the code routes by a manifest field (display_name/provider/effects); reword or leave (Ben's call)
    (
        "crates/ironclaw_extension_contracts/src/surface.rs",
        "slack",
    ),
    ("crates/ironclaw_filesystem/src/index.rs", "acme"),
    ("crates/ironclaw_host_api/src/capability.rs", "slack"),
    ("crates/ironclaw_host_api/src/http.rs", "slack"),
    ("crates/ironclaw_host_api/src/ids.rs", "github"),
    ("crates/ironclaw_loop_host/src/capability_port.rs", "gmail"),
    (
        "crates/ironclaw_outbound/src/delivered_gate_routes.rs",
        "slack",
    ),
    // ✎ 2026-08-05: was `crates/ironclaw_projects/src/lib.rs`. The crate merged
    // into `ironclaw_identity` as its `projects` module (WS10 / PROPOSAL
    // §12.10), so the same doc-comment hit moved with the file. A 1-for-1
    // repoint: the count is unchanged and this stays a `lane-4: doc-str` row
    // awaiting the owner's reword-or-leave call, not a new debt.
    ("crates/domains/ironclaw_identity/src/projects.rs", "github"),
    (
        "crates/ironclaw_assistant/src/blocked_auth_resume.rs",
        "google",
    ),
    (
        "crates/ironclaw_assistant/src/blocked_auth_resume.rs",
        "slack",
    ),
    ("crates/ironclaw_composition/src/factory.rs", "google"),
    (
        "crates/ironclaw_composition/src/factory/production_backend_assembly.rs",
        "google",
    ),
    (
        "crates/ironclaw_composition/src/factory/runtime_lane_assembly.rs",
        "notion",
    ),
    (
        "crates/ironclaw_composition/src/google_oauth_secret_store.rs",
        "google",
    ),
    ("crates/ironclaw_composition/src/lib.rs", "google"),
    (
        "crates/ironclaw_operator/src/llm_admin/nearai_login_serve.rs",
        "github",
    ),
    (
        "crates/ironclaw_operator/src/llm_admin/nearai_login_serve.rs",
        "google",
    ),
    ("crates/ironclaw_auth/src/product_auth/api/auth.rs", "slack"),
    (
        "crates/ironclaw_auth/src/product_auth/credentials/runtime_credentials.rs",
        "google",
    ),
    ("crates/ironclaw_webui/src/product_auth/mod.rs", "slack"),
    (
        "crates/ironclaw_assistant/src/reborn_services/outbound_delivery_capability_surface.rs",
        "slack",
    ),
    ("crates/ironclaw_assistant/src/projection.rs", "slack"),
    (
        "crates/ironclaw_assistant/src/projection/turn_events.rs",
        "slack",
    ),
    (
        "crates/ironclaw_assistant/src/communication_context.rs",
        "slack",
    ),
    ("crates/ironclaw_skills/src/selector.rs", "github"),
    ("crates/ironclaw_skills/src/types.rs", "github"),
    ("crates/ironclaw_skills/src/types.rs", "google"),
    ("crates/ironclaw_skills/src/types.rs", "slack"),
    (
        "crates/ironclaw_config/src/capability_remediation.rs",
        "gmail",
    ),
    (
        "crates/ironclaw_config/src/capability_remediation.rs",
        "google",
    ),
    ("crates/ironclaw_config/src/config_file.rs", "gmail"),
    ("crates/ironclaw_config/src/config_file.rs", "google"),
    // The retired-section gravestones: the only place this crate still
    // names a vendor, and only as the TOML table name an operator typed.
    ("crates/ironclaw_config/src/retired_sections.rs", "slack"),
    ("crates/ironclaw_config/src/retired_sections.rs", "telegram"),
    ("crates/ironclaw_config/src/lib.rs", "google"),
    // lane-4: dev-deps — sanctioned DEL-7 linkage of concrete channel crates
    // for the production-shaped channel-host E2E suite; the scanner sees the
    // crate names in Cargo.toml even though Rust test sources are excluded.
    ("crates/ironclaw_composition/Cargo.toml", "slack"),
    // lane-4: branch — the provider catalog names github_copilot (an LLM
    // provider id, not the github extension); degenericize with the catalog
    // slice.
    (
        "crates/ironclaw_operator/src/llm_admin/provider_admin.rs",
        "github",
    ),
    // lane-4: doc — NEAR AI login copy names its upstream SSO providers.
    (
        "crates/ironclaw_operator/src/llm_admin/nearai_login_serve.rs",
        "github",
    ),
    // Sixth-fold debt: the second-channel host PR's frontend surface
    // (pairing/setup panels, setup API clients, chat/configure wiring, and
    // localized pairing copy). Consumed by the generic descriptor-driven
    // pairing seam (extension-runtime P2), which moves product copy and
    // routing onto manifest-declared account-setup descriptors.
    (
        "crates/ironclaw_webui/frontend/src/components/telegram-setup-panel.tsx",
        "telegram",
    ),
    (
        "crates/ironclaw_webui/frontend/src/lib/channel-setup-api.ts",
        "slack",
    ),
    (
        "crates/ironclaw_webui/frontend/src/lib/channel-setup-api.ts",
        "telegram",
    ),
    (
        "crates/ironclaw_webui/frontend/src/lib/telegram-setup-api.ts",
        "slack",
    ),
    (
        "crates/ironclaw_webui/frontend/src/lib/telegram-setup-api.ts",
        "telegram",
    ),
    ("crates/ironclaw_webui/frontend/src/i18n/ar.ts", "slack"),
    ("crates/ironclaw_webui/frontend/src/i18n/ar.ts", "telegram"),
    ("crates/ironclaw_webui/frontend/src/i18n/de.ts", "slack"),
    ("crates/ironclaw_webui/frontend/src/i18n/de.ts", "telegram"),
    ("crates/ironclaw_webui/frontend/src/i18n/en.ts", "slack"),
    ("crates/ironclaw_webui/frontend/src/i18n/en.ts", "telegram"),
    ("crates/ironclaw_webui/frontend/src/i18n/es.ts", "slack"),
    ("crates/ironclaw_webui/frontend/src/i18n/es.ts", "telegram"),
    ("crates/ironclaw_webui/frontend/src/i18n/fr.ts", "slack"),
    ("crates/ironclaw_webui/frontend/src/i18n/fr.ts", "telegram"),
    ("crates/ironclaw_webui/frontend/src/i18n/hi.ts", "slack"),
    ("crates/ironclaw_webui/frontend/src/i18n/hi.ts", "telegram"),
    ("crates/ironclaw_webui/frontend/src/i18n/ja.ts", "slack"),
    ("crates/ironclaw_webui/frontend/src/i18n/ja.ts", "telegram"),
    ("crates/ironclaw_webui/frontend/src/i18n/ko.ts", "slack"),
    ("crates/ironclaw_webui/frontend/src/i18n/ko.ts", "telegram"),
    ("crates/ironclaw_webui/frontend/src/i18n/pt-BR.ts", "slack"),
    (
        "crates/ironclaw_webui/frontend/src/i18n/pt-BR.ts",
        "telegram",
    ),
    ("crates/ironclaw_webui/frontend/src/i18n/uk.ts", "slack"),
    ("crates/ironclaw_webui/frontend/src/i18n/uk.ts", "telegram"),
    ("crates/ironclaw_webui/frontend/src/i18n/zh-CN.ts", "slack"),
    (
        "crates/ironclaw_webui/frontend/src/i18n/zh-CN.ts",
        "telegram",
    ),
];

/// WS0 baseline for the extension-specificity allowlist (target-architecture
/// epic #3773, workstream #6920): the number of `(path, term)` pairs measured
/// **on this checkout**, not copied from the design docs.
///
/// Measured 2026-07-30 against `origin/main` @ `ae0989c37` by counting the
/// entries of `ALLOWLIST` above (`ALLOWLIST.len()`, printed by the ratchet
/// below on failure).
///
/// Re-measured 2026-08-04 during the Wave 4 consolidation, and **lowered
/// 125 → 124**. The vendor-config retirement removes two net entries, which
/// against the `origin/main` this branch was first cut from (`1e2a294083`,
/// `ALLOWLIST` = 127) gave 125. #7094 then deleted a further entry on `main`
/// (127 → 126), so the same two removals now land on **124**. The count is
/// read off the ratchet's own failure message with this constant temporarily
/// set to `0`, never counted by eye — a plain paren count over the literal
/// answers 142, because the entries' comments contain parentheses too.
///
/// PROPOSAL §11.2.8 shrinks this list to the §8.1 rule-4 set and CHECKLIST
/// WS12 wants it empty. The staleness half of that discipline already lives in
/// the gate above (an entry that no longer matches fails); this ceiling is the
/// other half — the list cannot *grow* untracked either. Lower it in the same
/// PR that deletes entries so the new floor is locked in.
///
/// ✎ **129 → 125, and the gate below is now an equality (2026-08-04).**
/// Two branches found the same defect independently and this is their union;
/// the count was **recounted on the merged tree**, not taken from either side.
///
/// - **#7143 deleted an entry** — `("…/lifecycle_restore.rs", "slack")`, made
///   stale by the retired-identity branch deletion — and lowered the ceiling in
///   the same PR, which is exactly what the sentence above requires. Shipping
///   without that would have banked the deletion as slack rather than as a
///   floor.
/// - **#7147 found the ceiling was already carrying 3 entries of slack before
///   either branch.** The list held **126** pairs against a constant of 129, so
///   three earlier deletions lowered the list without lowering the ceiling —
///   silent, because a `<=` ratchet says nothing when the constant sits *above*
///   the list. Three new vendor carve-outs could have been added green.
/// - **#7147 also closed the mechanism, not just the number:** the gate below
///   is an **equality**. Slack now fails with its own message, so this cannot
///   recur — a deletion that forgets to lower the constant is a red build
///   instead of a fresh budget for the next carve-out.
///
/// Counted with a script over the entries between `const ALLOWLIST` and its
/// closing `];`, ignoring comment lines, on the **pushed ref** — not by eye and
/// not with `grep`, which also matches prose. Tracked as #7147; whichever of
/// the concurrent branches touching this list merges last must recount the
/// union rather than trusting any single branch's number.
///
/// ✎ **125 → 123 (2026-08-04, #7139 — the Wave 4 consolidation).** Neither
/// side of this merge was right for the union, so it was recounted rather than
/// picked. #7143 set **125** measuring its own branch; this branch had **124**
/// measuring its own. #7143 additionally *deleted* an entry
/// (`lifecycle_restore.rs`/`slack`) that this branch never saw, so the merged
/// list is lower than either: 126 on `main` after #7094, minus this branch's
/// two net vendor-config removals, minus #7143's one, is **123**.
///
/// Read off the ratchet's own failure message with this constant temporarily
/// set to `0` (`ALLOWLIST grew to 123 entries`) — never counted by eye, and
/// never with `grep`, which also matches prose (a paren count over the literal
/// once answered 142 because the entries' comments contain parentheses).
///
/// ⚠ **#7141 and #7152 are still open and both touch this list** (#7147).
/// Whichever merges last must recount the union the same way rather than
/// inheriting 123, 124 or 125 from any single branch.
///
/// ✎ **Union recount, 2026-08-04 (WS3/WS4 consolidation — the merge-last
/// recount #7147 asks for): the answer is 125.** Both branches lowered this
/// constant independently and each was right about its own tree — #7143
/// measured **125** after deleting the `lifecycle_restore.rs`/slack pair, and
/// the WS3/WS4 consolidation measured **126** after finding a slot of
/// pre-existing slack (129 → 127, then 126). Merging them, the union is
/// **125**: #7143's deletion is the only entry either branch removed, and the
/// WS3/WS4 work (the catalog-defaults move to `host_api`/`extensions`, the
/// `capabilities/host.rs` split) relocates code without naming a vendor, so it
/// removes no pair. The prediction going in was 124 — that both branches would
/// each contribute a deletion — and it was WRONG; the compiler said 125, which
/// is exactly why this is measured rather than reasoned. Read back the way the
/// paragraph above requires: baseline set to `0`, the ratchet's own failure
/// message reporting the length, never by eye and never with `grep` (which
/// matches prose, and once answered 142 where the truth was 124).///
/// ✎ **Union-of-unions recount, 2026-08-04 (#7139 merged to main, then main
/// merged into #7141):** per the ⚠ note above, recounted by the ratchet's own
/// failure message with the baseline set to 0 — never inherited from either
/// side (#7141's own-tree 125, #7139's 123). The ratchet reported **123**
/// — #7139's removals and #7141's repoints do not overlap on any entry.
/// The number is read off the **compiler** — set the constant to `0`, run the
/// gate, and the panic prints `ALLOWLIST.len()` — never by counting parens or
/// `grep`-ing the file, both of which also match the entry comments and the
/// prose above.
///
/// ⚠ **#7139 and #7141 also touch this list.** Whichever merges next must
/// recount on *its* merged tree and set this constant to that number in the
/// merge commit. The equality turns a stale union into a red build rather than
/// into new silent slack — which is the point — but it makes the recount
/// mandatory rather than optional.///
/// ✎ **Batch union recount, 2026-08-04 (Waves 0–4 batch: #7141+#7160+#7159+
/// #7161+#7156).** Recounted on the batch's merged tree per the mandate
/// above, by the ratchet's own failure message with the constant set to 0:
/// the batch union is **123** — #7161's conversions repoint entries in place
/// and add none.
///
/// ✎ **Union recount, 2026-08-05 (WS6 renames #7152 refreshed onto the
/// Waves 0–4 batch + the D-A factory port).** The merge-last recount the ⚠
/// notes above make mandatory. #7152 measured **125** on its own tree and the
/// incoming batch measured **122**; neither is evidence for the union, so the
/// constant was set to `0` and the true length read out of the ratchet's own
/// panic on the merged tree. Never by eye and never with `grep` — both also
/// match the entry comments and this prose. The ratchet answered **122**: the
/// batch's three vendor-pair removals (the `adapter_registry` pair repointed
/// onto `extension_registry::host_api::product_adapter`, and the
/// `config_file.rs` slack/telegram pair moved to `retired_sections.rs`) are the
/// only entries either side removed, and this branch's renames repoint entries
/// in place without adding any. So the union is the batch's number, not ours —
/// which is exactly why it is measured rather than reasoned.
///
/// **122 → 119, 2026-08-05 (WS5 `product` narrows, token-heuristics clause).**
/// The three `ironclaw_extension_registry/src/host_api/product_adapter.rs`
/// rows left the allowlist and became `PATH_TERM_COLLISIONS` carve-outs with
/// reasons. That is a re-classification, not a payment: allowlist entries are
/// *debt* — vendor names that ought to leave — and the measurement on that row
/// showed these three never can, because the guard they belong to is a safety
/// denylist that is a superset of the package inventory and the move its clause
/// named is layer-illegal besides. Keeping them in the debt column would have
/// left a permanent entry the shrink ratchet can never retire; the carve-out
/// list is where the trace-redaction classifier already records the identical
/// disposition. Net vendor-name surface is unchanged: three rows moved list,
/// zero terms appeared or vanished.
///
/// **119 → 118, 2026-08-06 (channel-generic trigger settlement delivery).**
/// The trigger-poller bridge now describes its owner as the channel delivery
/// hook instead of naming one concrete extension. The stale `slack` exception
/// was removed after the specificity scan proved the production file no
/// longer contains that vendor term.
const WS0_EXTENSION_SPECIFICITY_ALLOWLIST_BASELINE: usize = 118;

/// §11.2.8 vendor-scope shrink, armed at the WS0 baseline.
///
/// **Equality, not `<=`.** Growth past the baseline is new vendor debt;
/// *slack* (a baseline above the live list) is an unclaimed budget for exactly
/// that debt, and a one-directional ratchet cannot see it. Both directions
/// fail, with distinct messages, so the constant tracks the list on every
/// commit.
#[test]
fn reborn_extension_specificity_allowlist_ratchets_down_only() {
    assert!(
        !ALLOWLIST.is_empty(),
        "extension-specificity ALLOWLIST is empty — the ratchet below would pass having \
         measured nothing. Either the list really reached §11.2.8's target (delete this gate \
         and the baseline together, and say so in the PR) or the const was truncated."
    );
    assert!(
        ALLOWLIST.len() <= WS0_EXTENSION_SPECIFICITY_ALLOWLIST_BASELINE,
        "extension-specificity ALLOWLIST grew to {} entries (WS0 baseline {}): this list is \
         shrink-only (PROPOSAL §11.2.8). Degenericize the code — route on a manifest-declared \
         capability instead of naming the vendor — rather than allowlisting a new pair. If the \
         owner has approved a deliberate carve-out, raise \
         WS0_EXTENSION_SPECIFICITY_ALLOWLIST_BASELINE in the same PR with the rationale in the \
         PR body.",
        ALLOWLIST.len(),
        WS0_EXTENSION_SPECIFICITY_ALLOWLIST_BASELINE
    );
    assert!(
        ALLOWLIST.len() >= WS0_EXTENSION_SPECIFICITY_ALLOWLIST_BASELINE,
        "extension-specificity ALLOWLIST holds {} entries but \
         WS0_EXTENSION_SPECIFICITY_ALLOWLIST_BASELINE is {} — {} entries of UNTRACKED SLACK. \
         A ceiling above the live list is an unclaimed budget: that many new vendor carve-outs \
         can be added and every gate stays green. Lower the constant to {} in the PR that \
         deleted the entries, which is what the doc comment on it already required (#7147).",
        ALLOWLIST.len(),
        WS0_EXTENSION_SPECIFICITY_ALLOWLIST_BASELINE,
        WS0_EXTENSION_SPECIFICITY_ALLOWLIST_BASELINE - ALLOWLIST.len(),
        ALLOWLIST.len()
    );
}

/// One `(relative path, matched term)` scanner hit.
type HitEntry = (String, String);

fn classify_hits(
    root: &Path,
    hits: &BTreeSet<HitEntry>,
    allowlist: &[(&str, &str)],
) -> (Vec<HitEntry>, Vec<HitEntry>) {
    // Resolved through the inventory: a moved crate would otherwise turn every
    // entry naming it into BOTH a stale entry and a new violation at once.
    let allowed: BTreeSet<HitEntry> = allowlist
        .iter()
        .map(|(path, term)| (resolve_listed_path(root, path), term.to_string()))
        .collect();
    let new_violations = hits.difference(&allowed).cloned().collect();
    let stale_entries = allowed.difference(hits).cloned().collect();
    (new_violations, stale_entries)
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// A carve-out that names a directory which is not there stops carving
/// anything out — silently, and in the direction that *adds* terms. Pin the
/// two provider package directories so a rename has to come here.
#[test]
fn the_non_vendor_provider_carve_out_names_real_packages() {
    let packages = packages_root(&workspace_root());
    for name in NON_VENDOR_PROVIDER_PACKAGE_DIRS {
        let dir = packages.join(name);
        assert!(
            dir.join("manifest.toml").is_file(),
            "NON_VENDOR_PROVIDER_PACKAGE_DIRS names {name}, but {} has no manifest.toml. \
             A stale entry excludes nothing; a renamed package silently rejoins the vendor \
             vocabulary. Repoint it in the same change that moved the package.",
            dir.display()
        );
    }
}

#[test]
fn reborn_generic_code_names_no_concrete_extension() {
    let root = workspace_root();
    let terms = derive_forbidden_terms(&inventory_dirs(&root));
    assert!(
        !terms.is_empty(),
        "inventory-derived term set must not be empty — is the package inventory readable?"
    );
    let raw_hits = collect_workspace_hits(&root, &terms);
    let (hits, stale_carve_outs) = apply_path_term_collisions(&root, raw_hits);
    let (new_violations, stale_entries) = classify_hits(&root, &hits, ALLOWLIST);

    let mut failures = Vec::new();
    if !stale_carve_outs.is_empty() {
        failures.push(format!(
            "stale PATH_TERM_COLLISIONS carve-outs (nothing matches any more — delete \
             them):\n{}",
            stale_carve_outs.join("\n")
        ));
    }
    if !new_violations.is_empty() {
        failures.push(format!(
            "concrete extension names in generic code (fix the code, or — for pre-existing \
             debt only — add the exact entries below to ALLOWLIST, which also requires raising \
             WS0_EXTENSION_SPECIFICITY_ALLOWLIST_BASELINE, a reviewed decision):\n{}",
            new_violations
                .iter()
                .map(|(path, term)| format!("    (\"{path}\", \"{term}\"),"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if !stale_entries.is_empty() {
        failures.push(format!(
            "stale ALLOWLIST entries (the file no longer matches the term — delete the \
             entries; the allowlist only shrinks):\n{}",
            stale_entries
                .iter()
                .map(|(path, term)| format!("    (\"{path}\", \"{term}\"),"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    assert!(
        failures.is_empty(),
        "extension specificity gate failed:\n\n{}",
        failures.join("\n\n")
    );
}

// ---------------------------------------------------------------------------
// Dependency gate — only the binary and tests link concrete extension crates
// ---------------------------------------------------------------------------

/// Production (normal/build) dependency edges onto concrete extension crates
/// that predate the unified runtime. Each names the phase that deletes it; a
/// stale entry (edge no longer present) fails so the list only shrinks.
/// Checklist DEL-7 requires this list empty.
const CONCRETE_DEPENDENCY_EXCEPTIONS: &[(&str, &str, &str)] = &[];

#[test]
fn concrete_extension_crates_link_only_from_the_binary_and_tests() {
    let root = workspace_root();
    let metadata = cargo_metadata(&root);
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");

    // Fail-open guard: a concrete crate with a Cargo.toml on disk must be a
    // registered workspace member, or its edges would be invisible here.
    let registered: BTreeSet<&str> = packages
        .iter()
        .filter_map(|package| package["name"].as_str())
        .collect();
    // Resolved by PACKAGE NAME across the crate inventory, not by joining
    // `crates/<package name>/Cargo.toml`.
    //
    // The old join assumed directory == package name. WS2 colocation made the
    // directories `extensions/packages/{slack,telegram}` while the packages
    // kept their `ironclaw_*_extension` names, so the join has been resolving
    // to a path that does not exist and this fail-open guard has been checking
    // **zero** crates ever since — green, and measuring nothing. Walking the
    // inventory and reading each manifest's declared name fixes that and is
    // also what survives the family move (WS10).
    let mut resolved_concrete: Vec<&str> = Vec::new();
    for directory in crate_directories(&root) {
        let manifest = root.join(&directory).join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let Some(package_name) = text.lines().find_map(|line| {
            line.trim()
                .strip_prefix("name = \"")
                .and_then(|rest| rest.strip_suffix('"'))
        }) else {
            continue;
        };
        let Some(concrete) = CONCRETE_EXTENSION_CRATES
            .iter()
            .find(|concrete| **concrete == package_name)
        else {
            continue;
        };
        resolved_concrete.push(concrete);
        assert!(
            registered.contains(concrete),
            "{concrete} has a Cargo.toml at {} but is not in cargo metadata; register it as \
             a workspace member so this gate actually checks its dependents",
            manifest.display()
        );
    }
    // A planned-but-absent concrete crate is tolerated by design, but ALL of
    // them being absent means the guard resolved nothing and checked nothing.
    assert!(
        !resolved_concrete.is_empty(),
        "no concrete extension package resolved to a manifest, so the registration guard \
         checked nothing. CONCRETE_EXTENSION_CRATES is {CONCRETE_EXTENSION_CRATES:?} \
         (docs/reborn/target-architecture/CHECKLIST.md WS10)."
    );

    let mut violations = Vec::new();
    let mut used_exceptions = BTreeSet::new();
    for package in packages {
        let Some(name) = package["name"].as_str() else {
            continue;
        };
        if !(name == "ironclaw" || name.starts_with("ironclaw_")) {
            continue;
        }
        if name == "ironclaw" || CONCRETE_EXTENSION_CRATES.contains(&name) {
            continue;
        }
        let Some(dependencies) = package["dependencies"].as_array() else {
            continue;
        };
        for dependency in dependencies {
            let Some(dependency_name) = dependency["name"].as_str() else {
                continue;
            };
            if !CONCRETE_EXTENSION_CRATES.contains(&dependency_name) {
                continue;
            }
            // Dev-dependencies are the sanctioned test linkage.
            if dependency["kind"].as_str() == Some("dev") {
                continue;
            }
            if CONCRETE_DEPENDENCY_EXCEPTIONS
                .iter()
                .any(|(dependent, concrete, _)| *dependent == name && *concrete == dependency_name)
            {
                used_exceptions.insert((name.to_string(), dependency_name.to_string()));
                continue;
            }
            violations.push(format!(
                "{name} takes a production dependency on concrete extension crate \
                 {dependency_name}; only ironclaw (native factory assembly) and \
                 dev-dependencies may"
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "concrete extension crate dependency gate failed:\n{}",
        violations.join("\n")
    );

    let stale: Vec<String> = CONCRETE_DEPENDENCY_EXCEPTIONS
        .iter()
        .filter(|(dependent, concrete, _)| {
            !used_exceptions.contains(&(dependent.to_string(), concrete.to_string()))
        })
        .map(|(dependent, concrete, removes_in)| {
            format!("{dependent} -> {concrete} (scheduled removal: {removes_in})")
        })
        .collect();
    assert!(
        stale.is_empty(),
        "stale CONCRETE_DEPENDENCY_EXCEPTIONS entries — the edge is gone, delete the \
         exception:\n{}",
        stale.join("\n")
    );
}

fn cargo_metadata(root: &Path) -> Value {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .output()
        .expect("cargo metadata must run");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata must be valid JSON")
}

// ---------------------------------------------------------------------------
// Scanner self-tests (TEST-6, TEST-7)
// ---------------------------------------------------------------------------

/// TEST-6: the forbidden vocabulary derives from the inventory — an invented
/// package id is caught without editing the scanner.
#[test]
fn scanner_derives_terms_from_an_invented_inventory_package() {
    let temp = tempfile::tempdir().expect("tempdir");
    let package_dir = temp.path().join("zephyr-chat");
    std::fs::create_dir_all(&package_dir).expect("create package dir");
    std::fs::write(
        package_dir.join("manifest.toml"),
        r#"
schema_version = "reborn.extension_manifest.v3"
id = "zephyr-chat"
name = "Zephyr Chat"
version = "0.1.0"
description = "invented"
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "zephyr-chat.extension/v1"

[[tools]]
id = "zephyr.send"
description = "send"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/zephyr/send.input.v1.json"

[[tools.credentials]]
handle = "zephyr_token"
vendor = "zephyr"
scopes = ["send"]
audience = { scheme = "https", host = "api.zephyr.example" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }

[auth.zephyr]
method = "oauth2_code"
display_name = "Zephyr account"
authorization_endpoint = "https://auth.zephyr.example/authorize"
token_endpoint = "https://auth.zephyr.example/token"
scopes = ["send"]
"#,
    )
    .expect("write manifest");

    let terms = derive_forbidden_terms(&[temp.path().to_path_buf()]);
    for expected in [
        "zephyr-chat",
        "zephyr_chat",
        "zephyrchat",
        "zephyr",
        "api.zephyr.example",
        "auth.zephyr.example",
    ] {
        assert!(
            terms.contains(expected),
            "expected derived term `{expected}`; derived set: {terms:?}"
        );
    }

    // A generic source file naming the invented product is caught, including
    // the camel-case compound form.
    let source_dir = temp.path().join("generic_crate/src");
    std::fs::create_dir_all(&source_dir).expect("create src dir");
    let source_path = source_dir.join("lib.rs");
    std::fs::write(
        &source_path,
        "pub struct ZephyrChatDelivery;\npub fn route() -> &'static str { \"generic\" }\n",
    )
    .expect("write source");
    let matched = scan_file(&source_path, FileKind::Rust, &terms);
    assert!(
        matched.contains(&"zephyrchat".to_string()),
        "camel-case compound must be caught; matched: {matched:?}"
    );
}

/// TEST-7: the allowlist can only shrink — stale entries fail and new
/// violations fail.
#[test]
fn scanner_allowlist_is_shrink_only() {
    let hits: BTreeSet<(String, String)> = [
        ("crates/a/src/lib.rs".to_string(), "slack".to_string()),
        ("crates/b/src/lib.rs".to_string(), "github".to_string()),
    ]
    .into_iter()
    .collect();
    let allowlist = [
        ("crates/a/src/lib.rs", "slack"),
        ("crates/gone/src/lib.rs", "gmail"),
    ];
    let (new_violations, stale_entries) = classify_hits(&workspace_root(), &hits, &allowlist);
    assert_eq!(
        new_violations,
        vec![("crates/b/src/lib.rs".to_string(), "github".to_string())],
        "an unlisted hit must be reported as a new violation"
    );
    assert_eq!(
        stale_entries,
        vec![("crates/gone/src/lib.rs".to_string(), "gmail".to_string())],
        "an allowlist entry with no matching hit must be reported stale"
    );
}

/// WS10: the allowlist keys on the crate NAME, not on the crate's directory.
///
/// A family move (PROPOSAL section 5) changes every discovered path at once. If
/// the listed paths were compared literally, every entry naming a moved crate
/// would be reported BOTH stale and as a new violation in the same run — ~215
/// of them, which is the lockstep sweep this row exists to remove. Resolution
/// makes the same entry keep matching; an entry naming a crate that is really
/// gone still falls through to `stale`.
#[test]
fn reborn_allowlist_entries_follow_a_crate_into_its_family_directory() {
    let temporary = tempfile::tempdir().expect("tempdir");
    let root = temporary.path();
    std::fs::write(root.join("Cargo.toml"), "[workspace]\n").expect("root manifest");
    for index in 0..24 {
        let filler = root.join(format!("crates/ironclaw_filler_{index}"));
        std::fs::create_dir_all(&filler).expect("filler crate");
        std::fs::write(filler.join("Cargo.toml"), "[package]\n").expect("filler manifest");
    }
    let moved = root.join("crates/substrates/ironclaw_llm");
    std::fs::create_dir_all(moved.join("src")).expect("moved crate");
    std::fs::write(moved.join("Cargo.toml"), "[package]\n").expect("moved manifest");

    // What the scanner would discover on the moved tree.
    let hits: BTreeSet<HitEntry> = [(
        "crates/substrates/ironclaw_llm/src/registry.rs".to_string(),
        "nearai".to_string(),
    )]
    .into_iter()
    .collect();
    // What the allowlist still says, unedited.
    let allowlist = [
        ("crates/ironclaw_llm/src/registry.rs", "nearai"),
        ("crates/ironclaw_deleted/src/lib.rs", "slack"),
    ];

    let (new_violations, stale_entries) = classify_hits(root, &hits, &allowlist);
    assert!(
        new_violations.is_empty(),
        "an allowlisted hit must stay allowlisted after its crate moves: {new_violations:?}"
    );
    assert_eq!(
        stale_entries,
        vec![(
            "crates/ironclaw_deleted/src/lib.rs".to_string(),
            "slack".to_string()
        )],
        "an entry naming a crate that no longer exists must still be reported stale"
    );
}

/// The carve-out list stays scoped to the documented LLM-vocabulary
/// collisions; broadening it is a gate regression.
#[test]
fn term_collision_carve_outs_stay_documented_and_narrow() {
    assert_eq!(
        TERM_COLLISIONS
            .iter()
            .map(|(term, _)| *term)
            .collect::<Vec<_>>(),
        vec!["nearai", "near.ai"],
        "bare-term carve-outs are reserved for LLM-provider vocabulary collisions"
    );
    let carved_terms: BTreeSet<&str> = PATH_TERM_COLLISIONS
        .iter()
        .map(|(_, term, _)| *term)
        .collect();
    assert_eq!(
        carved_terms,
        BTreeSet::from([
            "google",
            "github",
            "slack",
            "gmail",
            "google_calendar",
            "google-calendar",
            "notion",
            "telegram",
            "api.github.com",
            "private.near.ai",
            "accounts.google.com",
            "oauth2.googleapis.com",
            "www.googleapis.com",
        ]),
        "path-scoped carve-outs are reserved for the four documented collision domains \
         (LLM providers, SSO login, GitHub-as-skill-source, vendor-safety detection — \
         credential-format scanners plus the trace payload-redaction classifier); \
         new terms here are a gate regression"
    );
    let carved_paths: BTreeSet<&str> = PATH_TERM_COLLISIONS
        .iter()
        .map(|(fragment, _, _)| *fragment)
        .collect();
    for fragment in &carved_paths {
        assert!(
            !fragment.contains("composition") && !fragment.contains("product_surface"),
            "carve-outs must never cover the product assembly/workflow crates — those are \
             debt, not collisions: {fragment}"
        );
    }
    let root = workspace_root();
    let terms = derive_forbidden_terms(&inventory_dirs(&root));
    for compound in ["nearai-mcp", "nearai_mcp", "nearaimcp", "private.near.ai"] {
        assert!(
            terms.contains(compound),
            "compound extension forms must survive the carve-outs; missing `{compound}`"
        );
    }
}
