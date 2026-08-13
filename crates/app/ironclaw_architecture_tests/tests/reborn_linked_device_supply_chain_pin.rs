//! Supply-chain pin for the in-process MTProto stack (`grammers-*`).
//!
//! **Why this gate exists, stated as a security control and not as dependency
//! hygiene.** The linked-device auth hook runs `grammers` *in our process*, so
//! the exposure is not bounded by the auth flow: a malicious or compromised
//! release can read the process heap — every other user's decrypted session
//! key, the secrets master key, provider credentials — open its own sockets,
//! and never consult our validation seam. The design accepted that trade
//! explicitly and rejected the out-of-process sidecar on cost
//! (`docs/internal/design/telegram-linked-device/ADR-device-link-auth-hook.md`,
//! "The larger trade this sits inside"). The price of that acceptance is that
//! the supply-chain controls ship **with** the dependency:
//!
//! > A dependency with full process authority and a caret version range is not
//! > a controlled dependency.
//!
//! **Why `=0.10.0` specifically, and why a bump is a design change.** `0.10.0`
//! is the last release in which `update_config` parses Telegram's server-pushed
//! datacenter list and then **discards** it without calling `set_dc_option`. That
//! is the whole reason the package's DC-address validation is airtight: every
//! address the dialer can reach comes from the compiled-in table or from a value
//! we wrote, so `Session::dc_option` on `IronclawSession` gates 100% of dials.
//! Upstream commit `5f94e83` ("Fix update_config did not set_dc_option") lands
//! *after* this release; adopting it starts flowing server-pushed addresses into
//! the session and silently deletes the seam. A version bump is therefore not a
//! routine refresh — the validation seam must be re-verified first, and this
//! gate is what forces a human to look. (PROPOSAL §3.4, §11.1.)
//!
//! **What is pinned, and where each half is measured.** Three surfaces, because
//! no single file carries all of it:
//!
//! 1. **`Cargo.lock`** — version *and* `.crate` archive checksum *and* registry
//!    source, for every `grammers-*` package in the graph. The checksum is the
//!    half that matters against the threat PROPOSAL §11.1 names: a crates.io
//!    account compromise republishing a different `0.10.z`. Only three of the
//!    eight are direct edges (`-client`, `-session`, `-tl-types`); the other
//!    five (`-crypto`, `-mtproto`, `-mtsender`, `-tl-gen`, `-tl-parser`) are
//!    transitive and *cannot* be `=`-pinned from a manifest at all. The lockfile
//!    is their only pin, which is precisely why this gate had to be built.
//! 2. **`cargo metadata`'s resolved feature sets** — under both the default and
//!    the `--all-features` resolution, because CI runs an `--all-features`
//!    clippy lane. Declaration is not resolution: Cargo unions features across
//!    every edge, so `grammers-tl-types` resolves with `tl-mtproto` and
//!    `deserializable-functions` that this workspace never asks for. The
//!    resolved set is what compiles into the binary, so the resolved set is what
//!    is pinned.
//! 3. **The package manifest** — every `grammers-*` edge is written `=x.y.z`
//!    (never a caret range) with `default-features = false` and an explicit
//!    allowlist, so a future upstream release cannot widen our feature set by
//!    widening its own `default`.
//!
//! **The socks5 `proxy` feature has its own test, and it is not a style rule.**
//! `grammers-client/proxy` forwards to `grammers-mtsender/proxy`, which dials
//! through `tokio-socks`. A proxied dial never consults `Session::dc_option` —
//! the one seam our DC-address validation owns — so enabling it does not weaken
//! the "100% of dials are validated" claim, it falsifies it. The test asserts
//! the feature is off in the declared edges, off in both resolutions, *and*
//! that `grammers-mtsender`'s own locked dependency list contains none of the
//! crates the feature would pull in.
//!
//! ## Residual: there is no named-reviewer surface in this repository
//!
//! PROPOSAL §11.1's fourth control is *"put grammers on a named
//! dependency-review list requiring human diff review on every bump."* State the
//! gap plainly rather than implying it is closed:
//!
//! * **There is no `CODEOWNERS` file in this repository** (verified by this
//!   gate's sibling assertion below). Nothing can route a `grammers` diff to a
//!   *named* human, and this test cannot create that capability.
//! * What exists is `.github/dependabot.yml`, whose `everything-else` group
//!   matches `*` and would otherwise open bump PRs for this stack. The strongest
//!   available substitute is implemented and asserted here: an `ignore` entry
//!   removes `grammers-*` from bot proposals entirely, so a bump can only arrive
//!   as a deliberate human edit — and that edit is red until the same human also
//!   edits the pin table in this file, where the paragraphs above are what they
//!   have to read past.
//! * **That is a forcing function, not a review.** It guarantees a human touched
//!   the change; it does not guarantee anyone diffed upstream. Closing the
//!   remainder needs a `CODEOWNERS` entry (or `cargo vet`), and neither exists
//!   today. Recorded as the residual the ADR asked for.
//!
//! ## When you are here because you want to bump it
//!
//! Do all four, in this order, or do none:
//!
//! 1. Read `update_config` in the new release and confirm whether it now calls
//!    `set_dc_option`. If it does, the DC-validation seam is gone and the bump
//!    is blocked on redesigning it — not on this test.
//! 2. Diff the upstream source. That is the control this gate cannot perform.
//! 3. Update `PINNED_CRATES` (version, checksum, resolved features) and
//!    `DECLARED_EDGES` here, and the `[dependencies]` comment block in
//!    `crates/extensions/packages/telegram/Cargo.toml`.
//! 4. Re-derive the numbers rather than hand-editing them:
//!
//! ```text
//! cargo metadata --format-version 1 | \
//!   python3 -c 'import json,sys; m=json.load(sys.stdin); print("\n".join(
//!     f"{n[\"id\"]} {sorted(n[\"features\"])}" for n in m["resolve"]["nodes"] if "grammers" in n["id"]))'
//! ```
//!
//! Run this gate alone with:
//!
//! ```text
//! cargo test -p ironclaw_architecture_tests --test reborn_linked_device_supply_chain_pin
//! ```
//!
//! ## Do not "tidy" the `reborn_` prefix off these test names
//!
//! One of the CI lanes that runs this crate is
//! `cargo test -p ironclaw_architecture_tests reborn`
//! (`.github/workflows/code_style.yml`, the `reborn-cli-smoke` job's "Test
//! Reborn architecture boundaries" step — a change-gated push/merge-group lane,
//! not a PR lane). That trailing `reborn` is a **name filter**, not a package
//! run: a test function whose name does not contain `reborn` is silently not
//! executed there. `telegram_extension_gates.rs` is the standing example of a
//! gate that misses it, and it compensates by being registered by target name
//! in `scripts/reborn-e2e-rust.sh`.
//!
//! This file does both, deliberately: every test carries the prefix, *and* the
//! target is listed in that script's `architecture-boundaries` group. Renaming
//! a test here without checking both turns a security gate into a file nothing
//! runs.

#[allow(dead_code)]
mod ratchet_support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use ratchet_support::{crate_path, workspace_root};
use serde_json::Value;

// ---------------------------------------------------------------------------
// The pin
// ---------------------------------------------------------------------------

/// Every package in the graph whose name starts with this prefix is in scope.
/// Deriving the scope from the prefix rather than from the pin table is what
/// makes a *new* member of the stack fail the gate instead of arriving unseen.
const STACK_PREFIX: &str = "grammers-";

/// The registry every member must resolve from. A silent move to a git rev or a
/// vendored path is exactly the substitution a version-only pin cannot see.
const REGISTRY_SOURCE: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// One pinned member of the in-process MTProto stack.
#[derive(Debug, Clone, Copy)]
struct PinnedCrate {
    name: &'static str,
    version: &'static str,
    /// The `.crate` archive digest recorded in `Cargo.lock`. This is the field
    /// that survives a republished same-version release.
    checksum: &'static str,
    /// The **resolved** feature set (`cargo metadata`'s `resolve.nodes[].features`),
    /// not the set this workspace declares — see the module docs.
    features: &'static [&'static str],
}

/// Measured on this checkout, 2026-08-10, by the `cargo metadata` recipe in the
/// module docs plus `Cargo.lock`. Eight members: three direct edges and five
/// transitive ones that no manifest can pin.
///
/// `grammers-tl-parser` is deliberately at a *different* version line (1.2.2 —
/// it is the `.tl` grammar parser, versioned independently of the protocol
/// crates). A pin table keyed on "they are all 0.10.0" would have been wrong the
/// day it was written.
const PINNED_CRATES: &[PinnedCrate] = &[
    PinnedCrate {
        name: "grammers-client",
        version: "0.10.0",
        checksum: "0f330139772e71b5e104f5a7bbf43bbda92fd8a734b4cf9c57839e04e949cf9b",
        // Empty by design: `default = ["fs"]` gates the path-taking
        // `upload_file`/`download_media` helpers, and attachment bytes move
        // through `ironclaw_attachments` instead. A verified minimum — drop it
        // wrongly and the package stops compiling.
        features: &[],
    },
    PinnedCrate {
        name: "grammers-crypto",
        version: "0.10.0",
        checksum: "23fe0fbd93bce6965d08248abdfec2d2544e11cb8d5c91dba3369c0a4295fd85",
        features: &[],
    },
    PinnedCrate {
        name: "grammers-mtproto",
        version: "0.10.0",
        checksum: "2a8d68ffa402c5f0707e22c29d44229eb01e968017dd0fb27e1adab953d49fc3",
        features: &[],
    },
    PinnedCrate {
        name: "grammers-mtsender",
        version: "0.10.0",
        checksum: "f39b8556ddc94f7ec935135dcfbf84ca8628538e103779cdc701423b759b078e",
        // The crate that owns the socket, and the crate that owns `proxy`.
        // Empty is the whole point of the test below.
        features: &[],
    },
    PinnedCrate {
        name: "grammers-session",
        version: "0.10.0",
        checksum: "3e4d5b9e434c6b9fc1e091fe82d1f201d8e19df5005d2c19e218b4977b4df506",
        // `serde` supplies the derives the session-blob codec needs. Its
        // `default` is `sqlite-storage`, which would pull a database driver
        // into a channel package for a backend custody never uses.
        features: &["serde"],
    },
    PinnedCrate {
        name: "grammers-tl-gen",
        version: "0.10.0",
        checksum: "1a37dda68c9e775dfbffb85b99826b5ac8144e9da40be3327e77cbcd8618fc30",
        features: &[],
    },
    PinnedCrate {
        name: "grammers-tl-parser",
        version: "1.2.2",
        checksum: "646fc5eeb27461ffcc94fcf9fda3ffd9b402a20250447ae4a65c39c1a905e381",
        features: &[],
    },
    PinnedCrate {
        name: "grammers-tl-types",
        version: "0.10.0",
        checksum: "67d450e2f2588af535fe85a17b9a6df8338794ee8ad6bcb1b97d61a14d472fc5",
        // Wider than what this workspace declares, and that is the fact worth
        // recording: `grammers-client` enables `default` + `tl-mtproto` and
        // `grammers-mtsender` adds `deserializable-functions`, on edges we do
        // not own. Feature unification is why "we set default-features = false"
        // is not by itself an answer to "what got compiled".
        features: &[
            "default",
            "deserializable-functions",
            "impl-debug",
            "impl-from-enum",
            "impl-from-type",
            "tl-api",
            "tl-mtproto",
        ],
    },
];

/// One declared edge in the package manifest — the half a human writes.
#[derive(Debug, Clone, Copy)]
struct DeclaredEdge {
    name: &'static str,
    /// Must be the exact form. `^0.10` / `0.10` / `~0.10.0` all admit a
    /// republished-under-a-new-patch attack that never touches this repository.
    requirement: &'static str,
    /// Exactly the feature list the manifest may declare.
    features: &'static [&'static str],
}

const DECLARED_EDGES: &[DeclaredEdge] = &[
    DeclaredEdge {
        name: "grammers-client",
        requirement: "=0.10.0",
        features: &[],
    },
    DeclaredEdge {
        name: "grammers-session",
        requirement: "=0.10.0",
        features: &["serde"],
    },
    DeclaredEdge {
        name: "grammers-tl-types",
        requirement: "=0.10.0",
        features: &["impl-debug", "impl-from-enum", "impl-from-type", "tl-api"],
    },
];

/// The manifest that owns the direct edges, spelled the readable way and
/// resolved through the crate inventory so a package move repoints it.
const PACKAGE_MANIFEST: &str = "crates/extensions/packages/telegram/Cargo.toml";

/// The feature that must never be on, in any spelling, anywhere in the stack.
const FORBIDDEN_FEATURE: &str = "proxy";

/// The package whose `proxy` feature the forwarding chain bottoms out at.
const PROXY_OWNER: &str = "grammers-mtsender";

/// The crates `grammers-mtsender/proxy` turns on. Their presence in *that
/// package's* locked dependency list is lockfile-level evidence the feature is
/// enabled — a check that does not depend on `cargo metadata` running at all.
const PROXY_ENABLED_DEPENDENCIES: &[&str] = &["tokio-socks", "hickory-resolver", "url"];

/// The bot-update surface that would otherwise propose bumps for this stack.
const DEPENDABOT_CONFIG: &str = ".github/dependabot.yml";

// ---------------------------------------------------------------------------
// `Cargo.lock`
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct LockedPackage {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
    /// Raw `dependencies` entries, each `"name"`, `"name version"`, or
    /// `"name version (source)"`.
    dependencies: Vec<String>,
}

impl LockedPackage {
    /// Whether this package's locked dependency list names `dependency`,
    /// matching on the leading token so the version/source suffixes do not
    /// have to be modelled.
    fn depends_on(&self, dependency: &str) -> bool {
        self.dependencies
            .iter()
            .any(|entry| entry.split_whitespace().next() == Some(dependency))
    }
}

fn parse_lockfile(text: &str) -> Result<Vec<LockedPackage>, String> {
    let table: toml::Table = text
        .parse()
        .map_err(|error| format!("Cargo.lock is not valid TOML: {error}"))?;
    let packages = table
        .get("package")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| {
            "Cargo.lock has no [[package]] array — the lockfile shape changed under this gate, \
             and a scan that finds nothing must never read as clean"
                .to_string()
        })?;
    let mut out = Vec::new();
    for package in packages {
        let entry = package
            .as_table()
            .ok_or_else(|| "a [[package]] entry is not a table".to_string())?;
        let name = entry
            .get("name")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| "a [[package]] entry has no name".to_string())?
            .to_string();
        let version = entry
            .get("version")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| format!("[[package]] {name} has no version"))?
            .to_string();
        out.push(LockedPackage {
            name,
            version,
            source: entry
                .get("source")
                .and_then(toml::Value::as_str)
                .map(ToString::to_string),
            checksum: entry
                .get("checksum")
                .and_then(toml::Value::as_str)
                .map(ToString::to_string),
            dependencies: entry
                .get("dependencies")
                .and_then(toml::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(toml::Value::as_str)
                        .map(ToString::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        });
    }
    Ok(out)
}

/// The stack's locked entries, keyed by name.
///
/// Fails closed twice, because both failures would otherwise read as a pass:
/// an empty result (the dependency is gone, or the scan lost the file) and a
/// name locked at two versions (a second copy of an in-process MTProto stack is
/// never intentional).
fn stack_entries(packages: &[LockedPackage]) -> Result<BTreeMap<String, LockedPackage>, String> {
    let mut out: BTreeMap<String, LockedPackage> = BTreeMap::new();
    for package in packages
        .iter()
        .filter(|package| package.name.starts_with(STACK_PREFIX))
    {
        if let Some(previous) = out.insert(package.name.clone(), package.clone()) {
            return Err(format!(
                "{} is locked at two versions ({} and {}). Two copies of the in-process MTProto \
                 stack means one of them is unpinned and unreviewed; resolve to a single version \
                 before this gate can say anything useful.",
                package.name, previous.version, package.version
            ));
        }
    }
    if out.is_empty() {
        return Err(format!(
            "no {STACK_PREFIX}* package is present in Cargo.lock. If the linked-device \
             dependency was removed deliberately, delete this gate in the same change and say so \
             in the PR — leaving a supply-chain pin that measures nothing is worse than having \
             none, because it reads as a control."
        ));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Resolved features (`cargo metadata`)
// ---------------------------------------------------------------------------

fn cargo_metadata(root: &Path, all_features: bool) -> Value {
    let mut command = Command::new("cargo");
    command.args(["metadata", "--format-version", "1"]);
    if all_features {
        command.arg("--all-features");
    }
    let output = command
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .output()
        .expect("cargo metadata must run");
    assert!(
        output.status.success(),
        "cargo metadata (all_features={all_features}) failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata must be valid JSON")
}

/// `name -> (version, resolved features)` for every stack member, read from the
/// `packages` array (which carries `name`/`version` as fields) joined to
/// `resolve.nodes` by id — so no package-id string format has to be parsed.
fn resolved_stack(root: &Path, all_features: bool) -> BTreeMap<String, (String, BTreeSet<String>)> {
    let metadata = cargo_metadata(root, all_features);
    let mut identities: BTreeMap<String, (String, String)> = BTreeMap::new();
    for package in metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages")
    {
        let (Some(id), Some(name), Some(version)) = (
            package["id"].as_str(),
            package["name"].as_str(),
            package["version"].as_str(),
        ) else {
            continue;
        };
        if name.starts_with(STACK_PREFIX) {
            identities.insert(id.to_string(), (name.to_string(), version.to_string()));
        }
    }

    let mut out = BTreeMap::new();
    for node in metadata["resolve"]["nodes"]
        .as_array()
        .expect("cargo metadata must include a resolve graph")
    {
        let Some(id) = node["id"].as_str() else {
            continue;
        };
        let Some((name, version)) = identities.get(id) else {
            continue;
        };
        let features = node["features"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToString::to_string)
                    .collect::<BTreeSet<String>>()
            })
            .unwrap_or_default();
        out.insert(name.clone(), (version.clone(), features));
    }
    assert!(
        !out.is_empty(),
        "cargo metadata (all_features={all_features}) resolved no {STACK_PREFIX}* package. A \
         feature pin that measures an empty graph is not a pin — see the Cargo.lock half of this \
         gate for the deliberate-removal path."
    );
    out
}

// ---------------------------------------------------------------------------
// The declared edges (`Cargo.toml`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeclaredDependency {
    name: String,
    table: String,
    requirement: Option<String>,
    default_features: Option<bool>,
    features: Vec<String>,
    /// `true` for the `name = "req"` shorthand, which cannot carry
    /// `default-features = false` at all — so it is always a violation here,
    /// with a message that says why rather than "expected false, got None".
    shorthand: bool,
}

/// Every dependency in `manifest` whose name starts with `prefix`, across the
/// three dependency tables. `[dev-dependencies]` and `[build-dependencies]` are
/// included on purpose: resolver-v2 unifies dev-dependency features into the
/// same build when tests are compiled, so a `proxy`-enabling dev edge would turn
/// the feature on for the very lane CI runs.
fn parse_declared_dependencies(
    manifest: &str,
    prefix: &str,
) -> Result<Vec<DeclaredDependency>, String> {
    let parsed: toml::Table = manifest
        .parse()
        .map_err(|error| format!("manifest is not valid TOML: {error}"))?;
    let mut out = Vec::new();
    for table_name in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let Some(table) = parsed.get(table_name).and_then(toml::Value::as_table) else {
            continue;
        };
        for (name, value) in table {
            if !name.starts_with(prefix) {
                continue;
            }
            let declared = match value {
                toml::Value::String(requirement) => DeclaredDependency {
                    name: name.clone(),
                    table: table_name.to_string(),
                    requirement: Some(requirement.clone()),
                    default_features: None,
                    features: Vec::new(),
                    shorthand: true,
                },
                toml::Value::Table(entry) => DeclaredDependency {
                    name: name.clone(),
                    table: table_name.to_string(),
                    requirement: entry
                        .get("version")
                        .and_then(toml::Value::as_str)
                        .map(ToString::to_string),
                    default_features: entry.get("default-features").and_then(toml::Value::as_bool),
                    features: entry
                        .get("features")
                        .and_then(toml::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(toml::Value::as_str)
                                .map(ToString::to_string)
                                .collect()
                        })
                        .unwrap_or_default(),
                    shorthand: false,
                },
                other => {
                    return Err(format!(
                        "dependency {name} in [{table_name}] has an unexpected shape: {other}"
                    ));
                }
            };
            out.push(declared);
        }
    }
    out.sort_by(|left, right| (&left.table, &left.name).cmp(&(&right.table, &right.name)));
    Ok(out)
}

fn read(root: &Path, relative: &str) -> String {
    let path = root.join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

// ---------------------------------------------------------------------------
// Gate 1 — versions, checksums, and the registry, from `Cargo.lock`
// ---------------------------------------------------------------------------

#[test]
fn reborn_linked_device_versions_and_checksums_are_pinned_in_the_lockfile() {
    let root = workspace_root();
    let packages =
        parse_lockfile(&read(&root, "Cargo.lock")).unwrap_or_else(|error| panic!("{error}"));
    let locked = stack_entries(&packages).unwrap_or_else(|error| panic!("{error}"));

    let pinned_names: BTreeSet<&str> = PINNED_CRATES.iter().map(|pin| pin.name).collect();
    let locked_names: BTreeSet<&str> = locked.keys().map(String::as_str).collect();
    assert_eq!(
        locked_names, pinned_names,
        "the {STACK_PREFIX}* package set changed. A member that appears here arrived with no \
         version review, no checksum pin, and full process authority; a member that disappears \
         means the pin below is now describing crates that are not built. Update PINNED_CRATES in \
         the same change, and read the bump checklist in this file's module docs first."
    );

    let mut violations = Vec::new();
    for pin in PINNED_CRATES {
        let Some(entry) = locked.get(pin.name) else {
            continue;
        };
        if entry.version != pin.version {
            violations.push(format!(
                "{}: locked at {} but pinned at {} — a version move is a design change here, not \
                 a refresh (see the `update_config`/`set_dc_option` note in the module docs)",
                pin.name, entry.version, pin.version
            ));
        }
        match entry.checksum.as_deref() {
            Some(checksum) if checksum == pin.checksum => {}
            Some(checksum) => violations.push(format!(
                "{}: locked checksum {checksum} does not match the pinned {}. Same version, \
                 different archive: this is the republished-release case the pin exists for. Do \
                 not update the constant until the upstream diff has been read.",
                pin.name, pin.checksum
            )),
            None => violations.push(format!(
                "{}: Cargo.lock records no checksum, so the archive is unverifiable. A registry \
                 dependency always has one — this means the source moved to a git rev or a path.",
                pin.name
            )),
        }
        match entry.source.as_deref() {
            Some(source) if source == REGISTRY_SOURCE => {}
            other => violations.push(format!(
                "{}: source is {:?}, expected {REGISTRY_SOURCE}. A source swap substitutes the \
                 code wholesale while every version string stays identical.",
                pin.name, other
            )),
        }
    }
    assert!(
        violations.is_empty(),
        "the in-process MTProto stack drifted from its supply-chain pin:\n  {}",
        violations.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// Gate 2 — resolved feature sets, under both resolutions
// ---------------------------------------------------------------------------

#[test]
fn reborn_linked_device_resolved_feature_sets_are_pinned() {
    let root = workspace_root();
    let mut violations = Vec::new();
    for all_features in [false, true] {
        let resolved = resolved_stack(&root, all_features);
        let lane = if all_features {
            "cargo metadata --all-features (the lane CI's clippy job runs)"
        } else {
            "cargo metadata (default resolution)"
        };

        let resolved_names: BTreeSet<&str> = resolved.keys().map(String::as_str).collect();
        let pinned_names: BTreeSet<&str> = PINNED_CRATES.iter().map(|pin| pin.name).collect();
        if resolved_names != pinned_names {
            violations.push(format!(
                "{lane}: resolved package set {resolved_names:?} does not match the pin \
                 {pinned_names:?}"
            ));
            continue;
        }

        for pin in PINNED_CRATES {
            let Some((version, features)) = resolved.get(pin.name) else {
                continue;
            };
            if version != pin.version {
                violations.push(format!(
                    "{lane}: {} resolves to {version} but is pinned at {}",
                    pin.name, pin.version
                ));
            }
            let expected: BTreeSet<String> = pin.features.iter().map(ToString::to_string).collect();
            if *features != expected {
                let added: Vec<&String> = features.difference(&expected).collect();
                let removed: Vec<&String> = expected.difference(features).collect();
                violations.push(format!(
                    "{lane}: {} resolves with features {features:?}, pinned {expected:?} \
                     (added {added:?}, removed {removed:?}). Feature sets are pinned because a \
                     feature is new code compiled into a process that already holds every user's \
                     session key — including features an edge you do not own turned on.",
                    pin.name
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "resolved feature sets drifted from the supply-chain pin:\n  {}",
        violations.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// Gate 3 — the socks5 `proxy` feature
// ---------------------------------------------------------------------------

#[test]
fn reborn_linked_device_socks5_proxy_feature_stays_off() {
    let root = workspace_root();
    let mut violations = Vec::new();

    // (a) Nothing declares it.
    let declared =
        parse_declared_dependencies(&read(&root, &resolved_manifest_path(&root)), STACK_PREFIX)
            .unwrap_or_else(|error| panic!("{error}"));
    for edge in &declared {
        if edge
            .features
            .iter()
            .any(|feature| feature == FORBIDDEN_FEATURE)
        {
            violations.push(format!(
                "[{}] {} declares the `{FORBIDDEN_FEATURE}` feature",
                edge.table, edge.name
            ));
        }
    }

    // (b) Nothing resolves with it, in either lane.
    for all_features in [false, true] {
        for (name, (_, features)) in resolved_stack(&root, all_features) {
            if features.contains(FORBIDDEN_FEATURE) {
                violations.push(format!(
                    "{name} resolves with `{FORBIDDEN_FEATURE}` under \
                     all_features={all_features}"
                ));
            }
        }
    }

    // (c) The lockfile agrees — an independent check that needs no cargo run.
    let packages =
        parse_lockfile(&read(&root, "Cargo.lock")).unwrap_or_else(|error| panic!("{error}"));
    let locked = stack_entries(&packages).unwrap_or_else(|error| panic!("{error}"));
    let owner = locked.get(PROXY_OWNER).unwrap_or_else(|| {
        panic!(
            "{PROXY_OWNER} is not in Cargo.lock, so the strongest half of this test would pass \
             without measuring anything"
        )
    });
    for dependency in PROXY_ENABLED_DEPENDENCIES {
        if owner.depends_on(dependency) {
            violations.push(format!(
                "{PROXY_OWNER}'s locked dependency list contains `{dependency}`, which only \
                 `{FORBIDDEN_FEATURE}` pulls in"
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "the socks5 `{FORBIDDEN_FEATURE}` feature is enabled somewhere in the in-process MTProto \
         stack:\n  {}\n\nThis is not a hygiene failure. A proxied dial is established through \
         `tokio-socks` and never consults `Session::dc_option`, which is the only seam the \
         package's datacenter-address validation owns — so with this feature on, the design's \
         \"100% of dials are validated\" claim is false, not merely weaker. Turn it off, or \
         redesign the validation seam first and amend the ADR.",
        violations.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// Gate 4 — the declared edges
// ---------------------------------------------------------------------------

#[test]
fn reborn_linked_device_package_declares_every_edge_exactly() {
    let root = workspace_root();
    let declared =
        parse_declared_dependencies(&read(&root, &resolved_manifest_path(&root)), STACK_PREFIX)
            .unwrap_or_else(|error| panic!("{error}"));
    assert!(
        !declared.is_empty(),
        "no {STACK_PREFIX}* dependency is declared in {PACKAGE_MANIFEST}, so every assertion \
         below would pass without measuring anything. If the direct edges moved to another \
         package, repoint PACKAGE_MANIFEST in the same change."
    );

    let expected: BTreeMap<&str, &DeclaredEdge> = DECLARED_EDGES
        .iter()
        .map(|edge| (edge.name, edge))
        .collect();
    let found: BTreeSet<&str> = declared.iter().map(|edge| edge.name.as_str()).collect();
    assert_eq!(
        found,
        expected.keys().copied().collect::<BTreeSet<&str>>(),
        "the declared {STACK_PREFIX}* edge set changed in {PACKAGE_MANIFEST}"
    );

    let mut violations = Vec::new();
    for edge in &declared {
        let Some(pin) = expected.get(edge.name.as_str()) else {
            continue;
        };
        if edge.shorthand {
            violations.push(format!(
                "{}: written as the `{} = \"…\"` shorthand, which cannot carry \
                 `default-features = false` — so upstream owns this crate's feature set",
                edge.name, edge.name
            ));
            continue;
        }
        match edge.requirement.as_deref() {
            Some(requirement) if requirement == pin.requirement => {}
            other => violations.push(format!(
                "{}: version requirement is {:?}, expected the exact pin {:?}. A caret or tilde \
                 range lets a republished patch release enter this process with no diff review.",
                edge.name, other, pin.requirement
            )),
        }
        if edge.default_features != Some(false) {
            violations.push(format!(
                "{}: `default-features` is {:?}, expected `false`. Leaving upstream's `default` \
                 in place means a future release can widen this workspace's feature set by \
                 widening its own.",
                edge.name, edge.default_features
            ));
        }
        let declared_features: BTreeSet<&str> = edge.features.iter().map(String::as_str).collect();
        let allowed: BTreeSet<&str> = pin.features.iter().copied().collect();
        if declared_features != allowed {
            violations.push(format!(
                "{}: declares features {declared_features:?}, allowlist is {allowed:?}",
                edge.name
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "the declared {STACK_PREFIX}* edges in {PACKAGE_MANIFEST} drifted from the allowlist:\n  \
         {}",
        violations.join("\n  ")
    );
}

fn resolved_manifest_path(root: &Path) -> String {
    let absolute = crate_path(root, PACKAGE_MANIFEST);
    assert!(
        absolute.is_file(),
        "{} does not exist, so every manifest assertion keyed on it would pass without \
         measuring anything — repoint PACKAGE_MANIFEST in the change that moved the package",
        absolute.display()
    );
    absolute
        .strip_prefix(root)
        .unwrap_or(&absolute)
        .to_string_lossy()
        .into_owned()
}

// ---------------------------------------------------------------------------
// Gate 5 — the human-review surface (and its stated residual)
// ---------------------------------------------------------------------------

/// The dependency-update surface must not propose this stack automatically.
///
/// Read the module docs' residual section before treating this as "human review
/// is in place": it removes the *bot* path, which is the only path this
/// repository actually has a lever on. There is no `CODEOWNERS` file, so no
/// named reviewer can be required — the second half of this test asserts that
/// absence rather than letting a later `CODEOWNERS` addition sit unnoticed while
/// the residual note keeps claiming it does not exist.
#[test]
fn reborn_linked_device_bumps_are_kept_off_the_automated_update_path() {
    let root = workspace_root();
    let config = read(&root, DEPENDABOT_CONFIG);
    let cargo_section = cargo_ecosystem_section(&config).unwrap_or_else(|| {
        panic!(
            "{DEPENDABOT_CONFIG} has no `package-ecosystem: cargo` entry. If the Rust update \
             surface was removed, this gate is measuring nothing — say so explicitly instead of \
             deleting the assertion."
        )
    });
    let ignored = ignored_dependency_globs(cargo_section);
    let unignored: Vec<&str> = PINNED_CRATES
        .iter()
        .map(|pin| pin.name)
        .filter(|name| !ignored.iter().any(|glob| glob_matches(glob, name)))
        .collect();
    assert!(
        unignored.is_empty(),
        "{DEPENDABOT_CONFIG}'s cargo entry does not ignore {unignored:?}, so a bot can open a \
         bump PR for a dependency that runs in-process with full process authority. The \
         `everything-else` group matches `*`, so an explicit `ignore` entry is what keeps this \
         stack off the automated path; a bump must arrive as a deliberate human edit that also \
         edits the pin table in this file. Ignore globs found: {ignored:?}"
    );

    // The residual, asserted so it cannot silently stop being true.
    let codeowners_locations = [
        "CODEOWNERS",
        ".github/CODEOWNERS",
        "docs/CODEOWNERS",
        ".gitlab/CODEOWNERS",
    ];
    let existing: Vec<&str> = codeowners_locations
        .into_iter()
        .filter(|relative| root.join(relative).is_file())
        .collect();
    assert!(
        existing.is_empty(),
        "a CODEOWNERS file now exists at {existing:?}. That is the surface PROPOSAL §11.1's \
         fourth control asked for and this gate recorded as absent — add the \
         `{STACK_PREFIX}`-owning paths to it and rewrite the residual paragraph in this file's \
         module docs, which currently tells the reader no named reviewer can be required."
    );
}

/// The text region of `config` belonging to the `cargo` ecosystem entry.
///
/// Deliberately a slice, not a YAML parse: this crate carries no YAML
/// dependency, and the question ("does the cargo entry ignore these names?") is
/// answerable from the region between one `- package-ecosystem:` line and the
/// next. The limitation is real and stated: an `ignore` written outside any
/// ecosystem entry would not be found, which fails *closed*.
fn cargo_ecosystem_section(config: &str) -> Option<&str> {
    let marker = "- package-ecosystem:";
    let mut start = None;
    for (offset, _) in config.match_indices(marker) {
        let line_end = config[offset..]
            .find('\n')
            .map(|at| offset + at)
            .unwrap_or(config.len());
        let value = config[offset + marker.len()..line_end]
            .trim()
            .trim_matches('"');
        match (start, value) {
            (None, "cargo") => start = Some(line_end),
            (Some(from), _) => return Some(&config[from..offset]),
            (None, _) => {}
        }
    }
    start.map(|from| &config[from..])
}

/// Every `dependency-name:` glob in `section`, unquoted.
fn ignored_dependency_globs(section: &str) -> Vec<String> {
    section
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim().trim_start_matches("- ").trim();
            let value = trimmed.strip_prefix("dependency-name:")?;
            Some(
                value
                    .trim()
                    .trim_matches(|character| character == '"' || character == '\'')
                    .to_string(),
            )
        })
        .filter(|value| !value.is_empty())
        .collect()
}

/// Dependabot's `dependency-name` matching, restricted to the two forms this
/// gate needs: an exact name, or a single trailing `*`.
fn glob_matches(glob: &str, name: &str) -> bool {
    match glob.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => glob == name,
    }
}

// ---------------------------------------------------------------------------
// Scanner self-tests — a guardrail that cannot fail is not a guardrail
// (crate guidance: sabotage-test every new gate).
// ---------------------------------------------------------------------------

const LOCK_FIXTURE: &str = r#"
[[package]]
name = "grammers-client"
version = "0.10.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "deadbeef"
dependencies = [
 "grammers-mtsender",
 "tokio",
]

[[package]]
name = "grammers-mtsender"
version = "0.10.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "cafebabe"
dependencies = [
 "tokio 1.2.3 (registry+https://github.com/rust-lang/crates.io-index)",
]

[[package]]
name = "serde"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "0badc0de"
"#;

#[test]
fn reborn_linked_device_lockfile_parser_self_test() {
    let packages = parse_lockfile(LOCK_FIXTURE).expect("fixture parses");
    let stack = stack_entries(&packages).expect("fixture has stack entries");
    assert_eq!(
        stack.keys().collect::<Vec<_>>(),
        vec!["grammers-client", "grammers-mtsender"],
        "only the prefixed packages are in scope"
    );
    let client = &stack["grammers-client"];
    assert_eq!(client.version, "0.10.0");
    assert_eq!(client.checksum.as_deref(), Some("deadbeef"));
    assert_eq!(client.source.as_deref(), Some(REGISTRY_SOURCE));
    assert!(
        client.depends_on("grammers-mtsender") && client.depends_on("tokio"),
        "bare dependency entries are matched"
    );
    assert!(
        stack["grammers-mtsender"].depends_on("tokio"),
        "the `name version (source)` dependency form must match on its leading token — a \
         substring or equality match would report the proxy crates as absent when they are there"
    );
    assert!(!client.depends_on("tokio-socks"));
}

#[test]
fn reborn_linked_device_lockfile_scan_fails_closed_on_an_empty_result() {
    let packages = parse_lockfile("[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n")
        .expect("fixture parses");
    let error = stack_entries(&packages).expect_err("an empty stack must refuse, not return {}");
    assert!(
        error.contains("delete this gate"),
        "the refusal must tell the reader what to do; got: {error}"
    );
}

#[test]
fn reborn_linked_device_lockfile_scan_refuses_two_versions_of_one_member() {
    let doubled = format!(
        "{LOCK_FIXTURE}\n[[package]]\nname = \"grammers-client\"\nversion = \"0.11.0\"\n\
         source = \"{REGISTRY_SOURCE}\"\nchecksum = \"f00d\"\n"
    );
    let packages = parse_lockfile(&doubled).expect("fixture parses");
    let error = stack_entries(&packages).expect_err("two versions must refuse");
    assert!(error.contains("locked at two versions"), "got: {error}");
}

#[test]
fn reborn_linked_device_lockfile_parser_refuses_an_unknown_shape() {
    let error = parse_lockfile("version = 4\n").expect_err("a lockfile with no packages refuses");
    assert!(error.contains("no [[package]] array"), "got: {error}");
}

#[test]
fn reborn_linked_device_manifest_parser_self_test() {
    let manifest = r#"
[dependencies]
grammers-client = { version = "=0.10.0", default-features = false, features = [] }
grammers-session = { version = "=0.10.0", default-features = false, features = ["serde"] }
grammers-tl-types = "0.10.0"
serde = "1"

[dev-dependencies]
grammers-mtsender = { version = "0.10", features = ["proxy"] }
"#;
    let declared = parse_declared_dependencies(manifest, STACK_PREFIX).expect("fixture parses");
    let names: Vec<&str> = declared.iter().map(|edge| edge.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "grammers-client",
            "grammers-session",
            "grammers-tl-types",
            "grammers-mtsender",
        ],
        "dev-dependencies are in scope — resolver-v2 unifies their features into the test build, \
         so a proxy-enabling dev edge turns the feature on for the lane CI runs"
    );

    let client = declared
        .iter()
        .find(|edge| edge.name == "grammers-client")
        .expect("client edge");
    assert_eq!(client.requirement.as_deref(), Some("=0.10.0"));
    assert_eq!(client.default_features, Some(false));
    assert!(client.features.is_empty() && !client.shorthand);

    let shorthand = declared
        .iter()
        .find(|edge| edge.name == "grammers-tl-types")
        .expect("shorthand edge");
    assert!(
        shorthand.shorthand && shorthand.default_features.is_none(),
        "the `name = \"req\"` form must be reported as shorthand, not as a table with defaults \
         left unset — the two need different messages"
    );

    let dev = declared
        .iter()
        .find(|edge| edge.name == "grammers-mtsender")
        .expect("dev edge");
    assert_eq!(dev.table, "dev-dependencies");
    assert_eq!(dev.requirement.as_deref(), Some("0.10"));
    assert_eq!(dev.features, vec![FORBIDDEN_FEATURE.to_string()]);
    assert_eq!(
        dev.default_features, None,
        "an omitted `default-features` must read as unset, which the gate rejects"
    );
}

#[test]
fn reborn_linked_device_manifest_parser_refuses_an_unreadable_manifest() {
    let error = parse_declared_dependencies("this is not toml = = =", STACK_PREFIX)
        .expect_err("invalid TOML refuses");
    assert!(error.contains("not valid TOML"), "got: {error}");
}

#[test]
fn reborn_linked_device_dependabot_section_and_glob_matching_self_test() {
    let config = "\
version: 2
updates:
  - package-ecosystem: cargo
    directory: \"/\"
    ignore:
      - dependency-name: \"grammers-*\"
  - package-ecosystem: github-actions
    directory: \"/\"
    ignore:
      - dependency-name: \"actions/checkout\"
";
    let section = cargo_ecosystem_section(config).expect("cargo section found");
    assert!(
        section.contains("grammers-*") && !section.contains("actions/checkout"),
        "the section must stop at the next ecosystem entry, so an ignore belonging to a \
         different ecosystem cannot satisfy this gate"
    );
    assert_eq!(ignored_dependency_globs(section), vec!["grammers-*"]);

    assert!(glob_matches("grammers-*", "grammers-tl-parser"));
    assert!(glob_matches("grammers-client", "grammers-client"));
    assert!(!glob_matches("grammers-client", "grammers-session"));
    assert!(
        !glob_matches("grammers-*", "notgrammers-client"),
        "a trailing-star glob anchors at the start"
    );

    assert!(
        cargo_ecosystem_section("version: 2\nupdates: []\n").is_none(),
        "a config with no cargo entry must report none rather than returning the whole file"
    );
    assert!(
        ignored_dependency_globs("    ignore:\n      - dependency-name:\n").is_empty(),
        "an empty value is not an ignore glob"
    );
}

/// The pin table itself has to be well-formed, or every assertion above
/// compares against nonsense.
/// The consumer half of the pin: exactly one workspace crate may name the
/// MTProto stack, and this table says which.
///
/// Every other assertion in this file bounds *what* the stack is — version,
/// checksum, features. None of them bounds *who links it*, so a second crate
/// taking a `grammers-*` edge would inherit an in-process dependency with full
/// process authority while every gate here still reported success. Written as
/// a table rather than a single literal so a second linked-device package adds
/// a reviewed row instead of loosening the rule.
#[test]
fn reborn_linked_device_stack_has_exactly_the_declared_consumers() {
    /// Manifest paths permitted to declare a `grammers-*` dependency.
    const PERMITTED_CONSUMERS: &[&str] = &[PACKAGE_MANIFEST];

    let root = workspace_root();
    let metadata = cargo_metadata(&root, false);
    let members: BTreeSet<String> = metadata["workspace_members"]
        .as_array()
        .expect("cargo metadata must include workspace_members")
        .iter()
        .map(|id| id.as_str().expect("member id is a string").to_string())
        .collect();

    let mut actual: Vec<String> = Vec::new();
    for package in metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages")
    {
        let id = package["id"].as_str().expect("package id is a string");
        if !members.contains(id) {
            continue;
        }
        let manifest = Path::new(
            package["manifest_path"]
                .as_str()
                .expect("manifest_path is a string"),
        );
        let text = std::fs::read_to_string(manifest)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", manifest.display()));
        let declares = text
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .any(|line| line.contains(STACK_PREFIX));
        if declares {
            actual.push(
                manifest
                    .strip_prefix(&root)
                    .unwrap_or(manifest)
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    actual.sort();
    assert!(
        !members.is_empty(),
        "no workspace members resolved; this gate would pass having scanned nothing"
    );

    let mut expected: Vec<String> = PERMITTED_CONSUMERS
        .iter()
        .map(|path| {
            let resolved = crate_path(&root, path);
            resolved
                .strip_prefix(&root)
                .unwrap_or(&resolved)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    expected.sort();

    assert_eq!(
        actual, expected,
        "the MTProto stack must be linked by exactly the declared consumers. A crate that \
         appears here has taken an in-process dependency with full process authority; a crate \
         that disappeared has dropped the edge and its row should go with it."
    );
}

#[test]
fn reborn_linked_device_pin_table_is_internally_consistent() {
    assert!(
        !PINNED_CRATES.is_empty() && !DECLARED_EDGES.is_empty(),
        "an empty pin table makes every gate in this file vacuous"
    );
    let pinned: BTreeSet<&str> = PINNED_CRATES.iter().map(|pin| pin.name).collect();
    assert_eq!(
        pinned.len(),
        PINNED_CRATES.len(),
        "PINNED_CRATES has a duplicate name"
    );
    for pin in PINNED_CRATES {
        assert!(
            pin.name.starts_with(STACK_PREFIX),
            "{} is pinned but does not match the scope prefix, so the derived-scope assertion \
             would report it missing forever",
            pin.name
        );
        assert_eq!(
            pin.checksum.len(),
            64,
            "{}: a Cargo.lock checksum is a 64-character sha256 hex digest",
            pin.name
        );
        assert!(
            pin.checksum.chars().all(|c| c.is_ascii_hexdigit()),
            "{}: checksum is not hex",
            pin.name
        );
        assert!(
            !pin.features.contains(&FORBIDDEN_FEATURE),
            "{}: the pin table itself lists `{FORBIDDEN_FEATURE}` — the gate would then enforce \
             the very state it exists to forbid",
            pin.name
        );
    }
    for edge in DECLARED_EDGES {
        assert!(
            pinned.contains(edge.name),
            "{} is a declared edge with no entry in PINNED_CRATES",
            edge.name
        );
        assert!(
            edge.requirement.starts_with('=') && edge.requirement.len() > 1,
            "{}: the allowlisted requirement {:?} is not an exact pin",
            edge.name,
            edge.requirement
        );
        assert!(
            !edge.features.contains(&FORBIDDEN_FEATURE),
            "{}: the allowlist itself permits `{FORBIDDEN_FEATURE}`",
            edge.name
        );
    }
}
