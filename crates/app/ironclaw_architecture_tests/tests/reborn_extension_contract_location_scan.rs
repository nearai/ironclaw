//! PROPOSAL §11.2.4 — port-location scan for the extension tier.
//!
//! A contract's *definition* lives in the crate that owns it; its
//! *implementations* live wherever the behavior does. For the extension tier
//! that owner is `ironclaw_extension_contracts` (PROPOSAL §6.1.2,
//! `families/contracts.md`), and the rule has two halves:
//!
//! 1. **One home per contract.** No other crate defines a type or trait with a
//!    governed name. A second same-named definition is a shadow contract: two
//!    crates then disagree about what an installation state or a channel
//!    descriptor *is*, and nothing catches it at compile time because nobody
//!    has to import the other one.
//! 2. **One import path per contract.** No crate re-exports one. That is the
//!    re-export-path trap §11.2.4 names, and the extension tier had three live
//!    instances of it before WS1.3: `ironclaw_assistant` re-exported
//!    `PreferenceTargetCodec` from `host_api` and
//!    `ironclaw_composition` re-exported it again from *product*, so
//!    `ironclaw_telegram_extension` imported the trait through
//!    `ironclaw_assistant` (and carried a forbidden product dependency to do it)
//!    while `ironclaw_slack_extension` imported the same trait from
//!    `ironclaw_host_api`; and `ironclaw_extension_host::state` re-exported
//!    `InstallationState` beside `AuthAccountState` "so they stay
//!    discoverable together", which made `crate::InstallationState` a second
//!    path used inside that crate's own modules.
//!
//! **The two halves have deliberately different scopes.**
//!
//! * *One home* covers **every** type the crate defines, discovered rather than
//!   enumerated, so a new contract cannot be added without inheriting it. It
//!   passes today with no exemptions: nothing else in the workspace defines any
//!   of these names.
//! * *One import path* covers the crate's **traits plus the value types
//!   CHECKLIST WS1.3 names** — which is exactly §11.2.4's own scope ("adapter /
//!   surface / loop-port traits pinned to their owner; no cross-crate `pub use`
//!   of them"). The crate's `Lifecycle*` projection vocabulary is deliberately
//!   out of scope: PROPOSAL §6.1.3 assigns `package_lifecycle` to
//!   `ironclaw_product_contracts`, so `ironclaw_assistant`'s re-export of it is
//!   vocabulary heading *toward* that crate, not away from its owner, and WS1.4
//!   settles it. The two `as Reborn*` aliases in `product::reborn_services`
//!   (`RebornChannelConnectStrategy`, `RebornChannelConfigField`) are a second
//!   *name*, not just a second path; retiring the `Reborn*` prefix is CHECKLIST
//!   WS10's type-name row and is not smuggled in here.
//!
//! **Macro-generated refs are invisible to both halves, and one of them
//! collides.** `bounded_lifecycle_string!` declares `LifecyclePackageRef`,
//! `LifecyclePackageId`, and `LifecycleBlockerRef` as `pub struct $name`, which
//! no `pub struct` walk can resolve. That matters beyond coverage:
//! `ironclaw_auth::ids` declares its **own** `LifecyclePackageRef`
//! (`validated_string!`, `src/ids.rs:188`) beside `package_lifecycle`'s, so the
//! workspace carries two same-named types for one concept and this scan can see
//! neither. Recorded rather than silently passed — unifying them is a domain
//! change (auth's flow records persist its own), not a contracts extraction.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

// Only part of the shared ratchet module is reachable from this binary.
#[allow(dead_code)]
mod ratchet_support;

use ratchet_support::{
    TypeDefOccurrence, collect_type_defs, crate_dir, nested_workspace_roots, owning_crate_name,
    scan_type_defs, workspace_root,
};

const OWNER: &str = "ironclaw_extension_contracts";

const TYPE_KEYWORDS: &[&str] = &["trait ", "struct ", "enum "];

/// This scanner's own file, skipped so its fixtures do not look like
/// definitions.
const SELF_FILE: &str = "reborn_extension_contract_location_scan.rs";

/// The types CHECKLIST WS1.3 names explicitly. They must exist — a rename or
/// deletion has to come here in the same change, which is the point.
const FROZEN_CONTRACT_NAMES: &[&str] = &[
    // Adapter/port traits.
    "ChannelConnectionScopeSource",
    "Extension",
    "MemoryLifecycleHook",
    "PreferenceTargetCodec",
    // Channel manifest-surface descriptors.
    "ChannelDescriptor",
    "ChannelEgressDescriptor",
    "ChannelIngressDescriptor",
    "ChannelPresentation",
    // Lifecycle state machine.
    "InstallationState",
    "LifecyclePublicState",
    // Manifest surface kind.
    "CapabilitySurfaceKind",
    // Auth recipe schema.
    "OAuth2CodeRecipe",
    "PkceMode",
    "VendorAuthRecipe",
    // Memory manifest surface.
    "MemoryDescriptor",
    // Hosted-MCP registration input. Frozen by name because this type is one
    // half of the `hosted_mcp` <-> `package_lifecycle` mutual reference that
    // forced both modules into this crate; a future attempt to push it back
    // into `ironclaw_host_api` re-creates a dependency cycle that crate cannot
    // legally express.
    "RegisterHostedMcpRequest",
];

/// Names that are governed but whose *definition* the one-home half must not
/// police, because an identically-named type legitimately exists elsewhere.
///
/// Two entries. Four arrived with WS1.4, when `channel_adapter`, `external`,
/// and `tool_adapter` came over from `ironclaw_host_api` — where this scan
/// could not see them, because it governs only the owner crate's own
/// definitions. **WS5's `conversations`/`threads` naming-trap row discharged
/// two of them**: `ExternalActorRef` and `ExternalConversationRef` no longer
/// have a second definition, because `ironclaw_conversations` now consumes the
/// canonical pair instead of carrying its own copy, so the carve-out was
/// deleted rather than repointed. What remains is recorded rather than silently
/// passed, and names the colliding definition:
///
/// - `ToolCall` / `ToolResult` — `ironclaw_llm::provider` (`src/provider.rs`).
///   **Not the same concept.** The LLM pair is wire vocabulary: a tool call a
///   *model* emitted and the result sent back to it, both `Serialize`. The
///   extension pair is the in-process host→adapter invocation envelope, whose
///   own module doc says "call vocabulary, not wire vocabulary: nothing here
///   serializes". Two genuinely different types that happen to share the
///   obvious English name; renaming either is a §5.1 type-name decision (WS10),
///   not a contracts extraction.
///
/// `ironclaw_conversations` still declares `AdapterInstallationId` and
/// `ExternalEventId` beside the canonical pair, and this walk cannot see either:
/// they are `bounded_string_id!` macro expansions, the same blind spot the
/// module doc records for `LifecyclePackageRef`. They are adapter-scoped
/// bounded strings the binding domain owns, not channel refs, so they are a
/// naming overlap rather than a shadow contract — but they are invisible here,
/// which is why the fact is written down instead of assumed.
const COLLISION_EXEMPT: &[&str] = &["ToolCall", "ToolResult"];

/// `macro_rules!` bodies declare types with metavariable names (`pub struct
/// $name(String);` — the `bounded_lifecycle_string!` template in
/// `package_lifecycle`). Those are not identifiers and must not enter the
/// governed set, or every macro-declaring crate in the workspace matches the
/// same non-name and the scan reports nonsense. The types those macros *expand*
/// to are governed by name through `FROZEN_CONTRACT_NAMES` instead.
fn is_rust_identifier(ident: &str) -> bool {
    let mut chars = ident.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// The crate directory owning `path`, resolved through the crate inventory;
/// `None` for a file that no crate of THIS workspace owns.
///
/// Deliberately NOT "the first path component under `crates/`": that idiom
/// answers the FAMILY name once a crate sits in `crates/<family>/...`
/// (PROPOSAL section 5), so every ownership verdict below would compare a
/// type's owner against a directory name and silently attribute it to the
/// wrong crate. `owning_crate_name` is outermost-wins over the real inventory
/// (CHECKLIST WS10).
///
/// The only files legitimately owned by nothing are the ones inside a
/// directory that roots a SEPARATE workspace (`crates/ironclaw_silk_decoder`, the `wasm-src/` guest
/// components). This workspace never builds them, so they were never in scope
/// — the flat first-component idiom simply gave them a name that matched no
/// owner rule. Anything else under `crates/` with no owner means the walk left
/// the tree, and fails rather than being skipped.
fn owning_crate(root: &Path, path: &Path) -> Option<String> {
    let name = owning_crate_name(root, path);
    if !name.is_empty() {
        return Some(name);
    }
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    assert!(
        nested_workspace_roots(root)
            .iter()
            .any(|guest| relative.starts_with(&format!("{guest}/"))),
        "{relative} sits under crates/ but is owned by no crate and by no nested workspace          root — the ownership walk left the tree (CHECKLIST WS10)"
    );
    None
}

fn render(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Every type and trait `ironclaw_extension_contracts` defines. Discovery, not
/// enumeration: adding a contract to the crate enrolls it in the rule.
fn governed_names(root: &Path) -> BTreeSet<String> {
    let mut found: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    collect_type_defs(
        &crate_dir(root, OWNER).join("src"),
        TYPE_KEYWORDS,
        &is_rust_identifier,
        &[],
        &mut found,
    );
    assert!(
        !found.is_empty(),
        "no types were discovered in {OWNER} — the walk is broken, not the crate"
    );
    found.into_keys().collect()
}

#[test]
fn extension_contracts_are_defined_only_in_their_owning_crate() {
    let root = workspace_root();
    let governed = governed_names(&root);

    let mut violations = Vec::new();
    for name in FROZEN_CONTRACT_NAMES {
        if !governed.contains(*name) {
            violations.push(format!(
                "{name} is frozen as an extension-tier contract but {OWNER} no longer defines it \
                 — if it was renamed or moved, update FROZEN_CONTRACT_NAMES in the same change"
            ));
        }
    }

    let mut found: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    collect_type_defs(
        &root.join("crates"),
        TYPE_KEYWORDS,
        &|ident: &str| governed.contains(ident),
        &[SELF_FILE],
        &mut found,
    );

    for (name, occurrences) in &found {
        if COLLISION_EXEMPT.contains(&name.as_str()) {
            continue;
        }
        for occurrence in occurrences {
            let Some(actual) = owning_crate(&root, &occurrence.path) else {
                continue;
            };
            if actual != OWNER {
                violations.push(format!(
                    "{name} is defined in {actual} ({}) but the extension tier's contracts are \
                     owned by {OWNER}",
                    render(&occurrence.path, &root)
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "extension-contract location rule violated (PROPOSAL §11.2.4):\n{}",
        violations.join("\n")
    );
}

/// The names the one-import-path half governs: every trait the crate defines
/// (discovered, so a new port inherits the rule) plus the value types CHECKLIST
/// WS1.3 names. See the module doc for what is deliberately out of scope.
fn single_import_path_names(root: &Path) -> BTreeSet<String> {
    let mut found: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    collect_type_defs(
        &crate_dir(root, OWNER).join("src"),
        &["trait "],
        &is_rust_identifier,
        &[],
        &mut found,
    );
    assert!(
        !found.is_empty(),
        "no traits were discovered in {OWNER} — the walk is broken, not the crate"
    );
    let mut names: BTreeSet<String> = found.into_keys().collect();
    names.extend(FROZEN_CONTRACT_NAMES.iter().map(|name| (*name).to_string()));
    names
}

#[test]
fn no_crate_re_exports_an_extension_contract_it_does_not_own() {
    let root = workspace_root();
    let governed = single_import_path_names(&root);
    let names: Vec<&str> = governed.iter().map(String::as_str).collect();

    let mut files = Vec::new();
    collect_rust_files(&root.join("crates"), &mut files);

    let mut violations = Vec::new();
    for path in files {
        if path.file_name().and_then(|name| name.to_str()) == Some(SELF_FILE) {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        let Some(crate_name) = owning_crate(&root, &path) else {
            continue;
        };
        if crate_name == OWNER {
            continue;
        }
        for (line_number, name) in cross_crate_re_exports(&contents, &names) {
            violations.push(format!(
                "    {}:{line_number} re-exports {name}, which {OWNER} owns",
                render(&path, &root)
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "an extension-tier contract must have one import path, so no crate may `pub use` one it \
         does not own (PROPOSAL §11.2.4, the re-export-path trap):\n{}",
        violations.join("\n")
    );
}

/// Every `pub use` statement that names one of `names` *and* resolves outside
/// the current crate — i.e. names an `ironclaw_*` crate explicitly. An
/// intra-crate `pub use self::state::InstallationState` is module plumbing and
/// legal inside the owner; `pub use ironclaw_host_api::state::InstallationState`
/// from another crate is a second import path and is not.
fn cross_crate_re_exports(source: &str, names: &[&str]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut statement = String::new();
    let mut statement_line = 0usize;
    for (index, raw) in source.lines().enumerate() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if statement.is_empty() {
            if !line.starts_with("pub use ") {
                continue;
            }
            statement_line = index + 1;
        }
        statement.push(' ');
        statement.push_str(line);
        if !line.ends_with(';') {
            continue;
        }
        let finished = std::mem::take(&mut statement);
        if finished.contains("ironclaw_") {
            for name in names {
                if finished
                    .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
                    .any(|token| token == *name)
                {
                    out.push((statement_line, (*name).to_string()));
                }
            }
        }
    }
    out
}

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}"));
        let path = entry.path();
        if path.is_dir() {
            // `target` is the build tree; `node_modules` is the WebUI frontend's
            // vendored tree, which really is present under `crates/` (2,506
            // directories in a working checkout). Neither can hold a governed
            // definition, and descending them makes the walk's cost depend on
            // local build state.
            if matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some("target") | Some("node_modules")
            ) {
                continue;
            }
            collect_rust_files(&path, out);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

// ---------------------------------------------------------------------------
// Scanner self-tests — the guardrail must fail loudly on its own regressions
// (CHECKLIST WS10: every new scanner lands with positive + negative fixtures).
// ---------------------------------------------------------------------------

#[test]
fn governed_set_covers_the_frozen_contract_names() {
    let governed = governed_names(&workspace_root());
    // Discovery must reach traits, structs, and enums alike — the three
    // keywords are the whole reason a value type like `InstallationState` and a
    // port like `PreferenceTargetCodec` are governed by one rule.
    assert!(governed.contains("PreferenceTargetCodec"), "traits");
    assert!(governed.contains("ChannelDescriptor"), "structs");
    assert!(governed.contains("InstallationState"), "enums");
    for name in FROZEN_CONTRACT_NAMES {
        assert!(
            governed.contains(*name),
            "{name} is frozen but not discovered — see the failure message in \
             extension_contracts_are_defined_only_in_their_owning_crate"
        );
    }
}

#[test]
fn the_import_path_half_governs_traits_and_the_frozen_value_types_only() {
    let governed = single_import_path_names(&workspace_root());
    // Traits are discovered.
    assert!(governed.contains("PreferenceTargetCodec"));
    assert!(governed.contains("Extension"));
    assert!(governed.contains("ChannelConnectionScopeSource"));
    // Frozen value types are governed by name.
    assert!(governed.contains("InstallationState"));
    assert!(governed.contains("ChannelPresentation"));
    // The `Lifecycle*` projection vocabulary is out of scope (WS1.4 re-homes it).
    assert!(!governed.contains("LifecycleExtensionSummary"));
    assert!(!governed.contains("ChannelConnectStrategy"));
    // Macro-generated refs are out of both halves — including the one that
    // collides with `ironclaw_auth`'s same-named type (see the module doc).
    assert!(!governed.contains("LifecyclePackageRef"));
    assert!(!governed.contains("LifecyclePackageId"));
}

/// The collision exemptions are a documented carve-out, not a hole: each one
/// must still be a real type in the owner crate (so a rename cannot leave a
/// dead exemption behind), and the one-import-path half must still govern the
/// traits among them.
#[test]
fn collision_exemptions_name_live_owner_types_and_do_not_widen_the_import_path_half() {
    let root = workspace_root();
    let governed = governed_names(&root);
    assert!(
        !COLLISION_EXEMPT.is_empty(),
        "the exemption self-test measured nothing"
    );
    for name in COLLISION_EXEMPT {
        assert!(
            governed.contains(*name),
            "{name} is exempted from the one-home half but {OWNER} no longer defines it — \
             delete the exemption with the type"
        );
    }
    // The exemption is scoped to *definition* location only. Nothing here may
    // gain a second import path: no crate may re-export these either.
    let import_path_governed = single_import_path_names(&root);
    assert!(
        import_path_governed.contains("ToolAdapter"),
        "the import-path half must still govern the tool tier's trait"
    );
    assert!(
        import_path_governed.contains("ChannelAdapter"),
        "the import-path half must still govern the channel tier's trait"
    );
}

#[test]
fn definition_scan_matches_definitions_not_references() {
    let sample = r##"
        pub trait PreferenceTargetCodec {}
        pub(crate) enum InstallationState {}
        struct ChannelPresentation;              // not pub-visible -> ignored
        impl PreferenceTargetCodec for Thing {}  // reference -> ignored
        let x = "pub struct ChannelDescriptor;"; // string literal -> ignored
        // pub trait CommentedPort {}            -> ignored
    "##;
    let governed: BTreeSet<String> = [
        "PreferenceTargetCodec",
        "InstallationState",
        "ChannelPresentation",
        "ChannelDescriptor",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    let got: Vec<String> = scan_type_defs(sample, TYPE_KEYWORDS, &|ident: &str| {
        governed.contains(ident)
    })
    .into_iter()
    .map(|(ident, _)| ident)
    .collect();
    assert_eq!(got, vec!["PreferenceTargetCodec", "InstallationState"]);
}

#[test]
fn re_export_scan_separates_intra_crate_plumbing_from_cross_crate_paths() {
    let names = ["PreferenceTargetCodec", "InstallationState"];

    // Negative fixtures: legal.
    assert!(
        cross_crate_re_exports("pub use self::state::InstallationState;", &names).is_empty(),
        "intra-crate module plumbing is legal"
    );
    assert!(
        cross_crate_re_exports("pub use state::{InstallationState, Other};", &names).is_empty(),
        "an unqualified intra-crate re-export is legal"
    );
    assert!(
        cross_crate_re_exports(
            "use ironclaw_extension_contracts::state::InstallationState;",
            &names
        )
        .is_empty(),
        "a plain (non-pub) import is not a second import path"
    );
    assert!(
        cross_crate_re_exports(
            "// pub use ironclaw_extension_contracts::state::InstallationState;",
            &names
        )
        .is_empty(),
        "a commented-out re-export is not a re-export"
    );
    assert!(
        cross_crate_re_exports("pub use ironclaw_auth::AuthAccountState;", &names).is_empty(),
        "re-exporting a type this tier does not own is out of scope for this rule"
    );

    // Positive fixtures: the three real traps this change deleted.
    assert_eq!(
        cross_crate_re_exports("pub use ironclaw_assistant::PreferenceTargetCodec;", &names),
        vec![(1, "PreferenceTargetCodec".to_string())],
        "composition's re-export of product's re-export"
    );
    assert_eq!(
        cross_crate_re_exports(
            "pub use ironclaw_host_api::state::InstallationState;",
            &names
        ),
        vec![(1, "InstallationState".to_string())],
        "extension_host::state's discoverability facade"
    );
    assert_eq!(
        cross_crate_re_exports(
            "pub use ironclaw_host_api::product_adapter::{\n    PreferenceTargetCodec,\n    Other,\n};",
            &names
        ),
        vec![(1, "PreferenceTargetCodec".to_string())],
        "a multi-line group re-export is still one statement"
    );
}
