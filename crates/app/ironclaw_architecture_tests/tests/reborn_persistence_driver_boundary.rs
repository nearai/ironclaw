//! Where the PostgreSQL driver is allowed to be named.
//!
//! `deadpool_postgres` is a third-party driver type. PROPOSAL §11.2.6 charters a
//! specific, small set of crates to hold it — the persistence substrates that
//! run SQL, plus the composition root, which is "the one app-layer crate
//! permitted a database driver". §6.3.2 additionally requires
//! `ironclaw_event_store` to **stop leaking `deadpool_postgres::Pool` in
//! its public API**; that crate owns the TLS/driver cone but its callers should
//! not have to name the driver to use it.
//!
//! Two assertions, because they fail for different reasons: one pins *which
//! crates* may link the driver at all (a shrink-only ratchet), the other pins
//! that event_store's own public surface is driver-free.

#[allow(dead_code)]
mod ratchet_support;

use std::{collections::BTreeSet, process::Command};

use ratchet_support::{crate_path, workspace_root};
use serde_json::Value;

/// Crates permitted a *normal* (non-dev) `deadpool-postgres` dependency.
///
/// Shrink-only. Adding a crate here means a new part of the tree links a
/// database driver, which is the thing §11.2.6 constrains — that is a design
/// decision, not a build fix, and it should be argued in review rather than
/// discovered later. Removing one is always fine.
///
/// Dev-dependencies are deliberately unconstrained: a test that stands up a
/// real Postgres pool to exercise a contract is not the driver spreading into
/// production build graphs.
const DRIVER_LINKED_CRATES: &[&str] = &[
    // Substrates that execute SQL directly.
    //
    // Two of these are the §11.2.6 "ADR-or-converge" exceptions, decided
    // 2026-08-04 as KEEP with a written ADR each — `ironclaw_hooks` and
    // `ironclaw_triggers`, tagged below. The ADRs state why convergence onto
    // the `RootFilesystem` fabric is not available and what would reopen the
    // decision; read the one that argues for an entry before removing it.
    "ironclaw_auth",
    "ironclaw_filesystem",
    // ADR 0004 (`docs/internal/adr/0004-hooks-keeps-its-predicate-state-backends.md`).
    "ironclaw_hooks",
    "ironclaw_host_runtime",
    // ADR 0003 (`docs/internal/adr/0003-triggers-keeps-hand-written-sql.md`).
    "ironclaw_triggers",
    // Owns the TLS/driver cone for durable event/audit logs (§6.3.2).
    "ironclaw_event_store",
    // The assembly root: opens each database once and wires the shared runtime
    // (§11.2.6). The one app-layer crate permitted a driver.
    "ironclaw_composition",
    // Diagnostic binary, excluded from default work.
    "ironclaw_stress",
];

/// The other three drivers §11.2.6's clause (b) names, each seeded at its
/// **measured** set and ratcheting down only — same discipline as
/// `DRIVER_LINKED_CRATES` above, which had covered `deadpool-postgres` alone.
///
/// ✎ **Added 2026-08-05 (CHECKLIST WS10, §11.2.6 clause (b)).** The clause reads
/// *"no crate may hold a normal dep on `libsql`/`deadpool`/`deadpool-postgres`/
/// `tokio-postgres` outside …"*, and only the third of the four was gated — so
/// three of the four driver cones could spread with nothing to notice, which is
/// precisely the "spread invisibly" failure the clause exists to stop. The
/// `libsql` set is the interesting one: it is **nine** crates, and it is the
/// number §11.2.6's own target sentence is written against
/// (`{libsql_runtime, filesystem, event_store, composition}` plus shrink-only
/// `{triggers, hooks}` — so `auth`/`host_runtime`/`turn_runner`/`stress` are the
/// named residue, and `turn_runner` holding a database driver is the entry most
/// worth an owner's attention).
///
/// Seeded, not aspirational: every entry is measured on this tree through
/// `cargo metadata`, and the equality below fails in **both** directions, so a
/// crate that stops linking a driver must leave the list in the same PR
/// (#7147's lesson — a ceiling above the live set is an unclaimed budget).
const ADDITIONAL_DRIVER_ALLOWLISTS: &[(&str, &[&str])] = &[
    (
        "libsql",
        &[
            // §11.2.6's chartered four.
            "ironclaw_composition",
            "ironclaw_event_store",
            "ironclaw_filesystem",
            "ironclaw_libsql_runtime",
            // ADR 0004 / ADR 0003, as above — shrink-only by the clause's own
            // wording.
            "ironclaw_hooks",
            "ironclaw_triggers",
            // Measured residue the clause names explicitly as "today also
            // includes". Each is a standing §11.2.6 target, not a charter.
            "ironclaw_host_runtime",
            "ironclaw_turn_runner",
            "ironclaw_stress",
        ],
    ),
    (
        "deadpool",
        &[
            // The pool type itself — only the two crates that build pools.
            "ironclaw_filesystem",
            "ironclaw_libsql_runtime",
        ],
    ),
    (
        "tokio-postgres",
        &[
            "ironclaw_event_store",
            "ironclaw_filesystem",
            "ironclaw_hooks",
            "ironclaw_triggers",
            "ironclaw_stress",
        ],
    ),
];

#[test]
fn only_chartered_crates_link_the_postgres_driver() {
    assert_driver_allowlist("deadpool-postgres", DRIVER_LINKED_CRATES);
}

/// §11.2.6 clause (b) for the three drivers the postgres gate above does not
/// cover. One test rather than three so a reader cannot fix one driver and
/// leave the others reported nowhere.
#[test]
fn only_chartered_crates_link_the_other_persistence_drivers() {
    for (driver, allowed) in ADDITIONAL_DRIVER_ALLOWLISTS {
        assert_driver_allowlist(driver, allowed);
    }
}

/// Exact-match, both directions. Growth is new driver spread; shrinkage left
/// unrecorded is untracked slack that silently permits a re-add.
fn assert_driver_allowlist(driver: &str, allowed: &[&str]) {
    let actual = normal_dependents_of(driver);
    assert!(
        !actual.is_empty(),
        "no workspace crate holds a normal `{driver}` dependency — either the driver left the \
         tree (delete its allowlist in this PR) or the metadata query broke, in which case this \
         gate would pass having measured nothing"
    );
    let allowed: BTreeSet<String> = allowed.iter().map(|name| (*name).to_string()).collect();

    let added: Vec<_> = actual.difference(&allowed).cloned().collect();
    assert!(
        added.is_empty(),
        "these crates gained a normal `{driver}` dependency without being chartered for a \
         database driver (PROPOSAL §11.2.6): {added:?}. If that is intended, add them to the \
         allowlist with the reason — these lists are shrink-only on purpose."
    );

    let removed: Vec<_> = allowed.difference(&actual).cloned().collect();
    assert!(
        removed.is_empty(),
        "these crates no longer link `{driver}`: {removed:?}. That is good news — drop them \
         from the allowlist so the list keeps ratcheting down instead of quietly permitting a \
         re-add."
    );
}

/// §6.3.2: "stop leaking `deadpool_postgres::Pool` in the public API (wrap)".
///
/// The driver may still be *used* — this crate builds the pool and owns the TLS
/// policy — but only inside the private `postgres_backed` module. Anywhere else
/// in the crate is reachable from its public surface, and naming the driver
/// there puts a third-party type into callers' signatures.
///
/// Scans **every** `.rs` file in the crate minus that one module *body*. An
/// earlier version scanned only `lib.rs` up to the module header, which left
/// two blind spots that made the gate weaker than its own docs claimed: items
/// *after* the module body, and every sibling file (`coalescing_sink.rs`,
/// `durable_log.rs`).
///
/// The walk is **recursive and symlink-rejecting**. A third revision of "weaker
/// than its docs claimed": the scan used a flat `read_dir`, so the word "every"
/// held only while `src/` stayed flat. A later `src/postgres/pool.rs` — exactly
/// where a driver mention would go — would have been skipped, and a skipped file
/// is indistinguishable from a clean one. `crates/events/ironclaw_event_store/src`
/// is flat today, so this was latent rather than live; it is fixed here so the
/// doc sentence is true by construction rather than by luck.
#[test]
fn event_store_names_the_driver_only_inside_its_private_backend_module() {
    let src = crate_path(&workspace_root(), "crates/ironclaw_event_store/src");
    let mut files = rust_sources_recursive(&src);
    files.sort();
    assert!(
        files.len() >= 2,
        "expected several source files in {src:?}, saw {} — this scan cannot be trusted",
        files.len()
    );

    let mut leaks = Vec::new();
    for path in &files {
        let source = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("readable source {path:?}: {error}"));
        let exempt = private_backend_module_body(&source, path);
        for (index, line) in source.lines().enumerate() {
            if line.trim_start().starts_with("//") || !line.contains("deadpool_postgres") {
                continue;
            }
            if exempt.as_ref().is_some_and(|range| range.contains(&index)) {
                continue;
            }
            leaks.push(format!(
                "  {}:{}: {}",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                index + 1,
                line.trim()
            ));
        }
    }

    assert!(
        leaks.is_empty(),
        "`ironclaw_event_store` names `deadpool_postgres` outside its private \
         `postgres_backed` module body. Callers should receive \
         `ironclaw_filesystem::PostgresConnectionPool` instead — the driver cone belongs \
         to this crate and the filesystem substrate, not to their signatures (PROPOSAL \
         §6.3.2). Offending lines:\n{}",
        leaks.join("\n")
    );
}

/// The token that declares the module the scan exempts.
const MODULE_HEADER: &str = "mod postgres_backed {";

/// The visibility written in front of [`MODULE_HEADER`] on `line`, if this line
/// *declares* the module: `Some("")` for a private `mod`, `Some("pub")` or
/// `Some("pub(crate)")` for a visible one.
///
/// `None` when the line merely mentions the name — a comment, a string — so
/// widening the match from "the line starts with `mod`" to "the line contains
/// the token" cannot be fooled by prose into picking a header that is not one.
fn module_visibility(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let prefix = trimmed[..trimmed.find(MODULE_HEADER)?].trim_end();
    (prefix.is_empty() || prefix.starts_with("pub")).then_some(prefix)
}

/// Offset just past the `"##…` that closes a raw string opened with `hashes`
/// hashes. Searched over bytes rather than `&str`, so a multi-byte character
/// inside the literal cannot leave the index off a char boundary — `&str`
/// slicing panics there.
fn raw_string_end(bytes: &[u8], hashes: usize) -> Option<usize> {
    let terminator = 1 + hashes;
    bytes
        .windows(terminator)
        .position(|window| window[0] == b'"' && window[1..].iter().all(|byte| *byte == b'#'))
        .map(|start| start + terminator)
}

/// Line range of the private `postgres_backed` module *body*, if `source`
/// declares one. `None` when the file has no such module.
///
/// Brace-matched rather than delimited by the next `}` at some indentation, and
/// trivia-aware (line comments, nestable block comments, strings, raw strings,
/// char literals) so a brace inside a literal cannot move the body boundary —
/// which would silently re-expose part of the module to the scan, or hide part
/// of the file from it.
///
/// String state is carried **across** lines, like the block-comment state. An
/// earlier revision reset it at every newline, so the continuation lines of a
/// multi-line literal were scanned as code: a `}` there truncated the body, and
/// a `{` there stretched it past the real closing brace and swallowed the
/// driver mentions that followed. That second direction is fail-open, and it is
/// pinned by a fixture in both directions.
/// Every `.rs` file under `dir`, recursively, failing loud on anything it cannot
/// read.
///
/// Fail-closed on purpose, matching `reborn_composition_boundaries.rs`: for a
/// containment gate, "the walk could not look" and "there is nothing there" must
/// not produce the same verdict. Symlinks are rejected rather than followed —
/// `DirEntry::file_type()` reports the *link*, so a symlinked subtree is neither
/// `is_dir()` nor an `.rs` file and the walk would step over it silently.
fn rust_sources_recursive(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let metadata = std::fs::symlink_metadata(dir)
        .unwrap_or_else(|error| panic!("readable walk root {dir:?}: {error}"));
    assert!(
        !metadata.file_type().is_symlink(),
        "{dir:?} is a symlink; the driver-boundary walk does not follow symlinks"
    );
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
            assert!(
                !file_type.is_symlink(),
                "{path:?} is a symlink; the driver-boundary walk does not follow \
                 symlinks, and stepping over one would let the gate report clean \
                 on a subtree it never read"
            );
            if file_type.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    out
}

fn private_backend_module_body(
    source: &str,
    path: &std::path::Path,
) -> Option<std::ops::Range<usize>> {
    let lines: Vec<&str> = source.lines().collect();
    // Keyed on the token, not the start of the line, so a *visible* module is
    // found rather than missed. Missing it would leave `exempt` empty and blame
    // whichever driver mention happened to be reported first, instead of naming
    // the visibility change that actually broke the containment.
    let (header, visibility) = lines
        .iter()
        .enumerate()
        .find_map(|(index, line)| module_visibility(line).map(|vis| (index, vis)))?;
    assert!(
        visibility.is_empty(),
        "{path:?}: `postgres_backed` must stay private; `{visibility} mod postgres_backed` \
         re-exports the driver cone it exists to contain"
    );

    let mut depth = 0usize;
    let mut in_block_comment = 0usize;
    let mut in_string = false;
    let mut in_raw_string: Option<usize> = None;
    for (offset, line) in lines.iter().enumerate().skip(header) {
        let bytes = line.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            let rest = &bytes[i..];
            if let Some(hashes) = in_raw_string {
                match raw_string_end(rest, hashes) {
                    Some(end) => {
                        i += end;
                        in_raw_string = None;
                    }
                    // The literal runs past this line; stay in it.
                    None => break,
                }
                continue;
            }
            if in_string {
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                if i < bytes.len() {
                    in_string = false;
                    i += 1;
                }
                continue;
            }
            if in_block_comment > 0 {
                if rest.starts_with(b"/*") {
                    in_block_comment += 1;
                    i += 2;
                } else if rest.starts_with(b"*/") {
                    in_block_comment -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if rest.starts_with(b"//") {
                break;
            }
            if rest.starts_with(b"/*") {
                in_block_comment = 1;
                i += 2;
                continue;
            }
            if bytes[i] == b'r' {
                let mut hashes = 0usize;
                while bytes.get(i + 1 + hashes) == Some(&b'#') {
                    hashes += 1;
                }
                if bytes.get(i + 1 + hashes) == Some(&b'"') {
                    // Opened only; the loop above closes it, this line or a later one.
                    in_raw_string = Some(hashes);
                    i += 2 + hashes;
                    continue;
                }
            }
            if bytes[i] == b'"' {
                in_string = true;
                i += 1;
                continue;
            }
            if bytes[i] == b'\'' {
                let escaped = bytes.get(i + 1) == Some(&b'\\');
                let close = if escaped { i + 3 } else { i + 2 };
                if bytes.get(close) == Some(&b'\'') {
                    i = close + 1;
                    continue;
                }
            }
            match bytes[i] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(header..offset + 1);
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    panic!("{path:?}: unterminated `mod postgres_backed` body");
}

/// Every crate with a normal (non-dev) dependency on `name`.
fn normal_dependents_of(name: &str) -> BTreeSet<String> {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(workspace_root())
        .output()
        .expect("cargo metadata runs");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata emits valid JSON");
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata has a packages array");
    assert!(
        !packages.is_empty(),
        "cargo metadata returned no packages; this scan would vacuously pass"
    );

    packages
        .iter()
        .filter(|package| {
            package["dependencies"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|dependency| {
                    dependency["name"].as_str() == Some(name)
                        // `kind` is null for a normal dependency, "dev"/"build" otherwise.
                        && dependency.get("kind").and_then(Value::as_str).is_none()
                })
        })
        .filter_map(|package| package["name"].as_str().map(ToString::to_string))
        .collect()
}

#[cfg(test)]
mod backend_module_body_tests {
    use super::{private_backend_module_body, rust_sources_recursive};
    use std::path::Path;

    fn body(source: &str) -> Option<std::ops::Range<usize>> {
        private_backend_module_body(source, Path::new("fixture.rs"))
    }

    /// The walk must reach a nested module. `src/` is flat today, so a flat
    /// `read_dir` passed — but `src/postgres/pool.rs` is exactly where a driver
    /// mention would live, and a skipped file reads as a clean one.
    #[test]
    fn the_walk_reaches_a_nested_module() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::write(root.path().join("lib.rs"), "// root\n").expect("write lib");
        std::fs::create_dir(root.path().join("postgres")).expect("create nested dir");
        std::fs::write(
            root.path().join("postgres/pool.rs"),
            "pub fn leak(p: deadpool_postgres::Pool) {}\n",
        )
        .expect("write nested source");

        let found = rust_sources_recursive(root.path());
        assert_eq!(
            found.len(),
            2,
            "both the root and the nested file: {found:?}"
        );
        assert!(
            found.iter().any(|p| p.ends_with("postgres/pool.rs")),
            "the nested module must be scanned, else a driver mention there is invisible: {found:?}"
        );
    }

    /// A symlinked subtree must fail the walk rather than be stepped over.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_subtree_fails_the_walk() {
        let root = tempfile::tempdir().expect("tempdir");
        let real = root.path().join("real");
        std::fs::create_dir(&real).expect("create real dir");
        std::fs::write(real.join("a.rs"), "// nothing\n").expect("write source");
        std::os::unix::fs::symlink(&real, root.path().join("linked")).expect("symlink");

        let panicked = std::panic::catch_unwind(|| rust_sources_recursive(root.path()));
        assert!(
            panicked.is_err(),
            "a symlinked subtree must fail the walk, not be skipped"
        );
    }

    /// A `{` inside a line comment must not open the body. Without this the
    /// exempt range stretches past the real closing brace and swallows every
    /// driver mention after it — the fail-open direction.
    #[test]
    fn a_brace_in_a_line_comment_does_not_move_the_body_boundary() {
        let source = "mod postgres_backed {\n    // opens nothing {\n}\npub fn leak(p: deadpool_postgres::Pool) {}\n";
        let range = body(source).expect("module found");
        assert!(
            !range.contains(&3),
            "line 4 (index 3) is after the body and must still be scanned"
        );
    }

    /// Block comments nest in Rust, so a single `*/` must not re-enter code.
    #[test]
    fn a_brace_in_a_nested_block_comment_does_not_move_the_body_boundary() {
        let source = "mod postgres_backed {\n    /* outer /* inner { */ still comment { */\n}\npub fn leak(p: deadpool_postgres::Pool) {}\n";
        let range = body(source).expect("module found");
        assert!(
            !range.contains(&3),
            "line 4 (index 3) is after the body and must still be scanned"
        );
    }

    /// A driver mention *inside* the body is permitted — that is the whole
    /// point of keeping the cone somewhere.
    #[test]
    fn a_mention_inside_the_body_is_within_the_exempt_range() {
        let source = "pub fn a() {}\nmod postgres_backed {\n    use deadpool_postgres::Pool;\n}\n";
        let range = body(source).expect("module found");
        assert!(range.contains(&2), "line 3 (index 2) is inside the body");
    }

    /// The regression: an item *after* the body used to be invisible, because
    /// the old scan stopped at the module header.
    #[test]
    fn a_mention_after_the_body_is_outside_the_exempt_range() {
        let source = "mod postgres_backed {\n    fn inner() {}\n}\npub fn leak(p: deadpool_postgres::Pool) {}\n";
        let range = body(source).expect("module found");
        assert!(
            !range.contains(&3),
            "line 4 (index 3) is after the body and must be scanned"
        );
    }

    /// A brace inside a string literal must not close the body early — that
    /// would drag the rest of the file into the exempt range.
    #[test]
    fn a_brace_in_a_literal_does_not_end_the_body() {
        let source = "mod postgres_backed {\n    const A: &str = \"}\";\n    fn inner() {}\n}\npub fn leak(p: deadpool_postgres::Pool) {}\n";
        let range = body(source).expect("module found");
        assert_eq!(range, 0..4, "body must end at the real closing brace");
    }

    #[test]
    fn a_file_without_the_module_has_no_exempt_range() {
        assert!(body("pub fn a() {}\n").is_none());
    }

    /// A brace inside a *multi-line* literal must not move the boundary either.
    /// The scan used to drop string state at every newline, so the continuation
    /// lines of a literal were read as code: a `}` there truncated the body
    /// (noisy), and a `{` there extended it past the real closing brace and
    /// **swallowed a driver mention that should have failed the gate** —
    /// fail-open, which is why this is pinned in both directions.
    #[test]
    fn a_brace_in_a_multi_line_literal_does_not_move_the_body_boundary() {
        // `}` on a continuation line used to close the body two lines early.
        let early = "mod postgres_backed {\n    const SQL: &str = \"first\n} second\";\n    fn inner() {}\n}\npub fn leak(p: deadpool_postgres::Pool) {}\n";
        assert_eq!(body(early).expect("module found"), 0..5);

        // `{` on a continuation line used to extend the body past the closing
        // brace, exempting the driver mention on the line after it.
        let late = "mod postgres_backed {\n    const SQL: &str = \"first\n{ second\";\n    fn inner() {}\n}\npub fn leak(p: deadpool_postgres::Pool) {}\n}\n";
        let range = body(late).expect("module found");
        assert_eq!(range, 0..5);
        assert!(
            !range.contains(&5),
            "the driver mention after the body must stay outside the exempt range"
        );

        // Raw strings span lines far more often than regular ones (SQL).
        let raw = "mod postgres_backed {\n    const SQL: &str = r#\"first\n} second\"#;\n    fn inner() {}\n}\npub fn leak(p: deadpool_postgres::Pool) {}\n";
        assert_eq!(body(raw).expect("module found"), 0..5);
    }

    /// `pub mod postgres_backed` re-exports the very driver cone the module
    /// exists to contain. The header match used to require the line to *start*
    /// with `mod`, so a visible module was simply never found and the
    /// visibility assertion below it could not run.
    #[test]
    #[should_panic(expected = "must stay private")]
    fn a_public_module_header_is_rejected() {
        let _ = body("pub mod postgres_backed {\n    use deadpool_postgres::Pool;\n}\n");
    }

    #[test]
    #[should_panic(expected = "must stay private")]
    fn a_crate_visible_module_header_is_rejected() {
        let _ = body("pub(crate) mod postgres_backed {\n    use deadpool_postgres::Pool;\n}\n");
    }

    /// The header match keys on the `mod postgres_backed {` token rather than
    /// the start of the line, so it must not mistake a comment or a string that
    /// names the module for the declaration itself.
    #[test]
    fn a_mention_of_the_module_name_is_not_mistaken_for_the_declaration() {
        let source = "// mod postgres_backed {\nlet s = \"mod postgres_backed {\";\nmod postgres_backed {\n    use deadpool_postgres::Pool;\n}\n";
        assert_eq!(body(source).expect("module found"), 2..5);
    }

    /// An unterminated body must not silently exempt the rest of the file.
    #[test]
    #[should_panic(expected = "unterminated")]
    fn an_unterminated_body_panics_instead_of_exempting_the_rest_of_the_file() {
        let _ = body("mod postgres_backed {\n    fn inner() {}\n");
    }
}
