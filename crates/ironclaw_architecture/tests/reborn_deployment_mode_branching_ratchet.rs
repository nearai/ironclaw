//! Anti-slippage ratchet for the deployment-mode *branching* axis (§4.4 / §10
//! of `docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`).
//!
//! Its two siblings own deployment mode as a **type name**
//! (`reborn_localdev_typename_ratchet`, `reborn_deployment_mode_typename_ratchet`).
//! This one owns the behaviour those names were a symptom of: **code that
//! reads a deployment mode to decide what to do.**
//!
//! §4.4 is explicit that mode and lane get opposite treatment, and why:
//!
//! > A deployment mode must be branched on in exactly **zero** places past the
//! > composition edge — that is the whole §2.1 thesis — so giving it an enum
//! > would hand every crate an invitation to `match` on it (which is precisely
//! > how the 66-identifier `LocalDev*` family grew).
//!
//! `RebornCompositionProfile` *is* such an enum. It survives as the CLI/env
//! parse artifact and as a display label; what must not survive is consumers
//! reading its variants to select behaviour. `DeploymentConfig` (§5.6) is where
//! a profile becomes data — substrate, traffic policy, readiness contract,
//! storage shape — and everything downstream reads those fields.
//!
//! ## What this freezes
//!
//! The **set** of production files under `crates/ironclaw_reborn_composition/src`
//! that name a `RebornCompositionProfile` variant. Set membership, not a count,
//! per §10: a count lets a new violation silently replace a retired one; only
//! set membership catches a *swap*. A file entering the set fails; a file
//! leaving it must be removed from the allowlist in the same PR, so the debt
//! can only shrink.
//!
//! This is deliberately coarser than "detect a `match`": variant paths are what
//! branching needs, and a line-based scan cannot reliably tell
//! `match p { Profile::X => .. }` from `if p == Profile::X` from
//! `matches!(p, Profile::X | ..)` — all three are the same debt.
//!
//! ## Definition of done
//!
//! The allowlist reaches `{deployment.rs}` — `DeploymentConfig::for_profile`,
//! the one place a profile name becomes deployment data. The remaining entries
//! and why each is still here are documented on the allowlist itself.
//!
//! **Owner:** the #6274 driver (Illia Polosukhin) — the person driving this
//! allowlist to `{deployment.rs}`. §10 requires every ratchet to name one; an
//! unowned ratchet is telemetry, not a gate.
//!
//! Scanner semantics: comments and string literals are stripped before
//! matching, so this file's own doc comment and fixtures do not self-trip.
//! Skips `tests/`, `examples/`, `benches/` trees and `*tests.rs` files —
//! test fixtures naming a profile are not production branching.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Production files under composition `src/` allowed to name a
/// `RebornCompositionProfile` variant, each with the reason it is still here.
///
/// Sorted; entries are `src/`-relative with `/` separators.
const ALLOWLIST: &[(&str, &str)] = &[
    (
        "deployment.rs",
        "TARGET STATE — `DeploymentConfig::for_profile` is the one place a \
         profile name becomes deployment data (§4.4). This entry stays.",
    ),
    (
        "memory_binding.rs",
        "Maps the composition profile to a typed `MemoryDeploymentProfile` for \
         the fail-closed memory profile-binding certification policy (#3537): \
         production rejects unverified third-party bindings absent an admin \
         override; local-dev permits them. The branch produces a typed \
         memory-deployment axis, not a raw label. Retires into `DeploymentConfig` \
         when it grows a memory-binding axis (#5264).",
    ),
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("architecture crate must live under crates/ironclaw_architecture")
        .to_path_buf()
}

/// Remove line comments, block comments, and string literals so that prose and
/// fixtures inside them cannot trip the scan.
///
/// Char literals are consumed too: a `"` (or `/`) inside a char literal such as
/// `'"'` must not open a string/comment and swallow the rest of the file, which
/// would silently hide a `DeploymentMode` branch from the scan. A char literal
/// (`'x'` / `'\n'` / `'"'`) is dropped; a lifetime (`'a`) is emitted as-is.
/// Uses char-indexed lookahead (a char literal needs to see two chars ahead),
/// mirroring the shared `ratchet_support` stripper the other §10 ratchets use.
fn strip_comments_and_strings(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // Line comment — drop to (not including) the newline, which the next
        // iteration preserves.
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            i += 2;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        // Block comment — drop through the closing `*/`.
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
            continue;
        }
        // String literal — drop through the closing `"`, honoring escapes.
        if c == '"' {
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i += 2;
                    continue;
                }
                if chars[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        // Char literal vs lifetime — only consume when it closes as a literal.
        if c == '\'' {
            // Escaped char literal `'\...'`: drop through the closing quote.
            if chars.get(i + 1) == Some(&'\\') {
                let mut k = i + 2;
                while k < chars.len() && chars[k] != '\'' {
                    k += 1;
                }
                i = k + 1;
                continue;
            }
            // Single-char literal `'x'` (incl. `'"'`): the quote two chars ahead
            // proves it is a literal, not a lifetime — drop all three.
            if chars.get(i + 2) == Some(&'\'') {
                i += 3;
                continue;
            }
            // A lifetime (`'a`) — emit and move on.
            out.push(c);
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn is_scanned_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.ends_with(".rs") && !name.ends_with("tests.rs")
}

fn collect(dir: &Path, root: &Path, found: &mut BTreeSet<String>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("read dir entry: {err}"));
        let path = entry.path();
        if path.is_dir() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            // `*_tests` covers the inline test trees composition keeps beside
            // production modules (e.g. `factory/local_dev_host_tests/`).
            if matches!(name, "tests" | "examples" | "benches" | "target")
                || name.ends_with("_tests")
            {
                continue;
            }
            collect(&path, root, found);
            continue;
        }
        if !is_scanned_file(&path) {
            continue;
        }
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        if !strip_comments_and_strings(&source).contains("RebornCompositionProfile::") {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        found.insert(relative);
    }
}

#[test]
fn deployment_mode_branching_allowlist_is_frozen_and_only_shrinks() {
    let root = workspace_root().join("crates/ironclaw_reborn_composition/src");
    let mut found = BTreeSet::new();
    collect(&root, &root, &mut found);

    let allowed: BTreeSet<String> = ALLOWLIST
        .iter()
        .map(|(path, _)| (*path).to_string())
        .collect();

    let new_debt: Vec<&String> = found.difference(&allowed).collect();
    assert!(
        new_debt.is_empty(),
        "new deployment-mode branching in composition: {new_debt:?}\n\
         A `RebornCompositionProfile` variant in a production file means code is \
         reading a deployment mode to decide what to do (§4.4). Add the axis to \
         `DeploymentConfig` and read that field instead. If the reference is a \
         display label with no behaviour attached, add the file to ALLOWLIST \
         with that justification."
    );

    let retired: Vec<&String> = allowed.difference(&found).collect();
    assert!(
        retired.is_empty(),
        "ALLOWLIST names files that no longer reference a composition profile: \
         {retired:?}\n\
         The ratchet may only shrink: delete these entries in the same PR that \
         retired them, so the allowlist keeps meaning what it says."
    );
}

#[test]
fn deployment_rs_is_the_target_state_entry() {
    // The definition of done is `{deployment.rs}`. Pin that the target entry is
    // present and documented as terminal, so a future cleanup does not
    // accidentally drive the allowlist to empty and delete the one place a
    // profile is *supposed* to become data.
    let target = ALLOWLIST
        .iter()
        .find(|(path, _)| *path == "deployment.rs")
        .expect("deployment.rs must stay on the allowlist as the target state");
    assert!(
        target.1.contains("TARGET STATE"),
        "deployment.rs's allowlist reason must mark it terminal, got: {}",
        target.1
    );
}

#[test]
fn allowlist_is_sorted_and_unique() {
    let paths: Vec<&str> = ALLOWLIST.iter().map(|(path, _)| *path).collect();
    let mut sorted = paths.clone();
    sorted.sort_unstable();
    assert_eq!(
        paths, sorted,
        "ALLOWLIST must stay sorted for reviewability"
    );
    let unique: BTreeSet<&str> = paths.iter().copied().collect();
    assert_eq!(unique.len(), paths.len(), "ALLOWLIST has duplicate entries");
}

#[test]
fn scanner_strips_comments_and_strings() {
    // Self-test (§10: every check ships with its own self-test). Without
    // stripping, this ratchet's own doc comment would put it on the list.
    let source = r#"
        // RebornCompositionProfile::LocalDev in a line comment
        /* RebornCompositionProfile::Production in a block comment */
        let label = "RebornCompositionProfile::Disabled";
    "#;
    let stripped = strip_comments_and_strings(source);
    assert!(
        !stripped.contains("RebornCompositionProfile::"),
        "stripped source still contains a variant path: {stripped}"
    );

    let real = "match profile { RebornCompositionProfile::LocalDev => 1, _ => 0 }";
    assert!(
        strip_comments_and_strings(real).contains("RebornCompositionProfile::"),
        "real branching must survive stripping"
    );

    // Regression (2026-07-19 gemini review): a char literal containing `"` must
    // not open a string and swallow a following branch. Before the char-literal
    // handling, `'"'` flipped in_string and the DeploymentMode match after it
    // was hidden from the scan — a silent ratchet false negative.
    let with_char_literal = r#"
        let quote = '"';
        match profile { RebornCompositionProfile::LocalDev => 1, _ => 0 }
    "#;
    assert!(
        strip_comments_and_strings(with_char_literal).contains("RebornCompositionProfile::"),
        "a `'\"'` char literal must not hide the branch after it"
    );
    // The char literal itself is dropped (it is not a lifetime), and a real
    // lifetime is preserved.
    assert!(!strip_comments_and_strings("let c = '\"';").contains('"'));
    assert!(strip_comments_and_strings("fn f<'a>(x: &'a str) {}").contains("'a"));
}
