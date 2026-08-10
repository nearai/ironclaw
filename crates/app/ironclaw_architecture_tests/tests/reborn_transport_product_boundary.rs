//! CHECKLIST WS5, the `webui` and `openai_compat` rows — the transport port
//! inversion (PROPOSAL §6.1.3, §6.9.3, §6.9.4).
//!
//! Both transports are supposed to compile against the *product boundary*, not
//! against `ironclaw_assistant`: §6.9.4 says WebUI's "`ironclaw_assistant` dep →
//! `product_contracts`", §6.9.3 says the same for the OpenAI-compatible
//! adapter, and §6.1.3 exists so a route handler can name a request body, a
//! response DTO, or an operation descriptor without pulling a 50k-line
//! workflow crate.
//!
//! The layer matrix cannot see this edge — `ironclaw_webui`,
//! `ironclaw_openai_compat`, and `ironclaw_assistant` all declare
//! `layer = "products"`, so `products → products` is legal and there is no
//! `LAYER_MATRIX_EXCEPTION` to fall. Hence this gate, in the same shape as
//! `reborn_extension_host_port_inversion.rs`:
//!
//! - **The residue is frozen and shrink-only.** Every `ironclaw_assistant`
//!   symbol either transport still names is enumerated with the reason it
//!   could not move. A new one fails; a stale one fails too, so the entry is
//!   deleted by whatever change removes the need.
//! - **The moved vocabulary is pinned where it landed** — declared in
//!   `ironclaw_product_contracts`, not re-declared in `ironclaw_assistant`.
//! - **The frozen inventory is pinned where it stayed.** §6.1.3 keeps
//!   product's concrete command/view/capability *constants* in product. They
//!   are the largest term in the residue, and the tempting way to "finish" the
//!   row is to move them — which would hand the contracts crate the product's
//!   surface inventory. Asserted in the opposite direction so that is loud.

// The shared walker is compiled per test binary; each binary uses a subset.
#[allow(dead_code)]
mod ratchet_support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use std::process::Command;
use std::sync::OnceLock;

use serde_json::Value;

use ratchet_support::{
    TypeDefOccurrence, collect_type_defs, is_rust_identifier, production_rust_files,
    strip_cfg_test_blocks, strip_comments_and_strings, workspace_root,
};

const PRODUCT: &str = "ironclaw_assistant";
const PRODUCT_CONTRACTS: &str = "ironclaw_product_contracts";
const WEBUI: &str = "ironclaw_webui";
const EXTENSION_HOST: &str = "ironclaw_extension_host";
const OPENAI_COMPAT: &str = "ironclaw_openai_compat";

/// `ironclaw_assistant` symbols `ironclaw_webui` still names in production code,
/// grouped by the reason each stayed. **Shrink-only**: this list is the whole
/// surviving edge, so an addition re-widens it and a stale entry is a lie.
///
/// Two reasons, both structural:
///
/// 1. **`inventory`** — §6.1.3 assigns the command/view/capability *descriptor
///    types* to `product_contracts` but keeps "product's concrete constants …
///    as the frozen inventory" in product. Every `*_VIEW` / `*_COMMAND` /
///    `*_CAPABILITY` below is one of those constants. They are the reason the
///    `webui → product` **dependency itself cannot die in this row**, and
///    moving them is explicitly out of bounds (see the inventory test below).
/// 2. **`contract-purity`** — the symbol's own definition names a crate
///    `product_contracts` may not depend on (`ironclaw_host_api` +
///    `ironclaw_extension_contracts` is the whole allowlist, pinned by
///    `reborn_dependency_boundaries.rs`).
const PRODUCT_SYMBOLS_WEBUI_STILL_NAMES: &[(&str, &str)] = &[
    // --- contract-purity residue ------------------------------------------
    (
        "RebornCreateThreadResponse",
        "contract-purity: carries ironclaw_threads::SessionThreadRecord",
    ),
    (
        "RebornListThreadsResponse",
        "contract-purity: carries ironclaw_threads::SessionThreadRecord",
    ),
    (
        "RebornTimelineResponse",
        "contract-purity: carries ironclaw_threads::{SessionThreadRecord, \
         ThreadMessageRecord, SummaryArtifact}",
    ),
    (
        "RebornExtensionInfo",
        "contract-purity: auth_accounts carry ironclaw_auth::{AuthAccountState, \
         AuthAccountLastError}",
    ),
    (
        "RebornExtensionListResponse",
        "contract-purity: a Vec<RebornExtensionInfo> (see above)",
    ),
    (
        "RebornGetRunStateResponse",
        "contract-purity: carries ironclaw_common::llm_costs::RunCost and \
         ironclaw_loop_contracts::LoopModelUsage",
    ),
    (
        "RebornExecuteProductCommandResponse",
        "contract-purity: carries crate::commands::CommandResultView, the command \
         grammar §6.9.1 keeps in product",
    ),
    (
        "RebornRunArtifact",
        "contract-purity: carries ironclaw_threads records plus RunCost/LoopModelUsage",
    ),
    (
        "RebornThreadArtifact",
        "contract-purity: carries ironclaw_threads::ThreadMessageRecord",
    ),
    // --- frozen inventory (§6.1.3) ----------------------------------------
    ("ADMIN_CONFIGURATION_REPLACE_CAPABILITY", "inventory"),
    ("ADMIN_CONFIGURATION_VIEW", "inventory"),
    // Admin thread scraping (#7228): the three scrape views are part of the
    // same frozen view inventory as ADMIN_USERS_VIEW / ADMIN_USER_SECRETS_VIEW;
    // concrete descriptor constants stay in product per §6.1.3 (contracts
    // crate holds only the ProductView shape, never concrete inventory).
    ("ADMIN_THREAD_SCRAPE_ARTIFACT_VIEW", "inventory"),
    ("ADMIN_THREAD_SCRAPE_RUN_ARTIFACT_VIEW", "inventory"),
    ("ADMIN_THREAD_SCRAPE_THREADS_VIEW", "inventory"),
    ("ADMIN_USERS_VIEW", "inventory"),
    ("ADMIN_USER_CREATE_COMMAND", "inventory"),
    ("ADMIN_USER_DELETE_CAPABILITY", "inventory"),
    ("ADMIN_USER_DELETE_SECRET_COMMAND", "inventory"),
    ("ADMIN_USER_PUT_SECRET_CAPABILITY", "inventory"),
    ("ADMIN_USER_SECRETS_VIEW", "inventory"),
    ("ADMIN_USER_SET_ROLE_CAPABILITY", "inventory"),
    ("ADMIN_USER_SET_STATUS_CAPABILITY", "inventory"),
    ("ADMIN_USER_UPDATE_CAPABILITY", "inventory"),
    ("ADMIN_USER_VIEW", "inventory"),
    ("ATTACHMENT_READ_COMMAND", "inventory"),
    ("AUTOMATIONS_VIEW", "inventory"),
    ("AUTOMATION_DELETE_COMMAND", "inventory"),
    ("AUTOMATION_PAUSE_COMMAND", "inventory"),
    ("AUTOMATION_RENAME_COMMAND", "inventory"),
    ("AUTOMATION_RESUME_COMMAND", "inventory"),
    ("CANCEL_RUN_COMMAND", "inventory"),
    ("CREATE_THREAD_COMMAND", "inventory"),
    ("EXTENSIONS_VIEW", "inventory"),
    ("EXTENSION_IMPORT_CAPABILITY", "inventory"),
    ("EXTENSION_INSTALL_CAPABILITY", "inventory"),
    ("EXTENSION_REGISTER_HOSTED_MCP_CAPABILITY", "inventory"),
    ("EXTENSION_REGISTRY_VIEW", "inventory"),
    ("EXTENSION_REMOVE_CAPABILITY", "inventory"),
    ("EXTENSION_SETUP_SUBMIT_CAPABILITY", "inventory"),
    ("EXTENSION_SETUP_VIEW", "inventory"),
    ("FS_LIST_VIEW", "inventory"),
    ("FS_MOUNTS_VIEW", "inventory"),
    ("FS_READ_COMMAND", "inventory"),
    ("FS_STAT_VIEW", "inventory"),
    ("GLOBAL_AUTO_APPROVE_VIEW", "inventory"),
    ("LLM_ACTIVE_SET_CAPABILITY", "inventory"),
    ("LLM_CODEX_LOGIN_COMMAND", "inventory"),
    ("LLM_CONFIG_VIEW", "inventory"),
    ("LLM_LIST_MODELS_COMMAND", "inventory"),
    ("LLM_NEARAI_LOGIN_COMMAND", "inventory"),
    ("LLM_NEARAI_WALLET_LOGIN_COMMAND", "inventory"),
    ("LLM_PROVIDER_DELETE_CAPABILITY", "inventory"),
    ("LLM_PROVIDER_UPSERT_CAPABILITY", "inventory"),
    ("LLM_TEST_CONNECTION_COMMAND", "inventory"),
    ("LOGS_VIEW", "inventory"),
    ("OPERATOR_CONFIG_KEY_VIEW", "inventory"),
    ("OPERATOR_CONFIG_LIST_VIEW", "inventory"),
    ("OPERATOR_CONFIG_SET_AUTO_APPROVE_CAPABILITY", "inventory"),
    ("OPERATOR_CONFIG_SET_KEY_COMMAND", "inventory"),
    ("OPERATOR_CONFIG_VALIDATE_VIEW", "inventory"),
    ("OPERATOR_DIAGNOSTICS_VIEW", "inventory"),
    ("OPERATOR_LOGS_VIEW", "inventory"),
    ("OPERATOR_SERVICE_LIFECYCLE_COMMAND", "inventory"),
    ("OPERATOR_SETUP_RUN_CAPABILITY", "inventory"),
    ("OPERATOR_SETUP_VIEW", "inventory"),
    ("OPERATOR_STATUS_VIEW", "inventory"),
    ("OUTBOUND_DELIVERY_TARGETS_VIEW", "inventory"),
    ("NOTIFICATION_CHANNELS_SET_COMMAND", "inventory"),
    ("NOTIFICATION_CHANNELS_VIEW", "inventory"),
    ("PRODUCT_COMMAND_EXECUTE_COMMAND", "inventory"),
    ("PRODUCT_COMMAND_LIST_COMMAND", "inventory"),
    ("PROJECTS_VIEW", "inventory"),
    ("PROJECT_CREATE_COMMAND", "inventory"),
    ("PROJECT_DELETE_CAPABILITY", "inventory"),
    ("PROJECT_FS_LIST_VIEW", "inventory"),
    ("PROJECT_FS_READ_COMMAND", "inventory"),
    ("PROJECT_FS_STAT_VIEW", "inventory"),
    ("PROJECT_MEMBERS_VIEW", "inventory"),
    ("PROJECT_MEMBER_ADD_CAPABILITY", "inventory"),
    ("PROJECT_MEMBER_REMOVE_CAPABILITY", "inventory"),
    ("PROJECT_MEMBER_UPDATE_CAPABILITY", "inventory"),
    ("PROJECT_UPDATE_CAPABILITY", "inventory"),
    ("PROJECT_VIEW", "inventory"),
    ("RESOLVE_GATE_COMMAND", "inventory"),
    ("RETRY_RUN_COMMAND", "inventory"),
    ("RUN_ARTIFACT_VIEW", "inventory"),
    ("SKILLS_VIEW", "inventory"),
    ("SKILL_AUTO_ACTIVATE_LEARNED_SET_CAPABILITY", "inventory"),
    ("SKILL_AUTO_ACTIVATE_SET_CAPABILITY", "inventory"),
    ("SKILL_CONTENT_VIEW", "inventory"),
    ("SKILL_INSTALL_CAPABILITY", "inventory"),
    ("SKILL_REMOVE_CAPABILITY", "inventory"),
    ("SKILL_SEARCH_VIEW", "inventory"),
    ("SKILL_UPDATE_CAPABILITY", "inventory"),
    ("SUBMIT_TURN_COMMAND", "inventory"),
    ("THREADS_VIEW", "inventory"),
    ("THREAD_ARTIFACT_VIEW", "inventory"),
    ("THREAD_DELETE_CAPABILITY", "inventory"),
    ("TIMELINE_VIEW", "inventory"),
    ("TRACE_ACCOUNT_LOGIN_LINK_COMMAND", "inventory"),
    ("TRACE_ACCOUNT_TRACES_VIEW", "inventory"),
    ("TRACE_CREDITS_VIEW", "inventory"),
    ("TRACE_HOLD_AUTHORIZE_COMMAND", "inventory"),
];

/// The OpenAI-compatible adapter's whole surviving edge: three command
/// descriptors from the frozen inventory. Every DTO it speaks now comes from
/// `ironclaw_product_contracts`.
const PRODUCT_SYMBOLS_OPENAI_COMPAT_STILL_NAMES: &[(&str, &str)] = &[
    ("CANCEL_RUN_COMMAND", "inventory"),
    ("CREATE_THREAD_COMMAND", "inventory"),
    ("SUBMIT_TURN_COMMAND", "inventory"),
];

/// Ceilings on the two residues. Only ever move down.
// 102 when the WS5 transport inversion landed; **100** after the WS5
// `attachments widened` row moved `ProductAttachmentCapabilities` /
// `product_attachment_capabilities` to `ironclaw_attachments` as
// `AttachmentCapabilities` / `attachment_capabilities` — the two symbols this
// list called out by name as that row's to own. Shrink-only.
const WEBUI_PRODUCT_SYMBOL_BASELINE: usize = 103;
const OPENAI_COMPAT_PRODUCT_SYMBOL_BASELINE: usize = 3;

/// Boundary vocabulary this row moved: declared in `ironclaw_product_contracts`
/// and no longer *declared* in `ironclaw_assistant`. (Product still re-exports
/// most of it for its own consumers; a re-export is not a declaration, and the
/// difference is the whole point — the transports import from the owner.)
const INVERTED_BOUNDARY_TYPES: &[&str] = &[
    // descriptors — §6.1.3's "command/view/capability descriptor types"
    "EmptyProductCommandInput",
    "ProductCapabilityDescriptor",
    "ProductSurfaceCommandDescriptor",
    "ProductView",
    // inbound request bodies
    "ProductCancelReason",
    "ProductCancelRunRequest",
    "ProductCreateThreadRequest",
    "ProductGateResolution",
    "ProductInboundAttachment",
    "ProductListAutomationsRequest",
    "ProductListThreadsRequest",
    "ProductRenameAutomationRequest",
    "ProductResolveGateRequest",
    "ProductRetryRunRequest",
    "ProductSetupExtensionRequest",
    "ProductSubmitTurnRequest",
    // product wire DTOs
    "RebornCancelRunResponse",
    "RebornExtensionActionResponse",
    "RebornResolveGateResponse",
    "RebornRunArtifactRequest",
    "RebornStreamEventsResponse",
    "RebornSubmitTurnResponse",
    "RebornThreadArtifactRequest",
    "SettingsToolPermissionState",
    // operator/admin/workspace wire DTOs
    "LlmConfigSnapshot",
    "ProjectFsFile",
    "RebornAdminUserListResponse",
    "RebornFsListResponse",
    "RebornProjectResponse",
    "RebornTraceCreditsResponse",
    "UpsertLlmProviderRequest",
];

/// Concrete inventory constants §6.1.3 keeps in `ironclaw_assistant`. A sample,
/// one per family, asserted in the *opposite* direction from the moves above:
/// declared in product, never in contracts.
const FROZEN_INVENTORY_CONSTANTS: &[&str] = &[
    "SUBMIT_TURN_COMMAND",
    "THREADS_VIEW",
    "EXTENSION_INSTALL_CAPABILITY",
    "PROJECT_VIEW",
    "OPERATOR_STATUS_VIEW",
];

/// Normalization that could not follow the request bodies into contracts and
/// must stay in `ironclaw_assistant` (§6.1.3 "must never contain … any
/// handler/admission logic").
const NORMALIZATION_THAT_STAYED_IN_PRODUCT: &[&str] = &[
    "IntoProductInboundCommand",
    "DecodeInboundAttachments",
    "ProductInboundCommand",
];

/// Workspace package metadata, resolved once per test binary.
///
/// Same reason as `reborn_extension_host_port_inversion.rs`: WS7 moves these
/// crates into family directories, and a literal `crates/<name>` would then
/// resolve to nothing. Through cargo, a crate that is not a workspace member
/// is a panic and one that moved is simply followed.
fn workspace_metadata() -> &'static Value {
    static METADATA: OnceLock<Value> = OnceLock::new();
    METADATA.get_or_init(|| {
        let manifest_path = workspace_root().join("Cargo.toml");
        let output = Command::new("cargo")
            .args([
                "metadata",
                "--format-version",
                "1",
                "--no-deps",
                "--manifest-path",
            ])
            .arg(&manifest_path)
            .output()
            .unwrap_or_else(|error| panic!("failed to run cargo metadata: {error}"));
        assert!(
            output.status.success(),
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("cargo metadata output must be JSON")
    })
}

/// A workspace crate's directory, taken from its manifest's real location.
fn crate_dir(name: &str) -> PathBuf {
    let package = workspace_metadata()["packages"]
        .as_array()
        .expect("cargo metadata packages")
        .iter()
        .find(|package| package["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("{name} is not a workspace member"));
    let manifest = package["manifest_path"]
        .as_str()
        .unwrap_or_else(|| panic!("{name} has no manifest_path in cargo metadata"));
    Path::new(manifest)
        .parent()
        .unwrap_or_else(|| panic!("{name}'s manifest_path has no parent directory"))
        .to_path_buf()
}

fn crate_src(name: &str) -> PathBuf {
    crate_dir(name).join("src")
}

fn items_defined_in(crate_name: &str) -> BTreeSet<String> {
    let mut found: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    collect_type_defs(
        &crate_src(crate_name),
        &[
            "struct ", "enum ", "trait ", "const ", "type ", "fn ", "static ",
        ],
        &is_rust_identifier,
        &[],
        &mut found,
    );
    assert!(
        !found.is_empty(),
        "no items discovered in {crate_name} — the walk is broken, not the crate"
    );
    found.into_keys().collect()
}

/// Every `ironclaw_assistant::…` symbol a source names, whether through a braced
/// `use` group, a single `use`, or an inline path qualifier.
///
/// `ironclaw_product_contracts::` must never match: the crate name is a strict
/// prefix of it, so the scan requires `::` immediately after `ironclaw_assistant`
/// and rejects an identifier character there.
fn product_symbols_in(source: &str) -> BTreeSet<String> {
    const CRATE: &str = "ironclaw_assistant";
    // Comments and strings go first, so a brace inside either cannot
    // desynchronise the `#[cfg(test)]` brace matching that runs next. Getting
    // this backwards is a silent miscount — the depth walk overruns the gated
    // block and truncates the rest of the file. (Same ordering, for the same
    // reason, as `reborn_extension_host_port_inversion.rs`.)
    let cleaned = strip_cfg_test_blocks(&strip_comments_and_strings(source));
    let bytes = cleaned.as_bytes();
    let mut names = BTreeSet::new();
    let mut search_from = 0usize;
    while let Some(rel) = cleaned[search_from..].find(CRATE) {
        let at = search_from + rel;
        search_from = at + CRATE.len();
        // Reject `some_ironclaw_product` and `ironclaw_product_contracts`.
        if at > 0 {
            let prev = bytes[at - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                continue;
            }
        }
        let after = &cleaned[at + CRATE.len()..];
        let Some(tail) = after.strip_prefix("::") else {
            continue;
        };
        let tail = tail.trim_start();
        if let Some(group) = tail.strip_prefix('{') {
            let Some(close) = group.find('}') else {
                continue;
            };
            for raw in group[..close].split(',') {
                let name = raw.split(" as ").next().unwrap_or(raw).trim();
                if is_rust_identifier(name) {
                    names.insert(name.to_string());
                }
            }
        } else {
            let name: String = tail
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                .collect();
            if is_rust_identifier(&name) {
                names.insert(name);
            }
        }
    }
    names
}

fn product_symbols_named_by(crate_name: &str) -> (BTreeSet<String>, usize) {
    let files = production_rust_files(&crate_src(crate_name));
    assert!(
        files.len() > 5,
        "expected to walk {crate_name}'s source tree; found {} files",
        files.len()
    );
    let mut names = BTreeSet::new();
    for file in &files {
        let source = std::fs::read_to_string(file)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", file.display()));
        names.extend(product_symbols_in(&source));
    }
    (names, files.len())
}

fn assert_residue(
    crate_name: &str,
    found: &BTreeSet<String>,
    frozen_rows: &[(&str, &str)],
    baseline: usize,
) {
    let frozen: BTreeSet<String> = frozen_rows
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect();
    let mut violations = Vec::new();
    for name in found.difference(&frozen) {
        violations.push(format!(
            "{crate_name} names {PRODUCT}::{name}. A transport consumes the product \
             boundary, not the product crate: declare it in {PRODUCT_CONTRACTS} \
             (PROPOSAL §6.1.3) instead of adding a row here"
        ));
    }
    for name in frozen.difference(found) {
        violations.push(format!(
            "{name} is listed as {crate_name} residue but the crate no longer names \
             {PRODUCT}::{name} — delete its row in the same change"
        ));
    }
    assert!(
        violations.is_empty(),
        "WS5 transport port-inversion rule violated ({crate_name}):\n{}",
        violations.join("\n")
    );
    assert!(
        found.len() <= baseline,
        "{crate_name}'s product-symbol residue is shrink-only: {} > baseline {}",
        found.len(),
        baseline
    );
}

#[test]
fn transports_name_only_the_frozen_residue_of_product_symbols() {
    let (webui, webui_files) = product_symbols_named_by(WEBUI);
    assert_residue(
        WEBUI,
        &webui,
        PRODUCT_SYMBOLS_WEBUI_STILL_NAMES,
        WEBUI_PRODUCT_SYMBOL_BASELINE,
    );

    let (compat, compat_files) = product_symbols_named_by(OPENAI_COMPAT);
    assert_residue(
        OPENAI_COMPAT,
        &compat,
        PRODUCT_SYMBOLS_OPENAI_COMPAT_STILL_NAMES,
        OPENAI_COMPAT_PRODUCT_SYMBOL_BASELINE,
    );

    // Non-vacuity: a broken path or an over-eager `#[cfg(test)]` strip would
    // make both sets empty and every assertion above trivially true.
    assert!(
        webui_files > 20 && compat_files > 10,
        "the scan must walk both crates: webui {webui_files} files, compat {compat_files} files"
    );
    assert!(
        !webui.is_empty() && !compat.is_empty(),
        "both residues are non-empty today (the frozen inventory); an empty scan \
         means the matcher stopped matching, not that the edge died"
    );
}

#[test]
fn inverted_boundary_vocabulary_is_declared_in_contracts_and_not_in_product() {
    let contracts = items_defined_in(PRODUCT_CONTRACTS);
    let product = items_defined_in(PRODUCT);

    let mut violations = Vec::new();
    for name in INVERTED_BOUNDARY_TYPES {
        if !contracts.contains(*name) {
            violations.push(format!(
                "{name} must be declared in {PRODUCT_CONTRACTS}; the WS5 inversion moved it there"
            ));
        }
        if product.contains(*name) {
            violations.push(format!(
                "{name} is declared again in {PRODUCT} — the inversion gives it exactly \
                 one declaration (a `pub use` re-export is fine and is not this)"
            ));
        }
    }
    for name in NORMALIZATION_THAT_STAYED_IN_PRODUCT {
        if !product.contains(*name) {
            violations.push(format!(
                "{name} must stay declared in {PRODUCT}: it is normalization, not vocabulary"
            ));
        }
        if contracts.contains(*name) {
            violations.push(format!(
                "{name} appeared in {PRODUCT_CONTRACTS}; §6.1.3 forbids handler/admission \
                 logic there"
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "WS5 inverted-vocabulary placement violated (PROPOSAL §6.1.3):\n{}",
        violations.join("\n")
    );
}

#[test]
fn the_frozen_operation_inventory_stays_in_product() {
    let contracts = items_defined_in(PRODUCT_CONTRACTS);
    let product = items_defined_in(PRODUCT);

    let mut violations = Vec::new();
    for name in FROZEN_INVENTORY_CONSTANTS {
        if !product.contains(*name) {
            violations.push(format!(
                "{name} is no longer declared in {PRODUCT}; §6.1.3 keeps the concrete \
                 command/view/capability constants there as the frozen inventory"
            ));
        }
        if contracts.contains(*name) {
            violations.push(format!(
                "{name} was moved into {PRODUCT_CONTRACTS}. §6.1.3 grants that crate the \
                 descriptor *types*, explicitly \"not product's concrete constants\" — \
                 shrinking the residue this way hands the contract crate product's surface \
                 inventory and is not the fix"
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "WS5 frozen-inventory rule violated (PROPOSAL §6.1.3):\n{}",
        violations.join("\n")
    );
}

#[test]
fn import_scanner_reads_symbols_out_of_real_use_shapes() {
    let source = r#"
        use ironclaw_assistant::{Grouped, AlsoGrouped as Renamed};
        use ironclaw_assistant::Single;
        fn f(x: &ironclaw_assistant::Qualified) -> ironclaw_assistant::AlsoQualified { }
        use ironclaw_product_contracts::surface::NotProduct;
        use ironclaw_product_contracts::inbound_requests::{AlsoNotProduct};
        use my_ironclaw_product::NotOurs;
        // use ironclaw_assistant::Commented;
        #[cfg(test)]
        mod tests {
            use ironclaw_assistant::TestOnly;
        }
    "#;
    let found = product_symbols_in(source);
    assert!(found.contains("Grouped"), "grouped use: {found:?}");
    assert!(
        found.contains("AlsoGrouped"),
        "aliased group member keeps its source name: {found:?}"
    );
    assert!(found.contains("Single"), "single use: {found:?}");
    assert!(found.contains("Qualified"), "inline path: {found:?}");
    assert!(
        found.contains("AlsoQualified"),
        "inline path in return position: {found:?}"
    );
    assert!(
        !found.contains("NotProduct") && !found.contains("AlsoNotProduct"),
        "ironclaw_product_contracts is a different crate whose name this one prefixes: {found:?}"
    );
    assert!(
        !found.contains("NotOurs"),
        "a crate ending in ironclaw_assistant is not ironclaw_assistant: {found:?}"
    );
    assert!(
        !found.contains("Commented"),
        "commented-out import must not match: {found:?}"
    );
    assert!(
        !found.contains("TestOnly"),
        "a #[cfg(test)] import is not a production edge: {found:?}"
    );
    assert!(
        !found.contains("Renamed"),
        "the alias is local; the residue list is keyed by the exported name: {found:?}"
    );
}

/// CHECKLIST WS5's `webui` row ("gains pairing routes") and its WS2 twin
/// ("relocate extension_host strays: `channel_pairing_serve.rs` Axum routes →
/// `webui`"), executed 2026-08-02 and pinned here.
///
/// PROPOSAL §6.8.2 sheds "Axum pairing routes → `webui`" and §6.9.4 has this
/// crate gain them. The reason it matters beyond tidiness: the extension host
/// re-layers `products` → `loops` (§12.1c), and a crate below product cannot
/// own a bearer-authed product route surface. Its **only** remaining Axum
/// surface is the vendor-blind channel *ingress* router
/// (`extension_ingress::serve_mount`), which is its job by charter.
///
/// Stated as a path-prefix rule rather than a module-name rule so
/// reintroducing the routes under any other filename fails too. This is not a
/// workspace-wide "only webui serves `/api/webchat/`" claim — `ironclaw_operator`
/// serves the NEAR AI login callback under that prefix by charter (§6.9.2) —
/// it is a claim about the crate the stray left.
#[test]
fn the_extension_host_serves_no_webui_product_routes_and_webui_holds_the_pairing_ones() {
    let root = workspace_root();
    let src = crate_src(EXTENSION_HOST);
    let files = production_rust_files(&src);
    assert!(
        files.len() >= 60,
        "expected to walk {EXTENSION_HOST}'s source tree; found {} files — a broken path \
         must fail loudly rather than report a clean, vacuously passing scan",
        files.len()
    );
    let offenders: Vec<String> = files
        .iter()
        .filter(|file| {
            std::fs::read_to_string(file)
                .unwrap_or_else(|error| panic!("read {}: {error}", file.display()))
                .contains("/api/webchat/")
        })
        .map(|file| {
            file.strip_prefix(&root)
                .unwrap_or(file)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "{EXTENSION_HOST} declares WebUI product routes in {offenders:?}. Those belong in \
         {WEBUI} (PROPOSAL §6.8.2 shed list, §6.9.4); the extension host's only route \
         surface is the generic channel ingress router"
    );

    // The positive half, so "no offenders" cannot be an artefact of the routes
    // having been deleted rather than moved.
    let pairing = crate_src(WEBUI).join("channel_pairing.rs");
    let source = std::fs::read_to_string(&pairing)
        .unwrap_or_else(|error| panic!("read {}: {error}", pairing.display()));
    for pattern in ["pairing/mint", "pairing/status", "pairing/unpair"] {
        assert!(
            source.contains(pattern),
            "{WEBUI}'s channel_pairing.rs must serve {pattern} — the WS2 stray moved here, \
             it was not dropped"
        );
    }
}

#[test]
fn the_scan_strips_comments_before_matching_cfg_test_braces() {
    // Ordering regression. `strip_cfg_test_blocks` counts braces byte by byte,
    // so it must never see a comment or a string literal. Run the wrong way
    // round, the stray `{` below pushes the depth to 2, the gated module's `}`
    // only brings it back to 1, and the walk runs off the end of the file —
    // dropping every production import that follows and shrinking the residue
    // silently. The `files.len()` and non-empty guards cannot catch that: the
    // sets stay non-empty, they just stop being complete.
    let source = r#"
        #[cfg(test)]
        mod tests {
            // an unbalanced brace in a comment: {
            const PATTERN: &str = "and another unbalanced { in a literal";
            use ironclaw_assistant::TestOnly;
        }
        use ironclaw_assistant::StillCounted;
    "#;
    let found = product_symbols_in(source);
    assert!(
        found.contains("StillCounted"),
        "production code after a gated block whose comments/strings carry braces \
         must still be scanned: {found:?}"
    );
    assert!(
        !found.contains("TestOnly"),
        "the gated import is still not a production edge: {found:?}"
    );
}
