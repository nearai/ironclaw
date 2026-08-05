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
    // The retired-identity boot migration, deleted 2026-08-04 by owner ruling
    // (PROPOSAL §12.11 D-I: "just get rid of slack_bot and slack_personal
    // etc.. those can get nuked. Don't worry about migration data").
    //
    // The *identifiers* are banned, not the string `"slack_user"`: that literal
    // is still legitimate as a persisted id a deployed host may carry, as a
    // test fixture, and as a prefix of the live `slack_user_token` credential
    // handle. What must never come back is code that keys behaviour on it. The
    // behavioural half is
    // `restore_special_cases_no_extension_id_and_leaves_every_uncatalogued_row_intact`
    // (`ironclaw_extension_host/tests/lifecycle_restore_contract.rs`), which
    // fails if boot restore deletes the row; this pair fails earlier and more
    // cheaply, on the constant and the function themselves.
    "RETIRED_SLACK_USER_EXTENSION_ID",
    "remove_retired_internal_installation",
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
/// expected to bind. An *unreadable* path now fails the walk outright, so what
/// this floor is left guarding is the shape no I/O error can reveal: a `crates/`
/// that exists and holds a fraction of the tree — the partial family move —
/// where "no retired taxonomy found" is indistinguishable from "almost nothing
/// was looked at" (CHECKLIST WS0, #6963).
const MIN_SCANNED_FILES: usize = 500;

fn is_sanctioned(path: &str) -> bool {
    SANCTIONED_PATHS
        .iter()
        .any(|fragment| path.contains(fragment))
}

/// A scan error is a gate failure, not a skip: an unreadable directory or file
/// could hold a reintroduced term, and the floor cannot see one subtree going
/// missing from an otherwise healthy walk. Same shape as this gate's twin,
/// `reborn_memory_retired_vocabulary.rs`.
fn scan_dir(
    root: &Path,
    dir: &Path,
    hits: &mut Vec<String>,
    scanned: &mut Vec<String>,
) -> std::io::Result<()> {
    let entries = std::fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name == "target" || name == "node_modules" || name == ".git" {
                continue;
            }
            scan_dir(root, &path, hits, scanned)?;
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
        let contents = std::fs::read_to_string(&path)
            .map_err(|error| std::io::Error::new(error.kind(), format!("{relative}: {error}")))?;
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
    Ok(())
}

/// Every scanned path, plus the hits — so callers can ask whether the scan
/// measured anything instead of reading an empty hit list as a clean bill. An
/// I/O error propagates: the caller decides, and every production caller fails.
fn scan_workspace(root: &Path) -> std::io::Result<(Vec<String>, Vec<String>)> {
    let mut hits = Vec::new();
    let mut scanned = Vec::new();
    scan_dir(root, &root.join("crates"), &mut hits, &mut scanned)?;
    scan_dir(
        root,
        &root.join("tests/integration"),
        &mut hits,
        &mut scanned,
    )?;
    hits.sort();
    hits.dedup();
    Ok((hits, scanned))
}

/// Two distinct failures this gate could not previously report, both of which
/// produced an empty hit list indistinguishable from a clean scan.
///
/// A wrong root is now refused at the read, not merely caught by the floor: an
/// unreadable path (here, a `crates/` that is not there at all) fails instead of
/// contributing nothing. The floor's remaining job is the *partial* shape — a
/// `crates/` that exists and holds one family's worth of files, which is what a
/// staged family move produces and what no I/O error can reveal.
#[test]
fn a_wrong_root_is_refused_and_a_partial_tree_hits_the_floor() {
    let temp = tempfile::tempdir().expect("tempdir");
    let error = scan_workspace(temp.path()).expect_err("a missing scan root must refuse");
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::NotFound,
        "expected the read itself to refuse, got: {error}"
    );

    // One family's worth of source: real files, real directories, far too few.
    let root = temp.path();
    let family = root.join("crates/substrates/ironclaw_extension_registry/src");
    std::fs::create_dir_all(&family).expect("family tree");
    std::fs::create_dir_all(root.join("tests/integration")).expect("integration tree");
    for index in 0..10 {
        std::fs::write(family.join(format!("m{index}.rs")), "pub fn f() {}\n").expect("source");
    }

    let (hits, scanned) = scan_workspace(root).expect("a readable partial tree scans");
    assert_eq!(
        scanned.len(),
        10,
        "expected the partial tree, got {scanned:?}"
    );
    assert!(
        hits.is_empty(),
        "a partial scan also yields an empty hit list — that is the whole problem"
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
    let (_, scanned) =
        scan_workspace(&workspace_root()).expect("the workspace scan must not hit an I/O error");
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
    let (hits, scanned) =
        scan_workspace(&root).expect("the workspace scan must not hit an I/O error");
    assert!(
        scanned.len() >= MIN_SCANNED_FILES,
        "the taxonomy scan visited only {} file(s) under {} (floor {}). The walk is \
         broken, not the workspace — a partial tree yields an empty hit list that is \
         indistinguishable from a clean one, and no I/O error reveals it.",
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
