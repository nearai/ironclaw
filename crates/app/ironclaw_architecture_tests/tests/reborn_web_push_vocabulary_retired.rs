//! Zero-occurrence gate for the retired `web-push` channel identity
//! (unified channel model §13).
//!
//! The 2026-08-10 unified-channel-model train renamed the browser channel's
//! product identity `web-push` → `web-app`: the extension id, channel name,
//! catalog target id, package directory, and crate names all moved, and the
//! bespoke `/web-push/*` enrollment routes were deleted in favor of the
//! generic `/channels/{extension_id}/notifications` surface. This gate pins
//! the retired spelling at **zero occurrences** across Reborn code, the WebUI
//! frontend sources, packaged manifests, integration tests, and the embedded
//! `skills/` bundles — the same footing as `reborn_retired_taxonomy.rs` — so
//! neither the old identity nor a channel-specific route can be reintroduced
//! silently.
//!
//! What is *not* banned: the space-separated protocol prose "Web Push" /
//! "web push" (RFC 8030/8291/8292 is genuinely the Web Push protocol, and the
//! channel's package and domain crate speak it), and separator-free fixture
//! strings such as `webui-webpush-tenant` in tests. The ban is on the retired
//! *identifiers*: `web-push`, `web_push`, `WebPush`, `webPush`, `WEB_PUSH`.
//!
//! Sanctioned exceptions are path-scoped and each names the exact terms it
//! may carry. All but one exist because a PERSISTED coordinate deliberately
//! keeps the pre-rename bytes — renaming a storage key is data loss, not a
//! spelling fix:
//! - the `ironclaw_web_app` grammar decodes (never mints) the legacy
//!   `web-push/v1/` binding-ref prefix and keeps the `web_push_vapid`
//!   credential-handle value;
//! - its store keeps the `/web-push/subscriptions.json` document path;
//! - composition keeps the `/web-push` per-user mount alias those documents
//!   physically live under, while binary/test bindings carry that opaque
//!   pre-generic document address into composition;
//! - the web-app manifest names the same persisted credential handle;
//! - the specificity gate's carve-out doc records the rename;
//! - this gate names every term on purpose.

#[allow(dead_code)]
mod ratchet_support;

use std::path::Path;

use ratchet_support::workspace_root;

/// The retired identity spellings. A hit outside the sanctioned paths is a
/// regression, not a style issue.
const RETIRED_TERMS: &[&str] = &["web-push", "web_push", "WebPush", "webPush", "WEB_PUSH"];

/// Path fragments allowed to reference the retired spelling, each with the
/// exact terms it may use. An empty term list means "every term" and is
/// reserved for this gate itself. Shrink-only, and pinned to reality:
/// [`sanctioned_paths_all_match_real_files`] fails when a fragment matches no
/// scanned file, so an exemption cannot outlive the code it exempts.
const SANCTIONED_PATHS: &[(&str, &[&str])] = &[
    // Persisted coordinates: legacy ref-prefix decode + credential-handle
    // value (grammar), document path (store).
    (
        "crates/domains/ironclaw_web_app/src/grammar.rs",
        &["web-push", "web_push"],
    ),
    (
        "crates/domains/ironclaw_web_app/src/store.rs",
        &["web-push"],
    ),
    // RFC 8291's key-derivation info string is the protocol-fixed literal
    // `WebPush: info` — not a channel name; changing it breaks decryption.
    (
        "crates/domains/ironclaw_web_app/src/crypto.rs",
        &["WebPush"],
    ),
    // The per-user mount alias the enrollment documents physically live
    // under — it resolves to a physical subpath, so it keeps its spelling.
    ("crates/app/ironclaw_composition/src/lib.rs", &["web-push"]),
    (
        "crates/app/ironclaw_cli/src/runtime/native_extensions.rs",
        &["web-push"],
    ),
    (
        "tests/integration/support/harness/options.rs",
        &["web-push"],
    ),
    // The persisted credential handle in the channel's own manifest, and the
    // package README that documents that value for a reader who greps for it.
    (
        "crates/extensions/packages/web-app/manifest.toml",
        &["web_push"],
    ),
    (
        "crates/extensions/packages/web-app/README.md",
        &["web_push"],
    ),
    // The specificity gate's carve-out doc records the rename and names this
    // gate's file (which contains `web_push`).
    ("reborn_extension_specificity.rs", &["web-push", "web_push"]),
    // The integration suites assert the PERSISTED catalog target id, whose
    // value deliberately keeps the pre-rename spelling (see
    // `ironclaw_web_app::WEB_APP_TARGET_ID`). Pinning it is the point: it is
    // what proves a stored notification-channel selection survives the
    // rename, so these must spell it.
    ("tests/integration/webui_v2_product_api.rs", &["web-push"]),
    ("tests/integration/delivery_user_journeys.rs", &["web-push"]),
    // This gate names every term on purpose.
    ("reborn_web_push_vocabulary_retired.rs", &[]),
];

/// Sanity floor: far below the real scanned-file count, guarding only the
/// partial-tree shape where "no retired spelling found" would be
/// indistinguishable from "almost nothing was looked at".
const MIN_SCANNED_FILES: usize = 500;

fn sanctioned_terms(path: &str) -> Option<&'static [&'static str]> {
    SANCTIONED_PATHS
        .iter()
        .find(|(fragment, _)| path.contains(fragment))
        .map(|(_, terms)| *terms)
}

fn is_sanctioned(sanctioned: Option<&'static [&'static str]>, term: &str) -> bool {
    match sanctioned {
        None => false,
        Some([]) => true,
        Some(terms) => terms.contains(&term),
    }
}

/// A scan error is a gate failure, not a skip — same shape as
/// `reborn_retired_taxonomy.rs`. `dist/` is skipped beside `target/`: it is
/// git-ignored Vite build output, rebuilt from the scanned sources.
fn scan_file(
    root: &Path,
    path: &Path,
    hits: &mut Vec<String>,
    scanned: &mut Vec<String>,
) -> std::io::Result<()> {
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    scanned.push(relative.clone());
    let sanctioned = sanctioned_terms(&relative);
    let contents = std::fs::read_to_string(path)
        .map_err(|error| std::io::Error::new(error.kind(), format!("{relative}: {error}")))?;
    for term in RETIRED_TERMS {
        if contents.contains(term) && !is_sanctioned(sanctioned, term) {
            hits.push(format!("{relative}: `{term}`"));
        }
    }
    Ok(())
}

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
            if name == "target" || name == "node_modules" || name == ".git" || name == "dist" {
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
        let is_guidance = name.ends_with(".json") || name.ends_with(".md");
        let is_python = name.ends_with(".py");
        if !(is_rust || is_frontend || is_manifest || is_guidance || is_python) {
            continue;
        }
        scan_file(root, &path, hits, scanned)?;
    }
    Ok(())
}

fn scan_workspace(root: &Path) -> std::io::Result<(Vec<String>, Vec<String>)> {
    let mut hits = Vec::new();
    let mut scanned = Vec::new();
    scan_file(root, &root.join("Cargo.toml"), &mut hits, &mut scanned)?;
    scan_dir(root, &root.join("crates"), &mut hits, &mut scanned)?;
    scan_dir(
        root,
        &root.join("tests/integration"),
        &mut hits,
        &mut scanned,
    )?;
    scan_dir(root, &root.join("tests/e2e"), &mut hits, &mut scanned)?;
    scan_dir(root, &root.join("skills"), &mut hits, &mut scanned)?;
    hits.sort();
    hits.dedup();
    Ok((hits, scanned))
}

#[test]
fn retired_web_push_spelling_stays_at_zero_occurrences() {
    let root = workspace_root();
    let (hits, scanned) = scan_workspace(&root).expect("workspace scan must complete");
    assert!(
        scanned.len() >= MIN_SCANNED_FILES,
        "scan covered only {} files (< {MIN_SCANNED_FILES}) — the walk is broken, so an empty \
         hit list proves nothing",
        scanned.len(),
    );
    assert!(
        hits.is_empty(),
        "retired `web-push` identity spelling found outside the sanctioned persisted-compat \
         paths — the channel is `web-app` now, and generic code names no channel at all:\n  {}",
        hits.join("\n  "),
    );
}

#[test]
fn retired_vocabulary_scan_reaches_root_manifest_e2e_and_python() {
    let root = workspace_root();
    let (_, scanned) = scan_workspace(&root).expect("workspace scan must complete");
    assert!(
        scanned.iter().any(|path| path == "Cargo.toml"),
        "the retired-vocabulary gate must scan the workspace root manifest"
    );
    assert!(
        scanned
            .iter()
            .any(|path| path.starts_with("tests/e2e/") && path.ends_with(".py")),
        "the retired-vocabulary gate must scan Python sources under tests/e2e"
    );
}

/// An exemption must not outlive the code it exempts: every sanctioned
/// fragment matches at least one scanned file, and every sanctioned file
/// still uses each term it is sanctioned for (otherwise the entry is stale
/// slack a later regression could hide inside).
#[test]
fn sanctioned_paths_all_match_real_files_and_carry_no_slack() {
    let root = workspace_root();
    let (_, scanned) = scan_workspace(&root).expect("workspace scan must complete");
    for (fragment, terms) in SANCTIONED_PATHS {
        let matches: Vec<&String> = scanned
            .iter()
            .filter(|path| path.contains(fragment))
            .collect();
        assert!(
            !matches.is_empty(),
            "sanctioned fragment `{fragment}` matches no scanned file — remove the stale entry",
        );
        for term in *terms {
            let still_used = matches.iter().any(|path| {
                std::fs::read_to_string(root.join(path.as_str()))
                    .map(|contents| contents.contains(term))
                    .unwrap_or(false)
            });
            assert!(
                still_used,
                "sanctioned fragment `{fragment}` no longer uses `{term}` — shrink the entry",
            );
        }
    }
}

/// §13's second assertion: the notification-setup and session-inbound routes
/// are generic — parameterized by `{extension_id}`, with no per-channel
/// pattern. Pinned against the descriptor source so a channel-named route
/// cannot come back under a vocabulary this gate does not ban.
#[test]
fn notification_setup_and_session_routes_stay_extension_id_parameterized() {
    let root = workspace_root();
    let descriptors = root.join("crates/product/ironclaw_webui/src/webui_v2/descriptors.rs");
    let contents = std::fs::read_to_string(&descriptors).expect("descriptors source reads");
    for pattern in [
        "\"/api/webchat/v2/channels/{extension_id}/messages\"",
        "\"/api/webchat/v2/channels/{extension_id}/notifications\"",
        "\"/api/webchat/v2/channels/{extension_id}/notifications/enable\"",
        "\"/api/webchat/v2/channels/{extension_id}/notifications/disable\"",
    ] {
        assert!(
            contents.contains(pattern),
            "expected the generic route pattern {pattern} in webui_v2/descriptors.rs — \
             per-channel routes were retired by the unified channel model",
        );
    }
}

/// §0/§8's structural half: **no route may name a channel.** Every
/// channel-scoped route is `{extension_id}`-parameterized, so a literal like
/// `/api/webchat/v2/channels/telegram/pairing` is a per-channel surface no
/// matter which channel it names — the exact shape the unified channel model
/// deletes.
///
/// This scans SOURCE (Rust + the WebUI frontend) rather than the descriptor
/// table, because the defect this exists to catch lived entirely in frontend
/// modules calling routes the backend had already stopped serving: the route
/// table was clean while 871 lines of channel-named client code sat beside
/// it. A dead caller is still a channel-specific surface, and it is how the
/// next one comes back.
#[test]
fn no_source_file_names_a_channel_in_a_webchat_channel_route() {
    const MARKER: &str = "/api/webchat/v2/channels/";

    let root = workspace_root();
    let (_, scanned) = scan_workspace(&root).expect("workspace scan must complete");
    let mut offenders = Vec::new();
    for relative in &scanned {
        // This gate and its siblings name routes on purpose.
        if relative.contains("ironclaw_architecture_tests/") {
            continue;
        }
        // Tests may name concrete channels (extension-runtime overview §8):
        // a fixture id in a URL is exercising the generic route, not adding a
        // per-channel surface. The sibling specificity gate strips
        // `#[cfg(test)]` for the same reason.
        if relative.contains("/tests/")
            || relative.starts_with("tests/")
            || relative.contains(".test.")
        {
            continue;
        }
        // Code only. Guidance and contract docs carry dated gravestones that
        // describe per-channel routes precisely to record their REMOVAL —
        // banning the words there would delete the history that explains why
        // the generic route exists.
        if !(relative.ends_with(".rs")
            || relative.ends_with(".ts")
            || relative.ends_with(".tsx")
            || relative.ends_with(".mts")
            || relative.ends_with(".mjs")
            || relative.ends_with(".js"))
        {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(root.join(relative)) else {
            continue;
        };
        for (index, line) in contents.lines().enumerate() {
            let mut rest = line;
            while let Some(at) = rest.find(MARKER) {
                let tail = &rest[at + MARKER.len()..];
                let segment: String = tail
                    .chars()
                    .take_while(|c| !matches!(c, '/' | '"' | '\'' | '`' | ' ' | ')'))
                    .collect();
                // A `{…}` segment is a parameter — either the route
                // pattern's own `{extension_id}` or a caller interpolating a
                // variable it resolved from `GET /session`. Either way no
                // channel is named in the source.
                if !segment.is_empty() && !segment.starts_with('{') {
                    offenders.push(format!("{relative}:{}: /channels/{segment}/", index + 1));
                }
                rest = &tail[segment.len().min(tail.len())..];
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a channel-scoped route names a channel instead of taking {{extension_id}} — the \
         unified channel model routes every channel through one parameterized surface, and a \
         caller pointed at a per-channel route is a per-channel surface even when the backend \
         no longer serves it:\n  {}",
        offenders.join("\n  "),
    );
}
