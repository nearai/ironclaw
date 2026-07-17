//! Anti-slippage ratchet for the deployment-mode-as-type axis (§4.4 / §10 of
//! `docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`).
//!
//! §4.4's rule: **a deployment mode is a config value, never a type the kernel or
//! a substrate names.** Today a whole `LocalDev*` shadow runtime encodes local-dev
//! as a type family (approval/capability/lease/mount/network/outbound policy, the
//! store aliases, the root filesystem, the turn-state store). Slice B collapses
//! all of it to a `DeploymentConfig` value.
//!
//! That migration is incremental, so this test **freezes the current set of
//! `LocalDev*` type definitions** (struct/enum/trait/type alias, `pub` or
//! `pub(crate)`) and fails on any change:
//!
//! - a **new** `LocalDev*` type (not in the allowlist) fails — the deployment
//!   mode must resolve to policy data at the composition edge, not grow another
//!   type;
//! - **deleting** one without trimming [`FROZEN_LOCALDEV_TYPES`] also fails — so
//!   the allowlist shrinks in lock-step as Slice B lands (§10: compare set
//!   membership, never a count), and reviewers watch it get shorter.
//!
//! Definition of done for this axis (§4.4/§10): the allowlist reaches the empty
//! set — no `LocalDev*` type remains; local-dev is one `DeploymentConfig`
//! constant. (Scoped to `LocalDev*` specifically: the broader §4.4 `Local*` /
//! `Hosted*` name audit — Bucket 2 renames like `LocalFilesystem`→`DiskFilesystem`
//! and Bucket 3 false positives like `Locale`, `HostedMcp*`, `LocalTraceSubmission*`
//! — is a separate concern, so this ratchet stays high-signal with a clean
//! empty-set goal.)

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The frozen inventory of `LocalDev*` type definitions under `crates/`, as of
/// the store-consolidation ratchet (§10). Every entry is a deployment-mode-as-type
/// leak that Slice B removes by resolving mode to a `DeploymentConfig` value.
/// Remove an entry in the same PR that deletes its type; never add one.
const FROZEN_LOCALDEV_TYPES: &[&str] = &[
    "LocalDevActiveExtensionAuthorityForTest",
    "LocalDevApprovalDefaultsPolicy",
    "LocalDevApprovalGatePolicy",
    "LocalDevApprovalLeaseTermsProvider",
    "LocalDevApprovalPolicyAction",
    "LocalDevApprovalRequestStore",
    "LocalDevAuthInteractionReadModel",
    "LocalDevAutoApproveSettingStore",
    "LocalDevCapabilityGrantPolicy",
    "LocalDevCapabilityLeaseStore",
    "LocalDevCapabilityPolicy",
    "LocalDevCapabilityPolicyError",
    "LocalDevCapabilityWiring",
    "LocalDevConstraintPolicy",
    "LocalDevDurableBackend",
    "LocalDevExtensionSurface",
    "LocalDevExtensionSurfaceSource",
    "LocalDevMountProfile",
    "LocalDevNetworkProfile",
    "LocalDevOutboundStores",
    "LocalDevOverride",
    "LocalDevPersistentApprovalPolicyStore",
    "LocalDevProviderPolicy",
    "LocalDevRootFilesystem",
    "LocalDevSelectableSkillContextSource",
    "LocalDevSyntheticCapability",
    "LocalDevSyntheticCapabilityDescriptor",
    "LocalDevSyntheticCapabilityHandler",
    "LocalDevSyntheticCapabilityInvocation",
    "LocalDevToolPermissionOverrideStore",
    "LocalDevTurnStateStore",
];

#[test]
fn reborn_localdev_typename_allowlist_is_frozen_and_only_shrinks() {
    let crates_dir = workspace_root().join("crates");
    let mut found = BTreeSet::new();
    collect_localdev_type_defs(&crates_dir, &mut found);

    let frozen: BTreeSet<&str> = FROZEN_LOCALDEV_TYPES.iter().copied().collect();
    let found_refs: BTreeSet<&str> = found.iter().map(String::as_str).collect();

    let added: Vec<&&str> = found_refs.difference(&frozen).collect();
    assert!(
        added.is_empty(),
        "New `LocalDev*` type definitions are banned (arch-simplification §4.4/§10): a \
         deployment mode is a `DeploymentConfig` value, never a type. Offending new types: \
         {added:?}. Resolve the mode to policy data at the composition edge instead of \
         adding a type."
    );

    let removed: Vec<&&str> = frozen.difference(&found_refs).collect();
    assert!(
        removed.is_empty(),
        "FROZEN_LOCALDEV_TYPES lists types that no longer exist: {removed:?}. A LocalDev* \
         type was deleted (good — Slice B progress!) — trim it from the allowlist in the \
         same PR so the ratchet keeps shrinking toward empty (§10)."
    );
}

/// Self-test for the scanner: extract exactly the `LocalDev*` type definitions,
/// ignoring references, string literals, comments, and non-`LocalDev` types.
#[test]
fn localdev_type_def_scanner_self_test() {
    let sample = r#"
        pub struct LocalDevWidget { x: u8 }
        pub(crate) type LocalDevAlias = u8;
        pub enum LocalDevMode { A, B }
        pub trait LocalDevPort {}
        struct LocalDevPrivate;              // not pub / pub(crate) -> ignored
        pub struct HostedWidget;             // not LocalDev -> ignored
        let x = "LocalDevStringLiteral";     // string literal -> ignored
        // pub struct LocalDevCommented      // comment -> ignored (line-based)
        fn build_local_dev_runtime() {}      // fn, not a type -> ignored
    "#;
    let mut out = BTreeSet::new();
    scan_source_for_localdev_type_defs(sample, &mut out);
    let got: Vec<&str> = out.iter().map(String::as_str).collect();
    assert_eq!(
        got,
        vec![
            "LocalDevAlias",
            "LocalDevMode",
            "LocalDevPort",
            "LocalDevWidget",
        ],
        "scanner must match only `pub`/`pub(crate)` LocalDev* type definitions"
    );
}

fn collect_localdev_type_defs(dir: &Path, out: &mut BTreeSet<String>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}"));
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                continue;
            }
            collect_localdev_type_defs(&path, out);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        // This ratchet file's own self-test fixture contains sample `LocalDev*`
        // type lines; don't count them.
        if path.file_name().and_then(|n| n.to_str()) == Some("reborn_localdev_typename_ratchet.rs")
        {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        scan_source_for_localdev_type_defs(&contents, out);
    }
}

/// Line-based scan: pull the identifier from any `pub`/`pub(crate)`
/// `struct`/`enum`/`trait`/`type` definition whose name starts with `LocalDev`.
fn scan_source_for_localdev_type_defs(source: &str, out: &mut BTreeSet<String>) {
    for line in source.lines() {
        let mut rest = line.trim_start();
        let Some(after_pub) = rest.strip_prefix("pub") else {
            continue;
        };
        rest = after_pub.trim_start();
        // optional `(crate)` / `(super)` visibility scope
        if let Some(open) = rest.strip_prefix('(') {
            let Some(close) = open.find(')') else {
                continue;
            };
            rest = open[close + 1..].trim_start();
        }
        let mut matched = None;
        for kw in ["struct ", "enum ", "trait ", "type "] {
            if let Some(after_kw) = rest.strip_prefix(kw) {
                matched = Some(after_kw);
                break;
            }
        }
        let Some(after_kw) = matched else {
            continue;
        };
        let ident: String = after_kw
            .trim_start()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if ident.starts_with("LocalDev") {
            out.insert(ident);
        }
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("architecture crate must live under crates/ironclaw_architecture")
        .to_path_buf()
}
