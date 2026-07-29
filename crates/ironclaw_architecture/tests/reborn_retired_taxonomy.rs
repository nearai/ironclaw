//! Zero-legacy gate for the NEA-25 unified extension model.
//!
//! The unified model retired an entire vocabulary: the connectable-channels
//! rail, the one-variant lifecycle surface kind, the conflated extension
//! `kind` wire string, the split `slack_bot` package identity, the
//! `slack_personal` provider id, and the contract-free manifest parse paths.
//! This test pins all of it at **zero occurrences** across Reborn code
//! (`crates/`, the WebUI frontend sources, and `tests/integration/`) so none
//! of it can be reintroduced silently.
//!
//! Sanctioned exceptions are path-scoped, not term-scoped:
//! - the two one-time forward data migrations legitimately name the retired
//!   identities they fold forward
//!   (`extension_host/extension_installation_store.rs`,
//!   `product_auth/durable/`);
//! - this test names every term on purpose.
//!
//! v1 (`src/`, root `tests/*.rs`) is out of scope: it is being strangled
//! wholesale, not policed term-by-term.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("architecture crate under crates")
        .to_path_buf()
}

/// Retired vocabulary. Every term here was deleted by the NEA-25 stack; a hit
/// outside the sanctioned paths is a regression, not a style issue.
const RETIRED_TERMS: &[&str] = &[
    // The connectable-channels rail (replaced by extension-surface discovery).
    "ConnectableChannelsProductService",
    "RebornConnectableChannelInfo",
    "RebornConnectableChannelListResponse",
    "list_connectable_channels",
    "listConnectableChannels",
    "SlackOperatorRouteVisibility",
    "channel_connection_service_slot",
    // The one-variant lifecycle surface enum (replaced by the shared
    // CapabilitySurfaceKind vocabulary in ironclaw_host_api).
    "LifecycleExtensionSurfaceKind",
    // The conflated extension `kind` wire string (replaced by
    // runtime + surfaces).
    "isChannelExtensionKind",
    "KIND_LABELS",
    "extension_kind(",
    "wire_kind(",
    // Slack-specific identity resolution (replaced by the generic
    // ProviderIdentityActorResolver parameterized by manifest data).
    "slack_actor_identity",
    "SlackUserIdentityActorResolver",
    // Contract-free / legacy manifest parse paths (one parse entry point
    // remains: ExtensionManifestV2::parse with a contract registry).
    "parse_with_host_api_contracts",
    "parse_with_optional_host_api_contracts",
    "from_toml_with_contracts",
    "LegacyTopLevelCapabilitiesForInstalledSource",
];

/// Retired identities that survive only as *substrings* of sanctioned names:
/// `slack_bot_token` / `slack_signing_secret` are workspace credential
/// handles, so the identity forms are matched exactly. The retired extension
/// `kind` wire VALUES are likewise matched as exact quoted strings: bare
/// `channel`/`mcp` remain legitimate vocabulary (the surface kind and the
/// runtime label), but nothing in Reborn code may compare against the old
/// conflated kind strings.
const RETIRED_IDENTITY_FORMS: &[&str] = &[
    "\"slack_bot\"",
    "'slack_bot'",
    "\"slack_personal\"",
    "'slack_personal'",
    "assets/slack_bot/",
    // The conflated extension `kind` wire values (replaced by
    // runtime + surfaces).
    "\"wasm_channel\"",
    "'wasm_channel'",
    "\"channel_relay\"",
    "'channel_relay'",
    "\"mcp_server\"",
    "'mcp_server'",
];

/// Path fragments allowed to reference retired vocabulary.
const SANCTIONED_PATHS: &[&str] = &[
    // v1 → Reborn converter reads v1 domain names by design.
    // The v1 gateway is a legacy enclave being strangled wholesale — its
    // static JS still serves the v1 `kind` wire and is not policed
    // term-by-term (same footing as `src/`).
    "crates/ironclaw_gateway/",
    // One-time forward data migrations name what they fold forward.
    "extension_host/extension_installation_store.rs",
    "product_auth/durable/",
    // This gate names every term on purpose.
    "reborn_retired_taxonomy.rs",
];

fn is_sanctioned(path: &str) -> bool {
    SANCTIONED_PATHS
        .iter()
        .any(|fragment| path.contains(fragment))
}

fn scan_dir(root: &Path, dir: &Path, hits: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name == "target" || name == "node_modules" || name == ".git" {
                continue;
            }
            scan_dir(root, &path, hits);
            continue;
        }
        let is_rust = name.ends_with(".rs");
        let is_frontend = name.ends_with(".ts")
            || name.ends_with(".tsx")
            || name.ends_with(".mts")
            || name.ends_with(".mjs")
            || name.ends_with(".js");
        let is_manifest = name.ends_with(".toml");
        if !(is_rust || is_frontend || is_manifest) {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if is_sanctioned(&relative) {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        for term in RETIRED_TERMS {
            if contents.contains(term) {
                hits.push(format!("{relative}: `{term}`"));
            }
        }
        for form in RETIRED_IDENTITY_FORMS {
            if contents.contains(form) {
                hits.push(format!("{relative}: `{form}`"));
            }
        }
    }
}

#[test]
fn reborn_code_never_references_retired_taxonomy() {
    let root = workspace_root();
    let mut hits = Vec::new();
    scan_dir(&root, &root.join("crates"), &mut hits);
    scan_dir(&root, &root.join("tests/integration"), &mut hits);
    hits.sort();
    hits.dedup();
    assert!(
        hits.is_empty(),
        "retired NEA-25 taxonomy reintroduced (extension = the product object; \
         channel = a capability surface; runtime is implementation only):\n{}",
        hits.join("\n")
    );
}
