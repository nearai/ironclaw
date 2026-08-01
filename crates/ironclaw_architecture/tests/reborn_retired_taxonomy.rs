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
//! - the one-time forward data migration under `product_auth/durable/`
//!   legitimately names the retired identities it folds forward;
//! - this test names every term on purpose.
//!
//! The list is shrink-only and pinned to reality — see `SANCTIONED_PATHS`.

#[allow(dead_code)]
mod ratchet_support;

use std::path::Path;

use ratchet_support::workspace_root;

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
///
/// Shrink-only, and pinned to reality: `sanctioned_paths_all_match_real_files`
/// fails when a fragment matches no scanned file, so an exemption cannot outlive
/// the code it exempts. Two entries had already rotted when that check was added
/// — `crates/ironclaw_gateway/` (crate deleted) and
/// `extension_host/extension_installation_store.rs` (deleted by #6430) — and
/// neither was visible, because a fragment that matches nothing simply never
/// sanctions anything.
const SANCTIONED_PATHS: &[&str] = &[
    // One-time forward data migrations name what they fold forward.
    "product_auth/durable/",
    // This gate names every term on purpose.
    "reborn_retired_taxonomy.rs",
];

/// Sanity floor for the scan. Far below the real count (~1500) and never
/// expected to bind. It exists because the walk swallows `read_dir` failures:
/// with a wrong root every directory read fails, the hit list comes back empty,
/// and "no retired taxonomy found" is indistinguishable from "nothing was
/// looked at" (CHECKLIST WS0, #6963).
const MIN_SCANNED_FILES: usize = 500;

fn is_sanctioned(path: &str) -> bool {
    SANCTIONED_PATHS
        .iter()
        .any(|fragment| path.contains(fragment))
}

fn scan_dir(root: &Path, dir: &Path, hits: &mut Vec<String>, scanned: &mut Vec<String>) {
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
            scan_dir(root, &path, hits, scanned);
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
        scanned.push(relative.clone());
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

/// Every scanned path, plus the hits — so callers can ask whether the scan
/// measured anything instead of reading an empty hit list as a clean bill.
fn scan_workspace(root: &Path) -> (Vec<String>, Vec<String>) {
    let mut hits = Vec::new();
    let mut scanned = Vec::new();
    scan_dir(root, &root.join("crates"), &mut hits, &mut scanned);
    scan_dir(
        root,
        &root.join("tests/integration"),
        &mut hits,
        &mut scanned,
    );
    hits.sort();
    hits.dedup();
    (hits, scanned)
}

/// The failure this gate could not previously report: a root with no `crates/`
/// yields an empty hit list, which without the floor reads exactly like a clean
/// scan. Pins that the scan really does come back empty there, so the floor —
/// not luck — is what turns it into a failure.
#[test]
fn a_wrong_root_scans_nothing_and_the_floor_catches_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (hits, scanned) = scan_workspace(temp.path());

    assert!(
        scanned.is_empty(),
        "expected an empty scan, got {scanned:?}"
    );
    assert!(
        hits.is_empty(),
        "an empty scan also yields an empty hit list — that is the whole problem"
    );
    assert!(
        scanned.len() < MIN_SCANNED_FILES,
        "the floor must reject this scan"
    );
}

/// An exemption that matches nothing exempts nothing — it is dead text that
/// reads as policy. Both entries this check retired had outlived their code.
#[test]
fn sanctioned_paths_all_match_real_files() {
    let (_, scanned) = scan_workspace(&workspace_root());
    let stale: Vec<&str> = SANCTIONED_PATHS
        .iter()
        .copied()
        .filter(|fragment| !scanned.iter().any(|path| path.contains(fragment)))
        .collect();

    assert!(
        stale.is_empty(),
        "SANCTIONED_PATHS entries match no scanned file — delete them; this list is \
         shrink-only and an entry that sanctions nothing is dead text that reads as \
         policy: {stale:?}"
    );
}

#[test]
fn reborn_code_never_references_retired_taxonomy() {
    let root = workspace_root();
    let (hits, scanned) = scan_workspace(&root);
    assert!(
        scanned.len() >= MIN_SCANNED_FILES,
        "the taxonomy scan visited only {} file(s) under {} (floor {}). The walk is \
         broken, not the workspace — `scan_dir` swallows read_dir failures, so an \
         empty hit list from an unvisited tree is indistinguishable from a clean one.",
        scanned.len(),
        root.display(),
        MIN_SCANNED_FILES
    );
    assert!(
        hits.is_empty(),
        "retired NEA-25 taxonomy reintroduced (extension = the product object; \
         channel = a capability surface; runtime is implementation only):\n{}",
        hits.join("\n")
    );
}
