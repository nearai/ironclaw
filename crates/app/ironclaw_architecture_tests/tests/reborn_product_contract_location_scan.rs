//! PROPOSAL §11.2.4 — the port-location rule for the **product** tier.
//!
//! Two halves, deliberately scoped differently, mirroring the extension tier's
//! scan (`reborn_product_contract_location_scan.rs`'s sibling,
//! `reborn_extension_contract_location_scan.rs`):
//!
//! - **One home.** Every type `ironclaw_product_contracts` defines must be
//!   defined nowhere else under `crates/`. Discovery, not enumeration: adding a
//!   contract to the crate enrolls it in the rule automatically.
//! - **One import path.** No crate may `pub use` one of the product tier's
//!   *ports* or the headline membrane types CHECKLIST WS1.4 names. This is the
//!   re-export-path trap: a second path for one trait is how
//!   `extension_host`, `composition`, and `product` each ended up with their own
//!   name for `ChannelAdapter` before WS1.3/WS1.4 deleted the chain.
//!
//! **Scope note — superseded 2026-08-05 by the WS5 `product` narrows row, and
//! kept because the deferral it records is now discharged in part.** It used to
//! read: *"`ironclaw_assistant` re-exports roughly 120 product value DTOs … the
//! DTO re-export sweep belongs with WS2/WS6, which repoint those crates for
//! their own reasons."* WS2 and WS6 both landed and neither took it; §6.9.1's
//! `product` narrowing row owns it, and its facade half is **executed**:
//! `crates/ironclaw_assistant/src/lib.rs` now declares **zero** foreign
//! `pub use` statements, pinned below by
//! [`product_declares_no_foreign_re_export_facade`]. What went was the block the
//! row names — the 148 symbols (19 `host_api::product_adapter`, 42
//! `extension_contracts`, 87 `product_contracts`) that the WS1.3/WS1.4/WS1.5
//! splits left behind when the single `host_api::product_adapter` module they
//! all came from was divided across three crates.
//!
//! **What is deliberately still not governed, and by whose ruling.** 193 further
//! foreign re-exports live in this crate's *private* modules (`reborn_services`,
//! `lifecycle`, `auth_continuation`) and reach the outside only through
//! `lib.rs`'s re-exports of those modules. They are a different population — the
//! `product_wire` / `workspace_views` / `admin_users` / `descriptors` /
//! `package_lifecycle` WebUI DTOs, none of which ever lived in
//! `host_api::product_adapter` — and they belong to a decided carve-out:
//! PROPOSAL §12.11 D-B keeps the frozen operation inventory in
//! `ironclaw_assistant` because its constants are *generic over* these DTOs, and
//! the residue is already pinned shrink-only as
//! `WEBUI_PRODUCT_SYMBOL_BASELINE` in `reborn_transport_product_boundary.rs`
//! (the `webui` row's residue, with its own owner). Widening this scan onto them
//! would contradict D-B rather than enforce it.
//!
//! What the import-path rule exists to protect is the *ports* — the traits whose
//! implementation site is the whole reason the crate exists — and those are
//! governed here with zero exemptions: `ProductSurface`,
//! `ChannelInboundProductSurface`, and `ProjectionStream` each had exactly one
//! re-export chain at WS1.4, and all three are deleted.
//!
//! **A collision this scan can see and the extension tier's cannot.**
//! `ProductSurfaceKind` and `ProductAdapterError` stay in `ironclaw_host_api`
//! (`product_adapter::identity` / `product_adapter_error`) even though they
//! read like product-tier names. That is not an oversight: `host_api` may hold
//! no internal dependency, and `host_api::user_identity` names
//! `AdapterInstallationId` from the same module while `host_api`'s own
//! `product_adapter::auth` names `ProductAdapterError`. Both tiers reach them
//! downward, which is the only placement that serves both. They are therefore
//! not defined in this crate and this scan never sees them.

// This scan uses the shared type-def walker and workspace-root helper; the
// other helpers in the shared module are unreachable from this binary (each
// test binary compiles the whole module and uses a different subset).
#[allow(dead_code)]
mod ratchet_support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use ratchet_support::{
    TypeDefOccurrence, collect_type_defs, crate_dir, is_rust_identifier, nested_workspace_roots,
    owning_crate_name, workspace_root,
};

/// The crate that owns the product tier's contracts.
const OWNER: &str = "ironclaw_product_contracts";

const TYPE_KEYWORDS: &[&str] = &["trait ", "struct ", "enum "];

/// This file declares fixture types and fixture `pub use` lines that would
/// otherwise be scanned as real definitions and real re-exports.
const SELF_FILE: &str = "reborn_product_contract_location_scan.rs";

/// The contracts CHECKLIST WS1.4 promises by name. Enumerated (not discovered)
/// so a rename or deletion has to come through this file in the same change
/// rather than silently shrinking the governed set.
const FROZEN_CONTRACT_NAMES: &[&str] = &[
    // The membrane itself.
    "BoundProductSurface",
    "ChannelInboundProductSurface",
    "ProductSurface",
    "ProductSurfaceCaller",
    // The invoke/query/stream DTOs that cross it.
    "ProductSurfaceInvokeRequest",
    "ProductSurfaceInvokeResponse",
    "ProductSurfaceQueryPage",
    "ProductSurfaceQueryRequest",
    "ProductSurfaceStreamRequest",
    "ProductSurfaceStreamResponse",
    // The stable error family every transport renders.
    "ProductSurfaceError",
    "ProductSurfaceErrorCode",
    "ProductSurfaceErrorKind",
    // Product wire DTO homes named by the row.
    "LifecycleProductAction",
    "LifecycleProductResponse",
    "ProductInboundEnvelope",
    "ProductOutboundEnvelope",
    "ProductProjectionState",
];

/// Names that are governed but whose *definition* the one-home half must not
/// police, because an identically-named type legitimately exists elsewhere.
///
/// Two entries, both surfaced by WS1.4 when `outbound` and `projection`
/// arrived from `ironclaw_host_api`, where this scan could not see them. Each
/// is a *same name, different layer* pair — not a duplicate definition of one
/// concept — and each names the colliding definition:
///
/// - `ProjectionCursor` — `ironclaw_event_projections` (`src/lib.rs:116`).
///   The product-tier type is an opaque, bounded, `serde(transparent)` **wire
///   token** a transport round-trips without interpreting. The projections
///   type is the **structured internal position** it stands for
///   (`{ runtime: EventCursor, scope: ProjectionScope }`), with documented
///   rebase semantics and a `RebaseRequired` error the wire form has no notion
///   of. Collapsing them would put `EventCursor` and `ProjectionScope` on the
///   product wire.
/// - `ProjectionSubscriptionRequest` — `ironclaw_outbound`
///   (`src/types.rs:157`). Different fields, different cursor type, different
///   job: the product-tier one is `{ actor, scope: TurnScope, after_cursor }`,
///   the request a caller makes through the surface; the outbound one is
///   `{ subscription_id, actor, scope: ProjectionScope, thread_id,
///   after_cursor }`, the durable record request its subscription store takes.
///
/// In both cases the shared *name* is the real defect — two layers using one
/// English word — and renaming is a §5.1 type-name decision (CHECKLIST WS10),
/// not a contracts extraction. Recorded here so the finding is visible and
/// attributable rather than silently passed.
const COLLISION_EXEMPT: &[&str] = &["ProjectionCursor", "ProjectionSubscriptionRequest"];

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
/// directory that roots a SEPARATE workspace (the `wasm-src/` guest
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

/// Every `.rs` file under `dir`, recursively. Local (not shared) for the same
/// reason the extension tier's scan keeps its own copy: each test binary
/// compiles the shared module and uses a different subset of it.
fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Same skip list as the extension tier's scan: `target` is the build
            // tree and `node_modules` is the WebUI frontend's vendored tree,
            // which really is present under `crates/`. Neither can hold a
            // governed definition, and descending them makes the walk's cost
            // depend on local build state.
            if matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some("target") | Some("node_modules")
            ) {
                continue;
            }
            collect_rust_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn render(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Every type and trait `ironclaw_product_contracts` defines. Discovery, not
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
fn product_contracts_are_defined_only_in_their_owning_crate() {
    let root = workspace_root();
    let governed = governed_names(&root);

    let mut violations = Vec::new();
    for name in FROZEN_CONTRACT_NAMES {
        if !governed.contains(*name) {
            violations.push(format!(
                "{name} is frozen as a product-tier contract but {OWNER} no longer defines it — \
                 if it was renamed or moved, update FROZEN_CONTRACT_NAMES in the same change"
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
                    "{name} is defined in {actual} ({}) but the product tier's contracts are \
                     owned by {OWNER}",
                    render(&occurrence.path, &root)
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "product-contract location rule violated (PROPOSAL §11.2.4):\n{}",
        violations.join("\n")
    );
}

/// The names the one-import-path half governs: every trait the crate defines
/// (discovered, so a new port inherits the rule) plus the membrane types
/// CHECKLIST WS1.4 names. See the module doc for what is deliberately out of
/// scope and why.
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
    names.extend(
        [
            "BoundProductSurface",
            "ProductSurfaceCaller",
            "ProductSurfaceError",
        ]
        .into_iter()
        .map(str::to_string),
    );
    names
}

#[test]
fn no_crate_re_exports_a_product_contract_it_does_not_own() {
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
        "a product-tier contract must have one import path, so no crate may `pub use` one it \
         does not own (PROPOSAL §11.2.4, the re-export-path trap):\n{}",
        violations.join("\n")
    );
}

/// Every `pub use` statement that names one of `names` *and* resolves outside
/// the current crate — i.e. names an `ironclaw_*` crate explicitly. An
/// intra-crate `pub use self::surface::ProductSurface` is module plumbing and
/// legal inside the owner; `pub use ironclaw_product_contracts::surface::
/// ProductSurface` from another crate is a second import path and is not.
///
/// Statement-oriented, not line-oriented: a multi-line group re-export is one
/// statement, and `//` comments are stripped so a commented-out example never
/// counts.
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

// ---------------------------------------------------------------------------
// Self-tests (CHECKLIST WS10: every new scanner lands with positive AND
// negative fixtures, and every half asserts it measured something before it
// asserts anything else).
// ---------------------------------------------------------------------------

#[test]
fn governed_set_covers_the_frozen_contract_names() {
    let governed = governed_names(&workspace_root());
    // All three keywords are reached: a trait, a struct, and an enum.
    assert!(governed.contains("ProductSurface"), "trait discovery");
    assert!(governed.contains("BoundProductSurface"), "struct discovery");
    assert!(
        governed.contains("ProductSurfaceErrorCode"),
        "enum discovery"
    );
    for name in FROZEN_CONTRACT_NAMES {
        assert!(
            governed.contains(*name),
            "{name} is frozen but not discovered — the walk missed it"
        );
    }
}

#[test]
fn the_import_path_half_governs_ports_and_the_membrane_types_only() {
    let governed = single_import_path_names(&workspace_root());
    // Ports are discovered.
    assert!(governed.contains("ProductSurface"));
    assert!(governed.contains("ChannelInboundProductSurface"));
    assert!(governed.contains("ProjectionStream"));
    // The three membrane value types are governed by name.
    assert!(governed.contains("BoundProductSurface"));
    assert!(governed.contains("ProductSurfaceCaller"));
    assert!(governed.contains("ProductSurfaceError"));
    // The product DTO bulk is deliberately out of scope — see the module doc.
    assert!(!governed.contains("ProductInboundEnvelope"));
    assert!(!governed.contains("ProductProjectionState"));
    assert!(!governed.contains("LifecycleExtensionSummary"));
    // Macro-generated lifecycle refs are out of both halves: they are not
    // visible to a `pub struct` walk at all.
    assert!(!governed.contains("LifecyclePackageRef"));
}

/// The crate that owns the product tier's *implementation*. Its `lib.rs` is the
/// file the §6.9.1 facade lived in.
const PRODUCT: &str = "ironclaw_assistant";

/// `ironclaw_assistant/src/lib.rs` declares **no** foreign `pub use`.
///
/// This is the executable half of §6.9.1's *"its ~120-symbol re-export facade
/// over `host_api::product_adapter` dissolves; consumers import contracts"*.
/// The rule is a crate-root property rather than a name list on purpose: a name
/// list would have to be maintained alongside three contracts crates and would
/// go stale the first time one of them gained a type, while "this file re-exports
/// only what this crate declares" is the invariant the row actually bought and
/// cannot be satisfied by adding one more name.
///
/// Scoped to `lib.rs`, deliberately. The 193 foreign re-exports inside this
/// crate's private modules are §12.11 D-B's inventory carve-out and are pinned
/// shrink-only elsewhere (`WEBUI_PRODUCT_SYMBOL_BASELINE`); see the module doc.
/// Widening this test to the whole `src` tree would go red on a decided ruling
/// the day it landed.
#[test]
fn product_declares_no_foreign_re_export_facade() {
    let root = workspace_root();
    let lib = crate_dir(&root, PRODUCT).join("src/lib.rs");
    let source = std::fs::read_to_string(&lib)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", lib.display()));

    let mut offenders = Vec::new();
    let mut statement = String::new();
    let mut statement_line = 0usize;
    for (index, raw) in source.lines().enumerate() {
        let line = match raw.split_once("//") {
            Some((code, _)) => code,
            None => raw,
        };
        if statement.is_empty() {
            if !line.trim_start().starts_with("pub use ") {
                continue;
            }
            statement_line = index + 1;
        }
        statement.push_str(line);
        if !line.contains(';') {
            continue;
        }
        let stmt = std::mem::take(&mut statement);
        let path = stmt.trim().trim_start_matches("pub use ").trim();
        if path.starts_with("ironclaw_") {
            let named: String = path.chars().take(72).collect();
            offenders.push(format!(
                "    {PRODUCT}/src/lib.rs:{statement_line}: {named}"
            ));
        }
    }

    assert!(
        offenders.is_empty(),
        "{}/src/lib.rs re-exports {} foreign path(s). PROPOSAL §6.9.1 dissolved that \
         facade (WS5): this crate's root re-exports only what this crate DECLARES, and \
         a consumer imports every other name from the crate that owns it \
         (`ironclaw_product_contracts`, `ironclaw_extension_contracts`, \
         `ironclaw_host_api::product_adapter`). A second import path is the defect \
         §11.2.4 exists to prevent — it is how `ironclaw_extension_host` and \
         `ironclaw_webui` reached the protocol-auth mint family through product before \
         WS1.5:\n{}",
        PRODUCT,
        offenders.len(),
        offenders.join("\n")
    );
}

/// The collision exemptions are a documented carve-out, not a hole: each one
/// must still be a real type in the owner crate (so a rename cannot leave a
/// dead exemption behind), and none of them may be a *port* — an exempted
/// trait would be the rule quietly turning itself off.
#[test]
fn collision_exemptions_name_live_owner_value_types_only() {
    let root = workspace_root();
    let governed = governed_names(&root);
    assert!(
        !COLLISION_EXEMPT.is_empty(),
        "the exemption self-test measured nothing"
    );

    let mut traits: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    collect_type_defs(
        &crate_dir(&root, OWNER).join("src"),
        &["trait "],
        &is_rust_identifier,
        &[],
        &mut traits,
    );

    for name in COLLISION_EXEMPT {
        assert!(
            governed.contains(*name),
            "{name} is exempted from the one-home half but {OWNER} no longer defines it — \
             delete the exemption with the type"
        );
        assert!(
            !traits.contains_key(*name),
            "{name} is a port; a port may never be collision-exempt"
        );
    }
}

#[test]
fn definition_scan_matches_definitions_not_references() {
    let sample = r##"
pub struct ProductSurfaceCaller { tenant_id: String }
struct ProductSurfaceError;
impl ProductSurface for Thing {}
const DOC: &str = "pub enum ProductSurfaceErrorKind";
// pub trait ProductSurface: Send {}
pub enum ProductSurfaceErrorCode { Internal }
"##;
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("sample.rs");
    std::fs::write(&file, sample).expect("write sample");

    let governed: BTreeSet<String> = [
        "ProductSurface",
        "ProductSurfaceCaller",
        "ProductSurfaceError",
        "ProductSurfaceErrorCode",
        "ProductSurfaceErrorKind",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();

    let mut found: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    collect_type_defs(
        dir.path(),
        TYPE_KEYWORDS,
        &|ident: &str| governed.contains(ident),
        &[],
        &mut found,
    );

    // Only the two `pub` definitions count. The non-`pub` struct, the `impl`
    // reference, the string literal, and the commented-out trait must not.
    assert_eq!(
        found.keys().cloned().collect::<Vec<_>>(),
        vec![
            "ProductSurfaceCaller".to_string(),
            "ProductSurfaceErrorCode".to_string()
        ],
        "definition scan matched references or non-public items"
    );
}

#[test]
fn re_export_scan_separates_intra_crate_plumbing_from_cross_crate_paths() {
    let names = [
        "ProductSurface",
        "ChannelInboundProductSurface",
        "ProjectionStream",
    ];

    // Negative fixtures: none of these is a second cross-crate import path.
    let legal = r##"
pub use self::surface::ProductSurface;
pub use surface::ProductSurface;
use ironclaw_product_contracts::surface::ProductSurface;
// pub use ironclaw_product_contracts::surface::ProductSurface;
pub use ironclaw_host_api::product_adapter::ProductAdapterError;
"##;
    assert_eq!(
        cross_crate_re_exports(legal, &names),
        Vec::<(usize, String)>::new(),
        "intra-crate plumbing, plain `use`, comments, and unowned types must not be flagged"
    );

    // Positive fixtures: the three real chains WS1.4 deleted.
    let illegal = r##"
pub use ironclaw_product_contracts::surface::ProductSurface;
pub use ironclaw_product_contracts::surface::{
    ChannelInboundProductSurface, ChannelInboundSurfaceRequest,
};
pub use ironclaw_assistant::ProjectionStream;
"##;
    assert_eq!(
        cross_crate_re_exports(illegal, &names),
        vec![
            (2, "ProductSurface".to_string()),
            (3, "ChannelInboundProductSurface".to_string()),
            (6, "ProjectionStream".to_string()),
        ],
        "the multi-line group re-export must be reported at its opening line"
    );
}
