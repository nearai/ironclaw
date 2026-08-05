#[allow(dead_code)]
mod ratchet_support;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::Value;

const COMPOSITION_CRATE: &str = "ironclaw_composition";

// Crate paths are spelled flat (`crates/ironclaw_x/...`) and RESOLVED through
// the crate inventory, so the family move (PROPOSAL section 5) repoints them
// without editing the literals. Identity on today's tree - pinned by
// `reborn_crate_inventory.rs` (CHECKLIST WS10).
use ratchet_support::{crate_path, workspace_root};

const SUBSTRATE_CRATES: &[&str] = &[
    "ironclaw_auth",
    "ironclaw_host_api",
    "ironclaw_storage",
    "ironclaw_filesystem",
    "ironclaw_event_log",
    "ironclaw_event_projections",
    "ironclaw_event_streams",
    "ironclaw_extension_registry",
    "ironclaw_authorization",
    "ironclaw_approvals",
    "ironclaw_approvals",
    "ironclaw_resources",
    "ironclaw_trust",
    "ironclaw_capabilities",
    "ironclaw_processes",
    "ironclaw_secrets",
    "ironclaw_network",
    "ironclaw_memory",
    "ironclaw_host_runtime",
    "ironclaw_mcp",
    "ironclaw_sandbox",
    "ironclaw_wasm",
    "ironclaw_turns",
    "ironclaw_threads",
    "ironclaw_loop_host",
    "ironclaw_turn_runner",
    "ironclaw_openai_compat",
    "ironclaw_telegram_extension",
    "ironclaw_assistant",
    "ironclaw_assistant",
    "ironclaw_triggers",
];

#[test]
fn no_substrate_crate_depends_on_composition_root() {
    let dependencies = workspace_dependencies();
    for substrate in SUBSTRATE_CRATES {
        let Some(actual) = dependencies.get(*substrate) else {
            continue;
        };
        assert!(
            !actual.iter().any(|dep| dep == COMPOSITION_CRATE),
            "{substrate} must not depend on {COMPOSITION_CRATE}; actual deps: {actual:?}"
        );
    }
}

#[test]
fn composition_root_is_workspace_member() {
    let dependencies = workspace_dependencies();
    assert!(dependencies.contains_key(COMPOSITION_CRATE));
}

#[test]
fn composition_public_api_is_service_shaped() {
    let lib = std::fs::read_to_string(crate_path(
        &workspace_root(),
        "crates/ironclaw_composition/src/lib.rs",
    ))
    .expect("composition lib readable");
    let input = std::fs::read_to_string(crate_path(
        &workspace_root(),
        "crates/ironclaw_composition/src/input.rs",
    ))
    .expect("composition input readable");
    let factory = std::fs::read_to_string(crate_path(
        &workspace_root(),
        "crates/ironclaw_composition/src/factory.rs",
    ))
    .expect("composition factory readable");
    let public_surface = format!("{lib}\n{input}\n{factory}");

    assert!(
        !lib.contains("pub use input::RebornStorageInput"),
        "composition service API must not re-export raw storage input types"
    );
    assert!(
        !input.contains("pub enum RebornStorageInput"),
        "RebornStorageInput must stay crate-private"
    );
    assert!(
        !input.contains("pub db:") && !input.contains("pub pool:"),
        "raw database handles must not be public struct/enum fields"
    );

    for forbidden in [
        "pub run_state_store",
        "pub approval_request_store",
        "pub capability_lease_store",
        "pub event_log",
        "pub audit_log",
        "pub secret_store",
        "pub network_enforcer",
        "pub process_services",
        "pub filesystem_root",
        "pub resource_governor",
        "LegacyBridgeMode",
    ] {
        assert!(
            !public_surface.contains(forbidden),
            "composition root public API must not expose `{forbidden}`"
        );
    }
}

/// The composition root owns assembly, not prompt text. Prompt content is a
/// behavior of the crate that puts it in front of a model, so it lives in that
/// crate's `prompts/` directory (`ironclaw_loop_host` for the system prompt and
/// the identity-context protocols; `ironclaw_skills`, `ironclaw_turns` and the
/// extension packages for theirs) — CHECKLIST WS6 "system-prompt content →
/// owning prompt asset", PROPOSAL §6.10.1, and the house rule that prompt
/// templates live in files inside the crate that owns the behavior.
///
/// Embedding *config-as-data* is still composition's charter, so this scan is
/// keyed on markdown, not on `include_str!` (`builtin_capability_policy.toml`
/// is deliberately unaffected).
#[test]
fn composition_root_embeds_no_prompt_content() {
    let crate_root = workspace_root().join("crates/ironclaw_composition");

    let sources = rust_sources(&crate_root.join("src"));
    // `rust_sources` panics on an unreadable directory or entry, so an
    // *incomplete* walk is already loud. This floor covers the case that stays
    // silent: a walk that reads a perfectly good directory which is no longer
    // the crate — after the WS7 family move relocates `crates/…` under a family
    // directory, a stale path can resolve to something small and readable
    // rather than erroring.
    assert!(
        sources.len() >= 50,
        "expected the composition source walk to reach at least 50 files, saw {} — \
         the scan below cannot be trusted",
        sources.len()
    );

    let embedded_markdown: Vec<String> = sources
        .iter()
        .flat_map(|(path, contents)| {
            markdown_include_sites(contents)
                .into_iter()
                .map(|site| format!("{}: {site}", path.display()))
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        embedded_markdown.is_empty(),
        "the composition root must not embed markdown prompt content; move the asset \
         into the `prompts/` directory of the crate that owns the behavior and export \
         it as a `pub const` (see PROPOSAL §6.10.1). Offending sites:\n{}",
        embedded_markdown.join("\n")
    );

    // A prompt asset that is *shipped* but not yet embedded is the same debt one
    // commit earlier, so the directory itself is pinned rather than only its
    // `include_str!` call sites. Crate guides (`AGENTS.md` / `CLAUDE.md`) are
    // documentation, not prompt content.
    let shipped_markdown: Vec<String> = shipped_non_guidance_markdown(&crate_root)
        .into_iter()
        .map(|path| path.display().to_string())
        .collect();
    assert!(
        shipped_markdown.is_empty(),
        "the composition root ships markdown assets that are not crate guidance; \
         prompt content belongs to its owning crate's `prompts/` directory. Found:\n{}",
        shipped_markdown.join("\n")
    );
}

#[test]
fn composition_public_pub_use_surface_matches_snapshot() {
    let root = workspace_root();
    let lib = std::fs::read_to_string(composition_src_path().join("lib.rs"))
        .expect("composition lib.rs readable");
    let snapshot = std::fs::read_to_string(root.join("docs/plans/composition-pubuse.snapshot"))
        .expect("composition pub-use snapshot readable");

    let actual = extract_pub_use_surface(&lib);
    assert_eq!(
        snapshot, actual,
        "composition public pub-use surface must match docs/plans/composition-pubuse.snapshot; \
         update the snapshot only for intentional public service changes"
    );
}

/// CHECKLIST WS6 asks that the re-export wall be reduced *to a documented
/// snapshot* — "every survivor names consumer + enforcing test". The snapshot
/// half is pinned above; this is the documentation half.
///
/// Every top-level `pub use` in composition's `lib.rs` must carry a
/// `// consumer: … · pinned by: …` line in the comment block directly above it
/// (above any `#[cfg(…)]` attributes, which is also where the snapshot
/// extractor expects them, so annotating never perturbs the snapshot).
///
/// The rule this enforces: a re-export earns its place only when the consumer
/// cannot reach the symbol at its owning crate. Without the annotation, a wall
/// entry is indistinguishable from an accident, which is how the previous 52
/// entries accumulated 17 with no consumer at all.
#[test]
fn composition_public_pub_use_entries_name_their_consumer() {
    let lib = std::fs::read_to_string(composition_src_path().join("lib.rs"))
        .expect("composition lib.rs readable");
    let (documented, undocumented) = pub_use_consumer_annotations(&lib);

    assert!(
        documented > 0,
        "the annotation scan found no documented entries at all — the scan is broken, \
         not the wall"
    );
    assert!(
        undocumented.is_empty(),
        "every public `pub use` in the composition root must name its consumer and the \
         test that pins it, as a `// consumer: <who> · pinned by: <test>` line above the \
         entry (above any `#[cfg]`). A re-export whose consumer can reach the symbol at \
         its owning crate should be deleted, not annotated. Undocumented entries:\n{}",
        undocumented.join("\n")
    );
}

/// Split a crate root's top-level `pub use` entries into
/// `(annotated_count, unannotated_entries)`.
///
/// Extracted from the gate above so the scan itself is testable on synthetic
/// input — a scanner that silently stops looking is the fail-*open* direction,
/// and the only way to see that is to feed it a shape the real file does not
/// have today.
///
/// The walk upward is bracket-aware. A `#[cfg(…)]` split across lines puts
/// interior lines (`feature = "x",`) and a closer (`))]`) between the comment
/// block and the entry; matching only on a `#[` / `//` / `]` line prefix stops
/// on those and reports a correctly annotated re-export as undocumented. So
/// unmatched `]`s are counted while walking, and every line inside an open
/// attribute is consumed regardless of how it starts.
fn pub_use_consumer_annotations(source: &str) -> (usize, Vec<String>) {
    let lines: Vec<&str> = source.lines().collect();
    let mut undocumented = Vec::new();
    let mut documented = 0usize;
    let mut in_pub_use = false;
    for (index, line) in lines.iter().enumerate() {
        if in_pub_use {
            if line.trim_start().contains(';') {
                in_pub_use = false;
            }
            continue;
        }
        if !line.starts_with("pub use") {
            continue;
        }
        if !line.trim_start().contains(';') {
            in_pub_use = true;
        }

        // Walk back over the contiguous attribute / comment block above the
        // entry looking for the annotation.
        let mut cursor = index;
        let mut annotated = false;
        // Unmatched `]` seen so far while walking upward: > 0 means we are
        // inside an attribute whose `#[` opener is still above us.
        let mut open_attribute = 0i32;
        while cursor > 0 {
            let raw = lines[cursor - 1];
            let above = raw.trim_start();
            if open_attribute > 0 {
                open_attribute += bracket_balance(raw);
                open_attribute = open_attribute.max(0);
                cursor -= 1;
                continue;
            }
            if above.starts_with("//") {
                // Checked before the bracket arithmetic: a comment may carry
                // unbalanced brackets (`// consumer: foo[bar]`) and must never
                // be mistaken for an attribute tail.
                if above.starts_with("// consumer:") && above.contains("pinned by:") {
                    annotated = true;
                }
                cursor -= 1;
                continue;
            }
            let balance = bracket_balance(raw);
            if above.starts_with("#[") {
                // Single-line attribute, or the opener of a multiline one we
                // already walked through.
                open_attribute = (open_attribute + balance).max(0);
                cursor -= 1;
                continue;
            }
            if balance > 0 {
                // Tail of a multiline attribute: `))]`, `)]`, `]`, …
                open_attribute = balance;
                cursor -= 1;
                continue;
            }
            break;
        }
        if annotated {
            documented += 1;
        } else {
            undocumented.push(format!("lib.rs:{}: {}", index + 1, line));
        }
    }
    (documented, undocumented)
}

/// `count(']') - count('[')` for one line: positive when the line closes more
/// attribute brackets than it opens.
fn bracket_balance(line: &str) -> i32 {
    line.matches(']').count() as i32 - line.matches('[').count() as i32
}

/// The scan must see a `// consumer: … · pinned by: …` line through a
/// multiline attribute, and must still catch a genuinely unannotated entry.
///
/// The checked-in `lib.rs` carries only single-line `#[cfg(…)]` attributes, so
/// without this fixture the bracket-aware walk could regress to the
/// prefix-match version and stay green — while quietly rejecting the first
/// multiline `#[cfg(any(…))]` anyone writes above a re-export.
#[test]
fn the_consumer_annotation_scan_reads_through_a_multiline_attribute() {
    let multiline = "\
/// Doc.
// consumer: `ironclaw_cli` · pinned by: `some_test`
#[cfg(any(
    test,
    feature = \"test-support\"
))]
pub use ironclaw_thing::Thing;
";
    let (documented, undocumented) = pub_use_consumer_annotations(multiline);
    assert_eq!(
        documented, 1,
        "a multiline `#[cfg(any(…))]` between the annotation and the entry must not \
         hide the annotation; got undocumented: {undocumented:?}"
    );
    assert!(undocumented.is_empty(), "{undocumented:?}");

    let single_line = "\
// consumer: `ironclaw_cli` · pinned by: `some_test`
#[cfg(test)]
pub use ironclaw_thing::Thing;
";
    let (documented, undocumented) = pub_use_consumer_annotations(single_line);
    assert_eq!(documented, 1, "{undocumented:?}");

    // Sabotage: the same shape with the annotation removed must still fail.
    let unannotated = "\
/// Doc.
#[cfg(any(
    test,
    feature = \"test-support\"
))]
pub use ironclaw_thing::Thing;
";
    let (documented, undocumented) = pub_use_consumer_annotations(unannotated);
    assert_eq!(documented, 0);
    assert_eq!(
        undocumented.len(),
        1,
        "an unannotated entry under a multiline attribute must still be reported"
    );

    // A comment carrying brackets is a comment, not an attribute tail.
    let bracketed_comment = "\
// consumer: `cli` (see `map[key]`) · pinned by: `some_test`
pub use ironclaw_thing::Thing;
";
    let (documented, undocumented) = pub_use_consumer_annotations(bracketed_comment);
    assert_eq!(documented, 1, "{undocumented:?}");
}

#[test]
fn extension_host_cluster_stays_internal() {
    let lib = std::fs::read_to_string(composition_src_path().join("lib.rs"))
        .expect("composition lib.rs readable");

    assert!(
        !has_module_decl(&lib, "mod extension_host;"),
        "extension_host compatibility facade must stay removed from composition"
    );
    assert!(
        !has_module_decl(&lib, "pub mod extension_host;"),
        "extension_host must not become public"
    );
    assert!(
        !composition_src_path().join("extension_host").exists(),
        "composition must not grow a replacement extension_host module directory"
    );
    assert!(
        has_module_decl(&lib, "mod builtin_capability_policy;"),
        "builtin_capability_policy is runtime-profile policy and must stay at the crate root"
    );

    for module in EXTENSION_HOST_MOVED_MODULES
        .iter()
        .chain(EXTENSION_HOST_EXTERNALIZED_GENERIC_MODULES.iter())
    {
        let root_decl = format!("mod {module};");
        let public_root_decl = format!("pub mod {module};");
        assert!(
            !has_module_decl(&lib, &root_decl) && !has_module_decl(&lib, &public_root_decl),
            "{module} must not be reintroduced as a crate-root module"
        );
    }
}

#[test]
fn reborn_binary_main_is_thin_bootstrap() {
    let root = workspace_root();
    let reborn_main = std::fs::read_to_string(crate_path(&root, "crates/ironclaw_cli/src/main.rs"))
        .expect("reborn cli main.rs readable");

    assert!(
        reborn_main.contains("cli::run()"),
        "ironclaw-reborn main.rs should delegate to the clap command root"
    );
    for forbidden in [
        "build_reborn_runtime",
        "build_runtime",
        "axum::serve",
        "TcpListener::bind",
        "src/channels/web",
        "ironclaw::channels::web",
    ] {
        assert!(
            !reborn_main.contains(forbidden),
            "ironclaw-reborn main.rs must stay a thin bootstrap over Reborn-owned command \
             modules and factories; found `{forbidden}`"
        );
    }
}

/// The third-party hook-projection path MUST install installed-tier bindings
/// EXCLUSIVELY through `HookRegistrar::install`, never any lower-level
/// `HookDispatcherBuilder` primitive that could mint an `Installed`-tier binding
/// with a caller-chosen `owning_extension`. The registrar is the single seam
/// that (a) enforces the Installed-tier ceiling and the per-extension caps, and
/// (b) derives `owning_extension` from the installer argument (spoof-blocked).
///
/// # Why scan the WHOLE composition crate, not just `hooks.rs`
///
/// A substring scan limited to `hooks.rs` is evadable by a future refactor that
/// moves a bypass into a sibling helper module, or that hand-builds a
/// `HookBinding { trust_class: Installed, .. }` and calls `insert_binding`, or
/// that calls the generic `install_observer(.. HookTrustClass::Installed ..)`.
/// This test therefore scans EVERY non-test source line of
/// `ironclaw_composition` (the crate that owns the untrusted projection
/// path) and forbids ALL of those Installed-tier-minting primitives crate-wide.
/// The only sanctioned way for this crate to install an installed-tier binding
/// is `HookRegistrar::install`.
#[test]
fn composition_crate_installs_installed_tier_only_through_registrar() {
    let crate_src = crate_path(&workspace_root(), "crates/ironclaw_composition/src");
    let sources = rust_sources(&crate_src);
    assert!(
        !sources.is_empty(),
        "expected to find composition crate source files under {crate_src:?}"
    );

    // Direct builder/dispatcher primitives that can mint an Installed-tier
    // binding while accepting `owning_extension` (or a hand-built trust class)
    // as a free parameter — every one of these bypasses the registrar's
    // ceiling + spoof-blocked attribution.
    const FORBIDDEN_INSTALLED_PRIMITIVES: &[&str] = &[
        "install_installed_before_capability",
        "install_installed_before_prompt",
        "install_installed_observer",
        "install_installed_event_triggered",
        "install_installed_wasm_before_capability",
        "install_installed_wasm_before_prompt",
        "install_installed_wasm_observer",
        // Generic, trust-class-parameterized installer + the raw binding
        // insertion path: either could carry `HookTrustClass::Installed`.
        "install_observer(",
        "insert_binding(",
        // A hand-constructed installed-tier binding is the lowest-level bypass.
        "HookTrustClass::Installed",
    ];

    let mut saw_registrar_install = false;
    for (path, contents) in &sources {
        // Dedicated unit-test module files (e.g. `hooks/tests.rs`, declared via
        // `#[cfg(test)] mod tests;`) legitimately exercise builder APIs directly
        // and are not production code. Skip them wholesale — the inline
        // `#[cfg(test)] mod tests { .. }` stripping below covers same-file test
        // modules. This keeps the invariant robust across the #3951 finding-#4
        // decomposition (which moved the test matrix into its own file).
        if is_test_module_file(path) {
            continue;
        }
        let production = strip_test_module(contents);
        if production.contains("registrar.install(") {
            saw_registrar_install = true;
        }
        for forbidden in FORBIDDEN_INSTALLED_PRIMITIVES {
            assert!(
                !production.contains(forbidden),
                "{path:?}: third-party hook projection must install installed-tier \
                 bindings ONLY through HookRegistrar::install, never the direct \
                 primitive `{forbidden}` (registrar-only invariant: ceiling + \
                 spoof-blocked owning_extension). Move this into the registrar or \
                 use HookRegistrar::install."
            );
        }
    }

    // Positive anchor: the projection path DOES route through the registrar, so
    // the negative assertions above are not vacuously true.
    assert!(
        saw_registrar_install,
        "the projection path must install through HookRegistrar::install \
         somewhere in the composition crate"
    );
}

/// Strip trailing `#[cfg(test)] mod <name> { .. }` unit-test module(s) so the
/// architecture invariant applies to production code only (tests legitimately
/// exercise builder APIs directly). Matches the `#[cfg(test)]` attribute line
/// immediately followed by a `mod ` declaration of ANY name (so a refactor that
/// renames `mod tests` or adds a second `#[cfg(test)] mod` block is still fully
/// stripped), and cuts from the FIRST such occurrence onward — a bare
/// `#[cfg(test)]` substring also appears in doc comments, hence the `\nmod `
/// anchor rather than a bare match.
fn strip_test_module(contents: &str) -> &str {
    match contents.find("#[cfg(test)]\nmod ") {
        Some(idx) => &contents[..idx],
        None => contents,
    }
}

const EXTENSION_HOST_MOVED_MODULES: &[&str] = &[
    "bundled_skills",
    "extension_activation_credentials",
    "extension_lifecycle",
    "extension_lifecycle_capabilities",
    "extension_lifecycle_capabilities_auth_tests",
    "lifecycle",
    "skill_learning",
    "skill_listing",
    "webui_extension_credentials",
];

const EXTENSION_HOST_EXTERNALIZED_GENERIC_MODULES: &[&str] = &[
    "active_publication",
    "available_extension_import",
    "available_extensions",
    "channel_lifecycle",
    "channel_delivery",
    "channel_dm_targets",
    "extension_credential_requirements",
    "first_party_package",
    "host_api_contracts",
    "install_policy",
    "lifecycle_restore",
    "lifecycle_vocabulary",
    "mcp",
    "mcp_discovery",
    "nearai_mcp",
    "provider_instance_readiness",
    "product_lifecycle",
    "reply_contexts",
];

fn composition_src_path() -> PathBuf {
    crate_path(&workspace_root(), "crates/ironclaw_composition/src")
}

fn extract_pub_use_surface(contents: &str) -> String {
    let mut out = Vec::new();
    let mut attrs = Vec::new();
    let mut in_pub_use = false;

    for line in contents.lines() {
        let trimmed = line.trim_start();
        if in_pub_use {
            out.push(line);
            if trimmed.contains(';') {
                in_pub_use = false;
            }
            continue;
        }

        if line.starts_with("#[") {
            attrs.push(line);
            continue;
        }

        if line.starts_with("pub use") {
            out.append(&mut attrs);
            out.push(line);
            if !trimmed.contains(';') {
                in_pub_use = true;
            }
            continue;
        }

        attrs.clear();
    }

    assert!(
        !out.is_empty(),
        "expected at least one public pub-use in composition lib.rs"
    );

    let mut extracted = out.join("\n");
    extracted.push('\n');
    extracted
}

fn has_module_decl(contents: &str, declaration: &str) -> bool {
    contents
        .lines()
        .any(|line| line.trim_start() == declaration)
}

/// A dedicated unit-test module file: a `tests.rs` module file, or any source
/// under a `tests/` directory. These are test-only and may use builder APIs
/// directly, so the production-only invariant scan skips them.
fn is_test_module_file(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some("tests.rs")
        || path
            .components()
            .any(|component| component.as_os_str() == "tests")
}

/// Is this file crate guidance rather than a shipped asset?
///
/// `CONTRACT.MD` belongs here for the same reason as the other three: the repo
/// already ships it as crate-local guidance (`ironclaw_identity`,
/// `ironclaw_trust`) and CLAUDE.md's module-spec table names it. Without it, a
/// composition `CONTRACT.md` would be reported as prompt content and send the
/// author to the wrong fix.
fn is_crate_guidance(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            matches!(
                name.to_ascii_uppercase().as_str(),
                "AGENTS.MD" | "CLAUDE.MD" | "README.MD" | "CONTRACT.MD"
            )
        })
}

/// The markdown a crate *ships* — every `.md` under it that is not crate
/// guidance.
///
/// The gate and its test both go through this, so a change to the guidance
/// allowlist cannot pass because the test carries its own stale copy of it.
fn shipped_non_guidance_markdown(dir: &Path) -> Vec<PathBuf> {
    markdown_assets(dir)
        .into_iter()
        .filter(|path| !is_crate_guidance(path))
        .collect()
}

/// Refuse a symlink handed in as the *root* of either walk.
///
/// `reject_symlink` below only sees entries `read_dir` yields, and both walks
/// push their root onto the stack before that ever runs — so a symlinked root
/// was followed to its target silently, which is the same hole one level up.
fn reject_symlink_root(dir: &Path) {
    let metadata = std::fs::symlink_metadata(dir)
        .unwrap_or_else(|error| panic!("readable walk root {dir:?}: {error}"));
    reject_symlink(&metadata.file_type(), dir);
}

/// Refuse a symlink encountered by either ownership walk.
///
/// `DirEntry::file_type()` reports the **link's** type without following it, so
/// a symlink pointing at a source directory is neither `is_dir()` nor an `.rs`
/// file — the walk would step over the whole subtree and the gate would report
/// clean on source it never opened. That is the same "uninspected reads as
/// absent" failure the fail-closed reads above exist to prevent.
///
/// Rejecting is chosen over following: following needs canonical-root
/// containment plus cycle detection to be safe, and neither crate scanned here
/// has ever contained a symlink. If one is ever wanted, this panic is where the
/// decision gets made deliberately rather than silently.
fn reject_symlink(file_type: &std::fs::FileType, path: &Path) {
    assert!(
        !file_type.is_symlink(),
        "{path:?} is a symlink; the composition ownership walks do not follow \
         symlinks, and stepping over one would let the gate report clean on a \
         subtree it never read. Replace it with a real path, or teach both walks \
         to follow links with canonical-root and cycle protection."
    );
}

/// Recursively collect `(path, contents)` for every `.rs` file under `dir`.
fn rust_sources(dir: &Path) -> Vec<(PathBuf, String)> {
    reject_symlink_root(dir);
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        // Fail closed: this walk feeds ownership gates, and a skipped directory
        // makes "could not look" indistinguishable from "nothing there". The
        // file-contents read below has always panicked on failure; the
        // directory read now matches it.
        let read = std::fs::read_dir(&current)
            .unwrap_or_else(|error| panic!("readable directory {current:?}: {error}"));
        for entry in read {
            let entry =
                entry.unwrap_or_else(|error| panic!("readable entry under {current:?}: {error}"));
            let path = entry.path();
            // `Path::is_dir()` returns `false` on a metadata error, which would
            // silently drop an unreadable directory out of the walk. Ask the
            // entry and fail loud instead, matching `markdown_assets`.
            let file_type = entry
                .file_type()
                .unwrap_or_else(|error| panic!("readable file type for {path:?}: {error}"));
            reject_symlink(&file_type, &path);
            if file_type.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                let contents = std::fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("readable rust source {path:?}: {error}"));
                out.push((path, contents));
            }
        }
    }
    out
}

fn workspace_dependencies() -> HashMap<String, Vec<String>> {
    cargo_metadata()["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .filter_map(package_dependencies)
        .collect()
}

fn cargo_metadata() -> Value {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .expect("cargo metadata");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("metadata json")
}

fn package_dependencies(package: &Value) -> Option<(String, Vec<String>)> {
    let name = package["name"].as_str()?.to_string();
    let dependencies = package["dependencies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|dependency| {
            dependency
                .get("kind")
                .and_then(Value::as_str)
                .is_none_or(|kind| kind == "normal")
        })
        .filter_map(|dependency| dependency["name"].as_str())
        .filter(|name| name.starts_with("ironclaw_"))
        .map(ToString::to_string)
        .collect();
    Some((name, dependencies))
}

/// Every `include_str!` / `include_bytes!` invocation in `contents` whose
/// argument names a markdown file, rendered as a normalized single line.
///
/// Scans from each macro-name occurrence to the end of its **statement** (the
/// next `;`) rather than to the first `)`. Parsing the argument is what makes
/// this class of gate leaky, and every leak is silent: `rustfmt` wraps a long
/// argument onto its own line, so a per-line scan misses it;
/// `include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/prompt.md"))` closes its
/// *first* `)` before the path is ever seen, so a first-paren scan misses it;
/// and `include_str !(…)` plus comments inside the argument are both legal and
/// defeat naive tokenizing. A statement-bounded span is immune to all three:
/// whatever the nesting, spacing or line breaks, the path literal is inside it.
///
/// It errs toward **over**-reporting — a comment mentioning `.md` inside an
/// include statement is flagged. That direction is deliberate: a false positive
/// is a loud failure a human resolves in one line, a false negative is prompt
/// content silently back in the composition root.
fn markdown_include_sites(contents: &str) -> Vec<String> {
    let bytes = contents.as_bytes();
    let mut out = Vec::new();
    for macro_name in ["include_str", "include_bytes"] {
        let mut cursor = 0usize;
        while let Some(offset) = contents[cursor..].find(macro_name) {
            let start = cursor + offset;
            cursor = start + macro_name.len();

            // Whole identifier only: `my_include_str!` is a different macro.
            if start > 0 {
                let previous = bytes[start - 1];
                if previous.is_ascii_alphanumeric() || previous == b'_' {
                    continue;
                }
            }
            // It must actually be invoked as a macro; whitespace between the
            // name and `!` is legal Rust.
            let mut probe = cursor;
            while probe < bytes.len() && bytes[probe].is_ascii_whitespace() {
                probe += 1;
            }
            if bytes.get(probe) != Some(&b'!') {
                continue;
            }

            let statement_end = statement_end_after(contents, start);
            let statement = &contents[start..statement_end];
            if statement.to_ascii_lowercase().contains(".md") {
                out.push(statement.split_whitespace().collect::<Vec<_>>().join(" "));
            }
            cursor = statement_end;
        }
    }
    out
}

/// Index of the first `;` at or after `from` that actually terminates a Rust
/// statement — i.e. one that is not inside a string literal, a raw string, a
/// char literal, a line comment, or a (nestable) block comment. Returns
/// `contents.len()` if there is none.
///
/// A plain `.find(';')` is wrong in both directions that matter here: a `;` in
/// a doc-ish comment above the path (`// see note; below`) or inside the path
/// literal itself would end the span *before* the `.md`, and the gate would go
/// quiet. Skipping trivia is the smallest correct thing — this only has to find
/// a delimiter, not parse the expression.
fn statement_end_after(contents: &str, from: usize) -> usize {
    let bytes = contents.as_bytes();
    let mut i = from;
    let mut block_depth = 0usize;
    while i < bytes.len() {
        let rest = &bytes[i..];
        if block_depth > 0 {
            if rest.starts_with(b"/*") {
                block_depth += 1;
                i += 2;
            } else if rest.starts_with(b"*/") {
                block_depth -= 1;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if rest.starts_with(b"//") {
            i += contents[i..].find('\n').map_or(bytes.len() - i, |n| n + 1);
            continue;
        }
        if rest.starts_with(b"/*") {
            block_depth = 1;
            i += 2;
            continue;
        }
        // Raw string: `r` followed by any number of `#` then `"`.
        if bytes[i] == b'r' {
            let mut hashes = 0usize;
            while bytes.get(i + 1 + hashes) == Some(&b'#') {
                hashes += 1;
            }
            if bytes.get(i + 1 + hashes) == Some(&b'"') {
                let terminator = format!("\"{}", "#".repeat(hashes));
                let body = i + 2 + hashes;
                i = contents[body..]
                    .find(&terminator)
                    .map_or(bytes.len(), |n| body + n + terminator.len());
                continue;
            }
        }
        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
            continue;
        }
        // Char literal, but not a lifetime (`'a`) — a lifetime has no closing
        // quote, so require one within three bytes (`'x'` / `'\n'`).
        if bytes[i] == b'\'' {
            let escaped = bytes.get(i + 1) == Some(&b'\\');
            let close = if escaped { i + 3 } else { i + 2 };
            if bytes.get(close) == Some(&b'\'') {
                i = close + 1;
                continue;
            }
        }
        if bytes[i] == b';' {
            return i;
        }
        i += 1;
    }
    bytes.len()
}

/// Recursively collect every markdown file under `dir`, skipping build output.
///
/// Fails closed: an unreadable directory or entry panics rather than being
/// skipped, because "the walk could not see it" and "there is nothing there"
/// must not look the same to an ownership gate. Extensions are compared
/// case-insensitively so a `.MD` asset cannot slip past.
fn markdown_assets(dir: &Path) -> Vec<PathBuf> {
    reject_symlink_root(dir);
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let read = std::fs::read_dir(&current)
            .unwrap_or_else(|error| panic!("readable directory {current:?}: {error}"));
        for entry in read {
            let entry =
                entry.unwrap_or_else(|error| panic!("readable entry under {current:?}: {error}"));
            let path = entry.path();
            let file_type = entry
                .file_type()
                .unwrap_or_else(|error| panic!("readable file type for {path:?}: {error}"));
            reject_symlink(&file_type, &path);
            if file_type.is_dir() {
                if path.file_name().and_then(|name| name.to_str()) == Some("target") {
                    continue;
                }
                stack.push(path);
            } else if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod markdown_include_scan_tests {
    use super::markdown_include_sites;

    #[test]
    fn single_line_markdown_include_is_detected() {
        assert_eq!(
            markdown_include_sites("const A: &str = include_str!(\"../a/prompt.md\");").len(),
            1
        );
    }

    /// `rustfmt` wrapping a long path onto its own line used to make the site
    /// invisible to a per-line scan.
    #[test]
    fn multiline_markdown_include_is_detected() {
        assert_eq!(
            markdown_include_sites(
                "const A: &str = include_str!(\n    \"../../assets/prompts/a-very-long-name.md\"\n);",
            ),
            vec!["include_str!( \"../../assets/prompts/a-very-long-name.md\" )".to_string()]
        );
    }

    /// A nested argument macro closes an inner `)` before the path appears, so
    /// a first-paren scan reported clean.
    #[test]
    fn nested_argument_macro_is_detected() {
        assert_eq!(
            markdown_include_sites(
                "const A: &str = include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/prompt.md\"));",
            )
            .len(),
            1
        );
    }

    #[test]
    fn whitespace_before_the_bang_is_detected() {
        assert_eq!(
            markdown_include_sites("const A: &str = include_str !(\"../prompt.md\");").len(),
            1
        );
    }

    #[test]
    fn comment_inside_the_argument_does_not_hide_the_path() {
        assert_eq!(
            markdown_include_sites(
                "const A: &str = include_str!(\n    // seeded once at boot\n    \"../prompt.md\",\n);",
            )
            .len(),
            1
        );
    }

    #[test]
    fn uppercase_markdown_extension_is_detected() {
        assert_eq!(
            markdown_include_sites("const A: &[u8] = include_bytes!(\"../PROMPT.MD\");").len(),
            1
        );
    }

    /// Config-as-data stays composition's charter, so a non-markdown include is
    /// not a finding.
    #[test]
    fn non_markdown_include_is_not_a_finding() {
        assert!(
            markdown_include_sites(
                "const P: &str = include_str!(\"builtin_capability_policy.toml\");"
            )
            .is_empty()
        );
    }

    /// A `;` in a comment above the path used to end the span before the path
    /// was reached, which made the gate go quiet on real content.
    #[test]
    fn semicolon_in_a_comment_does_not_end_the_statement() {
        assert_eq!(
            markdown_include_sites(
                "const A: &str = include_str!(\n    // see the note; below\n    \"../prompt.md\",\n);",
            )
            .len(),
            1
        );
        assert_eq!(
            markdown_include_sites(
                "const A: &str = include_str!(\n    /* one; two */\n    \"../prompt.md\",\n);",
            )
            .len(),
            1
        );
    }

    /// The same hole one level in: a `;` inside the path literal itself.
    #[test]
    fn semicolon_inside_a_string_literal_does_not_end_the_statement() {
        assert_eq!(
            markdown_include_sites("const A: &str = include_str!(\"../a;b/prompt.md\");").len(),
            1
        );
        assert_eq!(
            markdown_include_sites("const A: &str = include_str!(r#\"../a;b/prompt.md\"#);").len(),
            1
        );
    }

    /// The span must still *stop*: a markdown path in the next statement is not
    /// this include's finding, or every non-markdown include would false-positive.
    #[test]
    fn the_span_stops_at_the_real_statement_end() {
        assert!(
            markdown_include_sites(
                "const P: &str = include_str!(\"policy.toml\");\nconst Q: &str = \"../prompt.md\";"
            )
            .is_empty()
        );
    }

    /// `markdown_assets` had no test beyond the symlink panic, so two behaviours
    /// the gate's message depends on were unpinned: the case-insensitive `.md`
    /// match, and the guidance-file exemption applied by the caller. Both drift
    /// in the *quiet* direction — a missed asset makes the shipped-asset
    /// assertion pass on a crate it never really checked.
    #[test]
    fn markdown_assets_matches_any_case_and_the_caller_keeps_only_non_guidance() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::write(root.path().join("AGENTS.md"), "# guidance\n").expect("write guidance");
        std::fs::write(root.path().join("CONTRACT.md"), "# contract\n").expect("write contract");
        std::fs::create_dir(root.path().join("prompts")).expect("create prompts dir");
        std::fs::write(root.path().join("prompts/seed.MD"), "# prompt\n").expect("write prompt");
        std::fs::write(root.path().join("policy.toml"), "k = 1\n").expect("write config");

        let mut found: Vec<String> = super::markdown_assets(root.path())
            .into_iter()
            .map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .expect("utf-8 file name")
                    .to_string()
            })
            .collect();
        found.sort();
        assert_eq!(
            found,
            vec!["AGENTS.md", "CONTRACT.md", "seed.MD"],
            "the walk must find markdown in any case and must not pick up `policy.toml` \
             (config-as-data is squarely composition's charter)"
        );

        // Drives the **production** filter, not a copy of it. A copied
        // allowlist would keep returning `seed.MD` even if the real gate
        // started dropping `CONTRACT.MD` — the gate would go quiet and this
        // test would not notice ("test through the caller").
        let mut shipped: Vec<String> = super::shipped_non_guidance_markdown(root.path())
            .into_iter()
            .map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .expect("utf-8 file name")
                    .to_string()
            })
            .collect();
        shipped.sort();
        assert_eq!(
            shipped,
            vec!["seed.MD"],
            "only the non-guidance asset may survive the filter, and `.MD` must survive it"
        );

        // Each guidance name individually, so dropping one from the allowlist
        // fails here rather than only when someone adds that file to the
        // composition crate.
        for name in ["AGENTS.md", "CLAUDE.md", "README.md", "CONTRACT.md"] {
            assert!(
                super::is_crate_guidance(std::path::Path::new(name)),
                "{name} must be treated as crate guidance, not as a shipped asset"
            );
        }
    }

    /// A symlinked subtree used to be stepped over silently by both walks,
    /// because `DirEntry::file_type()` reports the link, not its target.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_subtree_fails_the_walk_instead_of_being_skipped() {
        let root = tempfile::tempdir().expect("tempdir");
        let real = root.path().join("real");
        std::fs::create_dir(&real).expect("create real dir");
        std::fs::write(real.join("a.rs"), "// nothing\n").expect("write source");
        std::os::unix::fs::symlink(&real, root.path().join("linked")).expect("symlink");

        let panicked = std::panic::catch_unwind(|| super::rust_sources(root.path()));
        assert!(
            panicked.is_err(),
            "a symlinked subtree must fail the walk, not be skipped"
        );

        let panicked = std::panic::catch_unwind(|| super::markdown_assets(root.path()));
        assert!(
            panicked.is_err(),
            "a symlinked subtree must fail the markdown walk too"
        );

        // The root itself is pushed onto the stack before `read_dir` ever runs,
        // so a symlinked *root* used to be followed to its target silently.
        let linked_root = root.path().join("linked_root");
        std::os::unix::fs::symlink(&real, &linked_root).expect("symlink root");
        for label in ["rust_sources", "markdown_assets"] {
            let linked_root = linked_root.clone();
            let panicked = std::panic::catch_unwind(move || {
                if label == "rust_sources" {
                    super::rust_sources(&linked_root);
                } else {
                    super::markdown_assets(&linked_root);
                }
            });
            assert!(
                panicked.is_err(),
                "a symlinked walk root must fail {label}, not be followed"
            );
        }
    }

    /// The macro name must be a whole identifier and must be invoked.
    #[test]
    fn similar_identifiers_are_not_findings() {
        assert!(markdown_include_sites("let my_include_str = \"a.md\";").is_empty());
        assert!(markdown_include_sites("let include_str_path = \"a.md\";").is_empty());
    }
}
