//! Concrete-name specificity gate for the unified extension runtime.
// arch-exempt: large_file, unified extension specificity ratchet stays centralized, plan #6175
//!
//! Goal (docs/reborn/extension-runtime/overview.md §1): no generic crate
//! contains a concrete product name, vendor id, or vendor API host. The
//! forbidden vocabulary is **derived from the bundled package inventory**
//! (`crates/ironclaw_first_party_extensions/assets/` plus the test fixture
//! inventory `tests/fixtures/extensions/`), so a future `discord` package is
//! caught without editing this scanner (checklist TEST-6).
//!
//! Scope: the `src/` tree and `Cargo.toml` of every generic Reborn workspace
//! crate, plus the WebUI frontend sources. Excluded, by category:
//!
//! - **Concrete extension crates** (`CONCRETE_EXTENSION_CRATES`) — they *are*
//!   the product code.
//! - **The package inventory crate** (`ironclaw_first_party_extensions`) —
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

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("architecture crate under crates")
        .to_path_buf()
}

// ---------------------------------------------------------------------------
// Inventory-derived forbidden terms
// ---------------------------------------------------------------------------

/// Directories whose `*/manifest.toml` files form the package inventory the
/// forbidden vocabulary derives from.
fn inventory_dirs(root: &Path) -> Vec<PathBuf> {
    vec![
        root.join("crates/ironclaw_first_party_extensions/assets"),
        root.join("tests/fixtures/extensions"),
    ]
}

/// Bare terms carved out of the derived set because they collide with a
/// non-extension taxonomy that legitimately lives in generic crates. Compound
/// forms derived from the same package (directory-name variants, specific
/// hosts) remain scanned.
const TERM_COLLISIONS: &[(&str, &str)] = &[
    (
        "nearai",
        "`nearai` is also the assistant's LLM backend id (crates/ironclaw_llm, \
         crates/ironclaw_reborn_config); the extension forms `nearai-mcp`/`nearai_mcp` \
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
    (
        "crates/ironclaw_host_runtime/src/first_party_tools/skill_url_install",
        "github",
        "skill installation from GitHub repositories — GitHub as a code host, not the \
         github extension",
    ),
    (
        "crates/ironclaw_host_runtime/src/first_party_tools/skill_url_install.rs",
        "api.github.com",
        "GitHub content API allowlist for skill installation",
    ),
    (
        "crates/ironclaw_host_runtime/src/first_party_tools/skill_management.rs",
        "github",
        "skill-install tool description names GitHub as a skill source",
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
        "crates/ironclaw_reborn_config/src/secrets_guard.rs",
        "github",
        "vendor token-format inline-secret rejection",
    ),
    (
        "crates/ironclaw_reborn_config/src/secrets_guard.rs",
        "google",
        "vendor token-format inline-secret rejection",
    ),
    (
        "crates/ironclaw_reborn_config/src/secrets_guard.rs",
        "slack",
        "vendor token-format inline-secret rejection",
    ),
    (
        "crates/ironclaw_events/src/runtime_event.rs",
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
        "crates/ironclaw_turns/src/run_profile/host/validate.rs",
        "github",
        "credential-prefix redaction (github_pat_) — relocated here when \
         run_profile/host.rs was decomposed (#6391)",
    ),
    (
        "crates/ironclaw_turns/src/run_profile/host/validate.rs",
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
        "crates/ironclaw_turns/src/run_profile/prompt_text.rs",
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
        "crates/ironclaw_reborn_identity/src/filesystem_store.rs",
        "slack",
        "credential-authority ProviderKind vocabulary (persisted identity keys), not the extensions vendor",
    ),
    (
        "crates/ironclaw_reborn_identity/src/key.rs",
        "github",
        "credential-authority ProviderKind vocabulary (persisted identity keys), not the extensions vendor",
    ),
    (
        "crates/ironclaw_reborn_identity/src/key.rs",
        "google",
        "credential-authority ProviderKind vocabulary (persisted identity keys), not the extensions vendor",
    ),
    (
        "crates/ironclaw_reborn_identity/src/key.rs",
        "slack",
        "credential-authority ProviderKind vocabulary (persisted identity keys), not the extensions vendor",
    ),
    (
        "crates/ironclaw_reborn_identity/src/lib.rs",
        "github",
        "credential-authority ProviderKind vocabulary (persisted identity keys), not the extensions vendor",
    ),
    (
        "crates/ironclaw_reborn_identity/src/lib.rs",
        "google",
        "credential-authority ProviderKind vocabulary (persisted identity keys), not the extensions vendor",
    ),
    (
        "crates/ironclaw_reborn_identity/src/lib.rs",
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
        "crates/ironclaw_common/src/platform.rs",
        "telegram",
        "persisted credential-name / channel-id vocabulary in the shared identity crate (compat law: stored ids stay readable)",
    ),
    (
        "crates/ironclaw_reborn_identity/src/key.rs",
        "telegram",
        "credential-authority ProviderKind vocabulary (persisted identity keys), not the extensions vendor",
    ),
    (
        "crates/ironclaw_reborn_identity/src/lib.rs",
        "telegram",
        "credential-authority ProviderKind vocabulary (persisted identity keys), not the extensions vendor",
    ),
    (
        "crates/ironclaw_safety/src/leak_detector.rs",
        "telegram",
        "vendor token-format leak detection (telegram_bot_token pattern)",
    ),
    // Trace payload-redaction / side-effect safety classifier
    // (`ironclaw_reborn_traces`): `tool_payload_profile` selects the
    // payload-redaction profile and `classify_tool_side_effect` the
    // external-write side-effect off tool-name keywords. The vendor keywords
    // are a safety DENYLIST — a superset of the bundled inventory that must
    // also cover non-package messaging/issue-tracker tools (signal, discord,
    // gitlab) — so sourcing the set from the inventory would drop those and
    // weaken redaction. Not extension routing (the classifier has only the
    // tool-name string at trace-analysis time). Pinned by
    // `tool_payload_redaction_profile_is_a_safety_denylist_not_inventory_routing`.
    (
        "crates/ironclaw_reborn_traces/src/contribution.rs",
        "slack",
        "trace payload-redaction/side-effect safety classifier keyed off tool-name \
         keywords (messaging profile + external-write detection); a safety denylist, \
         not extension routing",
    ),
    (
        "crates/ironclaw_reborn_traces/src/contribution.rs",
        "telegram",
        "trace payload-redaction safety classifier keyed off tool-name keywords \
         (messaging profile); a safety denylist, not extension routing",
    ),
    (
        "crates/ironclaw_reborn_traces/src/contribution.rs",
        "gmail",
        "trace payload-redaction safety classifier keyed off tool-name keywords \
         (email profile); a safety denylist, not extension routing",
    ),
    (
        "crates/ironclaw_reborn_traces/src/contribution.rs",
        "github",
        "trace payload-redaction safety classifier keyed off tool-name keywords \
         (issue-tracker profile); a safety denylist, not extension routing",
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
        "crates/ironclaw_webui/frontend/src/pages/chat/lib/gates.ts",
        "github",
        "WebUI browser-login SSO provider button/route, not the extensions vendor",
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
const CONCRETE_EXTENSION_CRATES: &[&str] = &[
    "ironclaw_slack_extension",
    "ironclaw_telegram_extension",
    "ironclaw_telegram_v2_adapter",
];

/// Generic-side crates excluded from the scan for a documented structural
/// reason (see the module header).
const SANCTIONED_SCAN_EXEMPT_CRATES: &[&str] = &[
    // The package inventory + native executors for bundled extensions.
    "ironclaw_first_party_extensions",
    // The binary assembles the native factory registry (overview §4.0).
    "ironclaw",
    // The post-Tier-B workspace root: a test-only host for the Reborn
    // integration [[test]] suite (no lib/bin). Tests may name concrete
    // products (module header), and its manifest's workspace `exclude`
    // list names the WASM source dirs by path — assembly, not generic code.
    "ironclaw_reborn_integration_tests",
    // One-time forward migrations name what they fold forward.
    // This crate's tests name every term on purpose.
    "ironclaw_architecture",
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

/// Remove `#[cfg(test)]` items (inline `mod tests { … }` blocks and
/// `mod tests;` declarations) before matching: tests may name concrete
/// products (overview §8). Line-based brace counting — the same heuristic
/// `scripts/pre-commit-safety.sh` uses for its test-stripping.
fn strip_cfg_test_blocks(source: &str) -> String {
    let mut kept = String::with_capacity(source.len());
    let mut lines = source.lines().peekable();
    while let Some(line) = lines.next() {
        if !line.trim_start().starts_with("#[cfg(test)]") {
            kept.push_str(line);
            kept.push('\n');
            continue;
        }
        // Skip attribute lines, then the annotated item.
        let mut depth: i64 = 0;
        let mut opened = false;
        for skipped in lines.by_ref() {
            let trimmed = skipped.trim_start();
            if !opened && trimmed.starts_with("#[") {
                continue;
            }
            depth += skipped.matches('{').count() as i64;
            depth -= skipped.matches('}').count() as i64;
            if !opened {
                if skipped.contains('{') {
                    opened = true;
                } else if trimmed.ends_with(';') {
                    // `mod tests;` — single-line item, nothing else to skip.
                    break;
                }
            }
            if opened && depth <= 0 {
                break;
            }
        }
    }
    kept
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
        FileKind::Rust => strip_cfg_test_blocks(&contents),
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
    let frontend = root.join("crates/ironclaw_webui/frontend/src");
    scan_dir(root, &frontend, terms, &mut hits);

    hits.into_iter()
        .filter(|(path, _)| {
            !SANCTIONED_PATHS
                .iter()
                .any(|fragment| path.contains(fragment))
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
    hits: BTreeSet<(String, String)>,
) -> (BTreeSet<(String, String)>, Vec<String>) {
    let mut used: BTreeSet<(&str, &str)> = BTreeSet::new();
    let policed: BTreeSet<(String, String)> = hits
        .into_iter()
        .filter(|(path, term)| {
            let carved = PATH_TERM_COLLISIONS
                .iter()
                .find(|(fragment, carved_term, _)| path.contains(fragment) && term == carved_term);
            if let Some((fragment, carved_term, _)) = carved {
                used.insert((fragment, carved_term));
                return false;
            }
            true
        })
        .collect();
    let stale = PATH_TERM_COLLISIONS
        .iter()
        .filter(|(fragment, term, _)| !used.contains(&(fragment, term)))
        .map(|(fragment, term, _)| format!("{fragment} :: {term}"))
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
    (
        "crates/ironclaw_product_adapter_registry/src/lib.rs",
        "github",
    ),
    (
        "crates/ironclaw_product_adapter_registry/src/lib.rs",
        "slack",
    ),
    (
        "crates/ironclaw_product_adapter_registry/src/lib.rs",
        "telegram",
    ),
    ("crates/ironclaw_product_adapters/src/identity.rs", "slack"),
    (
        "crates/ironclaw_product_adapters/src/identity.rs",
        "telegram",
    ),
    ("crates/ironclaw_product_adapters/src/outbound.rs", "github"),
    ("crates/ironclaw_product_adapters/src/outbound.rs", "google"),
    ("crates/ironclaw_product_adapters/src/outbound.rs", "notion"),
    ("crates/ironclaw_product_adapters/src/outbound.rs", "slack"),
    (
        "crates/ironclaw_product_adapters/src/outbound.rs",
        "telegram",
    ),
    ("crates/ironclaw_product_workflow/Cargo.toml", "telegram"),
    (
        "crates/ironclaw_product_workflow/src/conversation_binding.rs",
        "slack",
    ),
    ("crates/ironclaw_product_workflow/src/lib.rs", "telegram"),
    (
        "crates/ironclaw_product_workflow/src/reborn_services.rs",
        "slack",
    ),
    (
        "crates/ironclaw_product_workflow/src/reborn_services/llm_config.rs",
        "github",
    ),
    (
        "crates/ironclaw_product_workflow/src/reborn_services/llm_config.rs",
        "google",
    ),
    (
        "crates/ironclaw_product_workflow/src/reborn_services/projects.rs",
        "github",
    ),
    ("crates/ironclaw_product_workflow/src/workflow.rs", "slack"),
    ("crates/ironclaw_host_api/src/dispatch.rs", "slack"),
    (
        "crates/ironclaw_host_runtime/src/first_party_tools/schemas.rs",
        "slack",
    ),
    (
        "crates/ironclaw_host_runtime/src/services/wasm_execution.rs",
        "slack",
    ),
    ("crates/ironclaw_runner/src/loop_driver_host.rs", "slack"),
    ("crates/ironclaw_runner/src/tool_disclosure.rs", "google"),
    (
        "crates/ironclaw_runner/src/tool_disclosure.rs",
        "google-calendar",
    ),
    (
        "crates/ironclaw_runner/src/tool_disclosure.rs",
        "web-access",
    ),
    (
        "crates/ironclaw_runner/src/tool_disclosure_port.rs",
        "google",
    ),
    (
        "crates/ironclaw_runner/src/tool_disclosure_port.rs",
        "google-calendar",
    ),
    (
        "crates/ironclaw_reborn_composition/src/extension_host/available_extensions.rs",
        "google",
    ),
    (
        "crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle.rs",
        "slack",
    ),
    (
        "crates/ironclaw_reborn_composition/src/extension_host/gsuite.rs",
        "google",
    ),
    (
        "crates/ironclaw_reborn_composition/src/projection/display_preview.rs",
        "web-access",
    ),
    (
        "crates/ironclaw_reborn_composition/src/runtime/local_dev/extension_surface.rs",
        "web_access",
    ),
    // lane-4: nearai-slice — the last catalog package (nearai_mcp) still assembled in composition because [mcp].server is patched from llm_admin config; DEFERRED — finish per the handoff (move static data to first_party_extensions::packages::nearai_mcp, inject the URL through the with_channel_extension_bindings-style seam)
    (
        "crates/ironclaw_reborn_composition/src/extension_host/available_extensions.rs",
        "nearai-mcp",
    ),
    (
        "crates/ironclaw_reborn_composition/src/extension_host/available_extensions.rs",
        "nearai_mcp",
    ),
    (
        "crates/ironclaw_reborn_composition/src/extension_host/available_extensions.rs",
        "nearaimcp",
    ),
    (
        "crates/ironclaw_reborn_composition/src/factory.rs",
        "nearai_mcp",
    ),
    (
        "crates/ironclaw_reborn_composition/src/input.rs",
        "nearai_mcp",
    ),
    (
        "crates/ironclaw_reborn_composition/src/input.rs",
        "nearaimcp",
    ),
    (
        "crates/ironclaw_reborn_composition/src/lib.rs",
        "nearai_mcp",
    ),
    ("crates/ironclaw_reborn_composition/src/lib.rs", "nearaimcp"),
    (
        "crates/ironclaw_reborn_composition/src/llm_admin/mod.rs",
        "nearai_mcp",
    ),
    (
        "crates/ironclaw_reborn_composition/src/llm_admin/nearai_mcp.rs",
        "nearai_mcp",
    ),
    (
        "crates/ironclaw_reborn_composition/src/llm_admin/nearai_mcp.rs",
        "nearaimcp",
    ),
    (
        "crates/ironclaw_reborn_composition/src/product_auth/api/auth.rs",
        "nearai_mcp",
    ),
    (
        "crates/ironclaw_reborn_composition/src/runtime.rs",
        "nearai_mcp",
    ),
    // lane-4: migration — one-time forward-migration call sites naming the v1 vocabulary they fold forward — correct-by-design (same pattern the retired-taxonomy gate sanctions); would become a SANCTIONED_PATHS carve if the sites move into a dedicated migration module
    ("crates/ironclaw_reborn_composition/src/factory.rs", "slack"),
    // lane-4: web-access-mod — the web-access first-party handler assembly module + its registration — extension-specific host wiring pending a generic first-party-handler seam keyed by manifest service
    (
        "crates/ironclaw_reborn_composition/src/factory.rs",
        "web_access",
    ),
    (
        "crates/ironclaw_reborn_composition/src/lib.rs",
        "web_access",
    ),
    (
        "crates/ironclaw_reborn_composition/src/web_access.rs",
        "web_access",
    ),
    (
        "crates/ironclaw_reborn_composition/src/web_access.rs",
        "webaccess",
    ),
    // lane-4: doc-str — incidental doc-comment / error-string / tool-description examples that NAME an extension but branch on nothing — the code routes by a manifest field (display_name/provider/effects); reword or leave (Ben's call)
    ("crates/ironclaw_filesystem/src/index.rs", "acme"),
    ("crates/ironclaw_host_api/src/capability.rs", "slack"),
    ("crates/ironclaw_host_api/src/http.rs", "slack"),
    ("crates/ironclaw_host_api/src/ids.rs", "github"),
    ("crates/ironclaw_host_api/src/surface.rs", "slack"),
    ("crates/ironclaw_loop_host/src/capability_port.rs", "gmail"),
    ("crates/ironclaw_auth/src/loopback_oauth.rs", "google"),
    ("crates/ironclaw_auth/src/loopback_oauth.rs", "notion"),
    (
        "crates/ironclaw_outbound/src/delivered_gate_routes.rs",
        "slack",
    ),
    ("crates/ironclaw_projects/src/lib.rs", "github"),
    (
        "crates/ironclaw_reborn_composition/src/automation/trigger_poller.rs",
        "slack",
    ),
    (
        "crates/ironclaw_reborn_composition/src/blocked_auth_resume.rs",
        "google",
    ),
    (
        "crates/ironclaw_reborn_composition/src/blocked_auth_resume.rs",
        "slack",
    ),
    (
        "crates/ironclaw_reborn_composition/src/extension_host/extension_lifecycle.rs",
        "github",
    ),
    (
        "crates/ironclaw_reborn_composition/src/factory.rs",
        "google",
    ),
    (
        "crates/ironclaw_reborn_composition/src/factory.rs",
        "notion",
    ),
    (
        "crates/ironclaw_reborn_composition/src/google_oauth_secret_store.rs",
        "google",
    ),
    ("crates/ironclaw_reborn_composition/src/lib.rs", "google"),
    ("crates/ironclaw_reborn_composition/src/lib.rs", "slack"),
    (
        "crates/ironclaw_reborn_composition/src/llm_admin/nearai_login_serve.rs",
        "github",
    ),
    (
        "crates/ironclaw_reborn_composition/src/llm_admin/nearai_login_serve.rs",
        "google",
    ),
    (
        "crates/ironclaw_product_workflow/src/reborn_services/outbound_delivery_capability_surface.rs",
        "slack",
    ),
    (
        "crates/ironclaw_reborn_composition/src/product_auth/api/auth.rs",
        "slack",
    ),
    (
        "crates/ironclaw_reborn_composition/src/product_auth/credentials/runtime_credentials.rs",
        "google",
    ),
    (
        "crates/ironclaw_reborn_composition/src/product_auth/serve/mod.rs",
        "slack",
    ),
    (
        "crates/ironclaw_reborn_composition/src/projection.rs",
        "slack",
    ),
    (
        "crates/ironclaw_reborn_composition/src/projection/turn_events.rs",
        "slack",
    ),
    (
        "crates/ironclaw_reborn_composition/src/root/communication_context.rs",
        "slack",
    ),
    ("crates/ironclaw_skills/src/selector.rs", "github"),
    ("crates/ironclaw_skills/src/types.rs", "github"),
    ("crates/ironclaw_skills/src/types.rs", "google"),
    ("crates/ironclaw_skills/src/types.rs", "slack"),
    (
        "crates/ironclaw_reborn_config/src/capability_remediation.rs",
        "gmail",
    ),
    (
        "crates/ironclaw_reborn_config/src/capability_remediation.rs",
        "google",
    ),
    ("crates/ironclaw_reborn_config/src/config_file.rs", "gmail"),
    ("crates/ironclaw_reborn_config/src/config_file.rs", "google"),
    ("crates/ironclaw_reborn_config/src/config_file.rs", "slack"),
    (
        "crates/ironclaw_reborn_config/src/config_file.rs",
        "telegram",
    ),
    ("crates/ironclaw_reborn_config/src/lib.rs", "google"),
    ("crates/ironclaw_reborn_config/src/lib.rs", "slack"),
    ("crates/ironclaw_reborn_config/src/lib.rs", "telegram"),
    // lane-4: dev-dep — the sanctioned DEL-7 dev-dependency on the concrete slack crate (test linkage only); the scanner sees the crate name in Cargo.toml
    ("crates/ironclaw_reborn_composition/Cargo.toml", "slack"),
    // lane-4: branch — the provider catalog names github_copilot (an LLM
    // provider id, not the github extension); degenericize with the catalog
    // slice or carve under an LLM-provider path outside composition.
    (
        "crates/ironclaw_reborn_composition/src/llm_admin/provider_admin.rs",
        "github",
    ),
    // lane-4: doc — NEAR AI login copy names its upstream SSO providers.
    (
        "crates/ironclaw_reborn_composition/src/llm_admin/nearai_login_serve.rs",
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
    (
        "crates/ironclaw_webui/frontend/src/pages/chat/chat.tsx",
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

/// One `(relative path, matched term)` scanner hit.
type HitEntry = (String, String);

fn classify_hits(
    hits: &BTreeSet<HitEntry>,
    allowlist: &[(&str, &str)],
) -> (Vec<HitEntry>, Vec<HitEntry>) {
    let allowed: BTreeSet<HitEntry> = allowlist
        .iter()
        .map(|(path, term)| (path.to_string(), term.to_string()))
        .collect();
    let new_violations = hits.difference(&allowed).cloned().collect();
    let stale_entries = allowed.difference(hits).cloned().collect();
    (new_violations, stale_entries)
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

#[test]
fn reborn_generic_code_names_no_concrete_extension() {
    let root = workspace_root();
    let terms = derive_forbidden_terms(&inventory_dirs(&root));
    assert!(
        !terms.is_empty(),
        "inventory-derived term set must not be empty — is the package inventory readable?"
    );
    let raw_hits = collect_workspace_hits(&root, &terms);
    let (hits, stale_carve_outs) = apply_path_term_collisions(raw_hits);
    let (new_violations, stale_entries) = classify_hits(&hits, ALLOWLIST);

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
             debt only — add the exact entries below to ALLOWLIST):\n{}",
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
    for concrete in CONCRETE_EXTENSION_CRATES {
        let manifest = root.join("crates").join(concrete).join("Cargo.toml");
        if manifest.exists() {
            assert!(
                registered.contains(concrete),
                "{concrete} has a Cargo.toml but is not in cargo metadata; register it as a \
                 workspace member so this gate actually checks its dependents"
            );
        }
    }

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
    let (new_violations, stale_entries) = classify_hits(&hits, &allowlist);
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
            !fragment.contains("composition") && !fragment.contains("product_workflow"),
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
