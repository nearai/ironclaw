//! Zero-occurrence gate for TLS-verification escape hatches in the sandbox
//! egress proxy's TLS-interception seam (W6 phase 1, design doc
//! `docs/plans/2026-07-26-sandbox-credential-firewall-design.md` §4).
//!
//! `sandbox_process::tls_intercept::TlsInterceptConfig::origin_connector` is
//! what the proxy uses to verify the origin it re-originates TLS to, on
//! behalf of a sandboxed container that is deliberately never given the
//! real credential. If a production caller ever builds that connector with
//! `rustls::ClientConfig::dangerous()`, a custom `ServerCertVerifier` that
//! skips verification, or an empty `RootCertStore`, the interception seam
//! stops being a credential firewall and becomes a working, silent MITM
//! against our own users' egress traffic to every bound host.
//!
//! `VerifiedOriginConnector` (`sandbox_process::tls_intercept`) makes this
//! type-enforced: its only production constructor,
//! `VerifiedOriginConnector::from_system_roots`, builds from the platform's
//! real trust anchors and fails closed on an empty store; the escape hatch
//! (`VerifiedOriginConnector::for_test`) is `#[cfg(test)]` only. This test
//! is the second half of that guarantee — it pins the escape-hatch spellings
//! at **zero occurrences** in non-test code under `sandbox_process/`, so a
//! future caller cannot route *around* `VerifiedOriginConnector` and
//! hand-roll a permissive connector directly against `rustls` instead.
//!
//! **Test code is legitimately exempt.** `tls_intercept`'s own tests build
//! deliberately-empty and deliberately-single-root connectors
//! (`connector_trusting_nothing`, `connector_trusting_only`) to force the
//! fail-closed path deterministically — that is correct test behavior, not
//! the bug this gate exists to catch. The scan below excludes standalone
//! `tests.rs` files and truncates any file at its own `#[cfg(test)] mod
//! tests` marker, scanning only what precedes it.
//!
//! **The standalone-`tests.rs` exemption verifies its wiring, not just the
//! filename.** A file merely being named `.../tests.rs` proves nothing on
//! its own — nothing stops that file's content from compiling into every
//! build if the parent module's `mod tests;` declaration ever loses its
//! `#[cfg(test)]`. [`parent_gates_tests_module_behind_cfg_test`] reads the
//! parent file and confirms `#[cfg(test)]` (or `#[cfg(all(test, ...))]`)
//! genuinely precedes `mod tests;` before exempting the file; if that
//! wiring is missing or unparseable, the file is scanned like production
//! code instead of silently exempted. See that function's doc for exactly
//! what it does and does not handle.
//!
//! **Comments (and string literals) are exempt too**, same rule and same
//! rationale as `reborn_retired_failure_vocabulary.rs`: this module's own
//! doc comments (including this one) explain the ban by naming the exact
//! escape-hatch spellings, and prose explaining what is banned is worth
//! keeping — only live code is policed. Stripping uses the crate's shared
//! `ratchet_support::strip_comments_and_strings`, the same lexer two
//! sibling ratchets already use, rather than a hand-rolled comment-only
//! stripper: a comment-only stripper would treat `//` inside a string
//! literal (e.g. a `"http://..."` URL, which already exists in this very
//! directory) as a real line-comment start and blank everything after it on
//! that line — hiding a banned spelling that happened to share a line with
//! such a string.
//!
//! **One sanctioned call site.** `RootCertStore::empty()` is also the
//! correct, ordinary way to *start* building any root store — including the
//! real one `VerifiedOriginConnector::from_system_roots` populates from the
//! platform's native certs before ever handing it to a `ClientConfig`. That
//! single, already-reviewed call site is scoped out by function body, not by
//! file — every other line in `sandbox_process/`, including everywhere else
//! in this same file, is still policed at zero, and `dangerous(`/
//! `with_custom_certificate_verifier` are never sanctioned anywhere, not
//! even inside `from_system_roots`.

// Each ratchet binary gets its own copy of this shared module; this
// binary uses only the comment/string stripper and workspace_root, so the
// other shared helpers are dead code HERE (and live in the sibling ratchet
// binaries) — same convention as `reborn_authorized_seal_ratchet.rs`.
#[allow(dead_code)]
mod ratchet_support;

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use ratchet_support::{strip_comments_and_strings, workspace_root};

/// Escape-hatch spellings that would turn `origin_connector` permissive.
/// Any hit in non-test `sandbox_process/` code is a regression against the
/// invariant `VerifiedOriginConnector` exists to make unrepresentable.
const BANNED_PATTERNS: &[&str] = &[
    "dangerous(",
    "with_custom_certificate_verifier",
    "RootCertStore::empty()",
];

fn sandbox_process_dir(root: &Path) -> PathBuf {
    root.join("crates/ironclaw_host_runtime/src/sandbox_process")
}

/// A whitespace-free copy of an entire (already comment/string-stripped)
/// file, paired with a per-character index back to the 1-based source line
/// each retained character came from. Lets [`scan_dir`] match a banned
/// pattern across the **whole file** in one search instead of one line at a
/// time, so semantically identical Rust written with incidental whitespace
/// or split across a line break — `.dangerous ()`, `.dangerous\n()`,
/// `RootCertStore :: empty ( )`, `RootCertStore::\nempty()` — cannot slip
/// past the ban by varying how (or whether) it is spread across lines. None
/// of `BANNED_PATTERNS` contains whitespace itself, so stripping whitespace
/// here only widens matching — it never turns a real hit into a miss.
struct Haystack {
    normalized: String,
    line_of_char: Vec<usize>,
}

impl Haystack {
    fn build(code_only: &str) -> Self {
        let mut normalized = String::with_capacity(code_only.len());
        let mut line_of_char = Vec::with_capacity(code_only.len());
        let mut line_number = 1usize;
        for character in code_only.chars() {
            if character == '\n' {
                line_number += 1;
                continue;
            }
            if character.is_whitespace() {
                continue;
            }
            normalized.push(character);
            line_of_char.push(line_number);
        }
        Self {
            normalized,
            line_of_char,
        }
    }

    /// Every occurrence of `pattern`, as (1-based source line the match
    /// starts on, byte offset into `normalized`) — the offset lets a caller
    /// (the sanctioned-call carve-out) test whether a specific match sits
    /// inside a specific line span without this type needing to know
    /// anything about that carve-out itself.
    fn find_all(&self, pattern: &str) -> Vec<(usize, usize)> {
        let mut hits = Vec::new();
        let mut search_from = 0usize;
        while let Some(found) = self.normalized[search_from..].find(pattern) {
            let start = search_from + found;
            let line = self.line_of_char.get(start).copied().unwrap_or(1);
            hits.push((line, start));
            search_from = start + pattern.len().max(1);
        }
        hits
    }
}

/// A standalone test file (`ca/tests.rs`, `credential_firewall/tests.rs`) is
/// pure test code end to end — excluded wholesale rather than line-scanned,
/// PROVIDED its parent module actually wires it in behind `#[cfg(test)]`
/// (see [`parent_gates_tests_module_behind_cfg_test`]). This predicate alone
/// is filename-only and must never be used, on its own, to decide
/// exemption — see that function's doc for why.
fn is_standalone_test_file(relative: &str) -> bool {
    relative.ends_with("/tests.rs") || relative.ends_with("\\tests.rs")
}

/// Whether `line` (already trimmed) is a `#[cfg(...)]` attribute that
/// *unconditionally requires* `test` — either the bare `#[cfg(test)]` or an
/// `#[cfg(all(test, ...))]` combination. An `any(test, ...)` is deliberately
/// rejected: it does not guarantee the module is test-only, since the `any`
/// branch could be satisfied without `test` being set, so a module gated
/// that way could still compile into a non-test build.
fn is_cfg_test_attribute(line: &str) -> bool {
    let Some(inner) = line
        .strip_prefix("#[cfg(")
        .and_then(|rest| rest.strip_suffix(")]"))
    else {
        return false;
    };
    if inner == "test" {
        return true;
    }
    match inner
        .strip_prefix("all(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        Some(args) => args.split(',').any(|part| part.trim() == "test"),
        None => false,
    }
}

/// The real fix this gate was missing: verifies a standalone `.../tests.rs`
/// file's *parent* module actually declares it behind `#[cfg(test)]` —
/// `#[cfg(test)]` (or `#[cfg(all(test, ...))]`), then `mod tests;`, allowing
/// blank lines, doc comments, and other (non-`cfg`) attributes in between —
/// rather than trusting the `/tests.rs` filename alone.
///
/// **Why this exists.** Without it, stripping `#[cfg(test)]` from
/// `tls_intercept.rs`'s `mod tests;` — so `tls_intercept/tests.rs` compiles
/// into *every* build, including release — combined with a `.dangerous()`
/// call planted inside that file, passed this gate silently: the file was
/// exempted purely because its name ended in `/tests.rs`, never checking
/// whether it was actually reachable only under `#[cfg(test)]`. This gate
/// has now silently failed to bind four times (see the module's history);
/// this function exists so a `/tests.rs` file with no verified `#[cfg(test)]`
/// wiring is scanned like production code instead of silently exempted.
///
/// **Fails toward scanning, not toward exemption.** Every unhandled shape —
/// a missing parent file, an `#[cfg(any(test, ...))]` gate, a multi-line
/// `#[cfg(...)]` attribute, `#[path = "..."]` redirection to a
/// differently-named module, or an inline `mod tests { ... }` body instead
/// of an external-file `mod tests;` declaration — returns `Ok(false)`
/// ("not verified"), which the caller treats as "scan it like production
/// code." A false negative here (a legitimately test-only file this check
/// can't verify) only costs an extra, harmless scan; a false positive would
/// recreate the exact hole this function exists to close. A genuine I/O
/// error reading the parent file is propagated, not swallowed — same
/// fail-loud policy as [`scan_dir`]'s own I/O handling.
fn parent_gates_tests_module_behind_cfg_test(root: &Path, relative: &str) -> io::Result<bool> {
    let Some(parent_stem) = relative
        .strip_suffix("/tests.rs")
        .or_else(|| relative.strip_suffix("\\tests.rs"))
    else {
        return Ok(false);
    };
    let parent_path = root.join(format!("{parent_stem}.rs"));
    let contents = match std::fs::read_to_string(&parent_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };

    let mut pending_cfg_test = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("///") || trimmed.starts_with("//!") {
            // Blank lines and doc comments never break a pending #[cfg(test)].
            continue;
        }
        if pending_cfg_test && trimmed == "mod tests;" {
            return Ok(true);
        }
        if is_cfg_test_attribute(trimmed) {
            pending_cfg_test = true;
            continue;
        }
        if trimmed.starts_with('#') && trimmed.ends_with(']') {
            // A different single-line attribute (e.g. `#[allow(dead_code)]`)
            // between `#[cfg(test)]` and `mod tests;` — tolerated, does not
            // reset the pending state.
            continue;
        }
        pending_cfg_test = false;
    }
    Ok(false)
}

/// Strips only the `#[cfg(test)] mod tests { ... }` (or `#[cfg(test)] mod
/// tests;`) span itself out of `contents`, leaving everything before *and
/// after* it intact.
///
/// This crate's convention is to keep that marker at the end of the file,
/// but that is a convention, not something the scanner may assume: earlier
/// versions of this function truncated everything from the marker line
/// onward, so a production item placed (accidentally or otherwise) after an
/// inline `mod tests { ... }` block's own closing brace — still legal Rust,
/// still compiled into every build — was silently dropped from the scan
/// along with the real test module. Tracking the inline body's brace depth
/// to find its true end, and continuing to scan whatever follows, closes
/// that hole without having to assume anything about file layout.
///
/// Brace depth is counted from a comment/string-stripped copy of each line
/// (via `ratchet_support::strip_comments_and_strings`, called once up front
/// on the whole file so line numbers stay aligned), not from raw bytes: a
/// string literal inside the inline module's own body containing a literal
/// `{` with no matching `}` on the same line (e.g. `let s = "{";`) would
/// otherwise unbalance the raw count so `depth` never returns to zero,
/// making the "read until the module's own closing brace" loop consume
/// every remaining line in the file — silently dropping whatever production
/// code follows, including a banned escape-hatch call. The *output* still
/// uses the original, unstripped lines — this only changes what decides
/// where the module ends.
fn truncate_at_inline_test_module(contents: &str) -> String {
    let stripped = strip_comments_and_strings(contents);
    let mut previous_was_cfg_test = false;
    let mut result = String::with_capacity(contents.len());
    let mut lines = contents.lines().zip(stripped.lines()).peekable();
    while let Some((line, stripped_line)) = lines.next() {
        let trimmed = stripped_line.trim();
        // Exact match only: `mod tests;` (external-file form) or
        // `mod tests {` (inline-body form). A prefix check
        // (`starts_with("mod tests")`) would also match an unrelated
        // `mod tests_helper;` sharing the line right after a `#[cfg(test)]`
        // marker, hiding any real production code that follows — the same
        // look-alike-name hole `declares_from_system_roots` was already
        // hardened against.
        if previous_was_cfg_test && trimmed == "mod tests;" {
            previous_was_cfg_test = false;
            continue;
        }
        if previous_was_cfg_test && trimmed.starts_with("mod tests {") {
            // Track brace depth from this line's *stripped* form to find
            // where the inline module body actually ends, then drop every
            // (original) line in between — but keep scanning whatever
            // follows.
            let mut depth: i64 = 0;
            for byte in stripped_line.bytes() {
                match byte {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
            }
            while depth > 0 {
                let Some((_, inner_stripped)) = lines.next() else {
                    break;
                };
                for byte in inner_stripped.bytes() {
                    match byte {
                        b'{' => depth += 1,
                        b'}' => depth -= 1,
                        _ => {}
                    }
                }
            }
            previous_was_cfg_test = false;
            continue;
        }
        previous_was_cfg_test = trimmed == "#[cfg(test)]";
        result.push_str(line);
        result.push('\n');
    }
    result
}

/// The line numbers (1-indexed, into `code_only`) covered by
/// `VerifiedOriginConnector::from_system_roots`'s body — the one place
/// `RootCertStore::empty()` is sanctioned (see the module doc's "one
/// sanctioned call site"). Only meaningful for `tls_intercept.rs`; callers
/// pass an empty set for every other file.
///
/// Uses simple brace-depth tracking from the `fn from_system_roots` line to
/// wherever that depth returns to zero. `code_only` has already had comments
/// *and string literal contents* stripped (`strip_comments_and_strings`), so
/// a format string like `"...: {error}"` contributes no stray braces to the
/// count — only real code braces are left to track.
///
/// The name match is on the **exact** identifier, not a prefix. A substring
/// test would also match a hostile look-alike such as
/// `fn from_system_roots_untrusted`, handing its whole body the carve-out and
/// letting a `dangerous()` call inside it pass the gate unnoticed. That would
/// make this test a guard with a hole in exactly the shape it exists to
/// close, so `declares_from_system_roots` requires the next character after
/// the identifier to be a non-identifier character.
///
/// "Non-identifier character" means Rust's real identifier-continuation
/// rule (`XID_Continue`, via the `unicode-ident` crate — the same crate
/// `proc-macro2`/`syn` already use in this workspace to tokenize
/// identifiers), not ASCII alphanumeric/underscore alone. A combining mark
/// (e.g. U+0301) is `XID_Continue` to rustc — `fn
/// from_system_roots\u{0301}(...)` is a genuinely different, visually
/// near-identical identifier — but is not `char::is_alphanumeric()`, so an
/// ASCII-only check would wrongly treat that look-alike as the real function
/// and hand its whole body the same carve-out this exact-identifier fix
/// exists to deny ASCII look-alikes.
fn declares_from_system_roots(line: &str) -> bool {
    const NEEDLE: &str = "fn from_system_roots";
    let mut search_from = 0usize;
    while let Some(found) = line[search_from..].find(NEEDLE) {
        let start = search_from + found;
        let after = start + NEEDLE.len();
        // Exact identifier: whatever follows must not extend the name.
        // `_` is itself `XID_Continue` (and `XID_Start`), so
        // `is_xid_continue` alone already covers it.
        let extends_identifier = line[after..]
            .chars()
            .next()
            .is_some_and(unicode_ident::is_xid_continue);
        if !extends_identifier {
            return true;
        }
        search_from = after;
    }
    false
}

fn sanctioned_from_system_roots_lines(relative: &str, code_only: &str) -> HashSet<usize> {
    let mut sanctioned = HashSet::new();
    if !relative.ends_with("sandbox_process/tls_intercept.rs") {
        return sanctioned;
    }
    let mut depth: i64 = 0;
    let mut inside = false;
    for (number, line) in code_only.lines().enumerate() {
        if !inside {
            if declares_from_system_roots(line) {
                inside = true;
            } else {
                continue;
            }
        }
        sanctioned.insert(number + 1);
        for byte in line.bytes() {
            match byte {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
        }
        if inside && depth <= 0 && line.contains('}') {
            break;
        }
    }
    sanctioned
}

/// Scans `dir` for the banned patterns, recursing into subdirectories.
///
/// Every I/O error (`read_dir`, a directory-entry read, or `read_to_string`)
/// is propagated rather than silently skipped: a zero-occurrence security
/// gate that can pass because it failed to read some of what it claims to
/// scan is worse than no gate at all, since it reports "clean" without
/// having looked. Fail the gate instead.
fn scan_dir(root: &Path, dir: &Path, hits: &mut Vec<String>) -> io::Result<()> {
    let entries = std::fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            scan_dir(root, &path, hits)?;
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.ends_with(".rs") {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if is_standalone_test_file(&relative)
            && parent_gates_tests_module_behind_cfg_test(root, &relative)?
        {
            continue;
        }
        let contents = std::fs::read_to_string(&path)?;
        let production_only = truncate_at_inline_test_module(&contents);
        let code_only = strip_comments_and_strings(&production_only);
        let sanctioned_lines = sanctioned_from_system_roots_lines(&relative, &code_only);
        let haystack = Haystack::build(&code_only);
        for pattern in BANNED_PATTERNS {
            for (line, _offset) in haystack.find_all(pattern) {
                if *pattern == "RootCertStore::empty()" && sanctioned_lines.contains(&line) {
                    continue;
                }
                hits.push(format!("{relative}:{line}: `{pattern}`"));
            }
        }
    }
    Ok(())
}

#[test]
fn sandbox_process_never_hand_rolls_a_permissive_origin_connector() {
    let root = workspace_root();
    let mut hits = Vec::new();
    scan_dir(&root, &sandbox_process_dir(&root), &mut hits)
        .unwrap_or_else(|error| panic!("scanning sandbox_process/ for escape hatches: {error}"));
    hits.sort();
    hits.dedup();
    assert!(
        hits.is_empty(),
        "a TLS-verification escape hatch appeared in production sandbox_process/ \
         code. `origin_connector` re-originates TLS to the real upstream on \
         behalf of a sandboxed container that is deliberately never given the \
         real credential — a permissive connector here turns the interception \
         seam into a working, silent MITM against our own users. Build the \
         connector through `tls_intercept::VerifiedOriginConnector::from_system_roots` \
         instead (test code may still use `VerifiedOriginConnector::for_test` \
         and deliberately-empty/single-root connectors under `#[cfg(test)]`):\n{}",
        hits.join("\n")
    );
}

/// Proves the test-code exclusion is real, not just claimed, for the
/// `truncate_at_inline_test_module` path — the mechanism this test pins is
/// still exercised by other `sandbox_process/` files that keep their tests
/// inline (`scope_key.rs`, `container_identity.rs`, `mounts.rs`); this test
/// uses `scope_key.rs`'s `mod tests`-only `fn scope(` fixture, pure test
/// code unique to its `mod tests` block, which must disappear once the scan
/// truncates at the inline test module marker.
///
/// `tls_intercept.rs` itself no longer has an inline test module — its
/// tests were extracted to the standalone `tls_intercept/tests.rs`, matching
/// `ca.rs`/`credential_firewall.rs`'s convention, and `is_standalone_test_file`
/// excludes `/tests.rs` files wholesale rather than by truncation (see
/// `the_scan_still_exempts_tls_intercepts_extracted_test_file` below) — so
/// this test no longer targets `tls_intercept.rs`.
///
/// (`RootCertStore::empty()` itself is *not* a safe marker for this: real
/// production code — `VerifiedOriginConnector::from_system_roots` —
/// legitimately calls it too, see the module doc's "one sanctioned call
/// site" — so this test pins truncation against a marker that only ever
/// appears in test code.) If this assertion ever fails, the gate above is
/// either scanning test code (false positives waiting to force someone to
/// weaken it) or not truncating correctly.
#[test]
fn the_scan_exempts_an_inline_test_module() {
    let root = workspace_root();
    let path = sandbox_process_dir(&root).join("scope_key.rs");
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    assert!(
        contents.contains("fn scope("),
        "expected scope_key.rs's own tests to still define the `fn scope(` \
         test fixture — if this changed, this test needs a different \
         test-only fixture, not deletion"
    );
    let production_only = truncate_at_inline_test_module(&contents);
    assert!(
        !production_only.contains("fn scope("),
        "truncate_at_inline_test_module let test-only content leak into the \
         scanned production prefix"
    );
}

/// Proves `tls_intercept.rs`'s own extracted test file
/// (`tls_intercept/tests.rs`) is excluded from the scan via
/// `is_standalone_test_file`, not via truncation — `tls_intercept.rs`
/// itself has no inline `mod tests` left to truncate at, so if this
/// exclusion regressed, the scan would either miss the file's test-only
/// `connector_trusting_nothing` fixture (a false negative for the gate,
/// meaning it isn't actually looking) or, worse, wrongly flag test-only
/// fixtures like `connector_trusting_nothing` as production code.
#[test]
fn the_scan_still_exempts_tls_intercepts_extracted_test_file() {
    let root = workspace_root();
    let tests_path = sandbox_process_dir(&root).join("tls_intercept/tests.rs");
    let contents = std::fs::read_to_string(&tests_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", tests_path.display()));
    assert!(
        contents.contains("fn connector_trusting_nothing"),
        "expected tls_intercept/tests.rs to still define \
         `connector_trusting_nothing` — if this changed, this test needs a \
         different test-only fixture, not deletion"
    );
    let relative = tests_path
        .strip_prefix(&root)
        .unwrap_or(&tests_path)
        .to_string_lossy()
        .replace('\\', "/");
    assert!(
        is_standalone_test_file(&relative),
        "tls_intercept/tests.rs must be recognized as a standalone test file \
         so the scan excludes it wholesale, not by truncation"
    );

    let main_path = sandbox_process_dir(&root).join("tls_intercept.rs");
    let main_contents = std::fs::read_to_string(&main_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", main_path.display()));
    assert!(
        !main_contents.contains("mod tests {"),
        "tls_intercept.rs should declare its extracted tests via `mod tests;`, \
         not keep an inline `mod tests {{ ... }}` block alongside the \
         extracted file"
    );
}

/// Proves the sanctioned-call-site carve-out is scoped to
/// `from_system_roots`'s own body, not the whole file: `tls_intercept.rs`
/// production code has exactly one `RootCertStore::empty()` call (inside
/// `from_system_roots`) and the scan above must report zero hits for it —
/// but the carve-out must not swallow a *different* line elsewhere in
/// production code. Regression-tests the exact bug the ratchet had before
/// this scoping existed: a file-wide exemption would have let a second,
/// unrelated `RootCertStore::empty()` call through unnoticed anywhere else
/// in this file.
#[test]
fn sanctioned_call_site_is_scoped_to_the_one_function_not_the_whole_file() {
    let root = workspace_root();
    let relative = "crates/ironclaw_host_runtime/src/sandbox_process/tls_intercept.rs";
    let path = root.join(relative);
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let production_only = truncate_at_inline_test_module(&contents);
    let code_only = strip_comments_and_strings(&production_only);
    let sanctioned_lines = sanctioned_from_system_roots_lines(relative, &code_only);
    let real_occurrences = code_only
        .lines()
        .filter(|line| line.contains("RootCertStore::empty()"))
        .count();
    assert_eq!(
        real_occurrences, 1,
        "expected exactly one production `RootCertStore::empty()` call (inside \
         `from_system_roots`); if a second one was added elsewhere in this file, \
         the sanctioned-lines carve-out must not silently cover it too"
    );
    assert_eq!(
        sanctioned_lines
            .iter()
            .filter(|line_number| {
                code_only
                    .lines()
                    .nth(**line_number - 1)
                    .is_some_and(|line| line.contains("RootCertStore::empty()"))
            })
            .count(),
        1,
        "the sanctioned line span should cover exactly the one \
         `RootCertStore::empty()` call inside `from_system_roots`"
    );
}

/// The carve-out must key off the **exact** function identifier. Before this,
/// the match was `line.contains("fn from_system_roots")`, so a look-alike such
/// as `from_system_roots_untrusted` inherited the exemption for its whole body
/// — anyone could have parked a `dangerous()` call inside a plausibly-named
/// helper and this gate would have reported clean. Since the gate exists
/// precisely to stop a permissive verifier reaching production, a hole shaped
/// like the thing it guards is the worst kind, so it is pinned here.
#[test]
fn the_carve_out_matches_the_exact_function_name_not_a_prefix() {
    assert!(
        declares_from_system_roots("    pub(crate) fn from_system_roots() -> Result<Self> {"),
        "the real declaration must be recognised"
    );
    assert!(
        declares_from_system_roots("fn from_system_roots(){"),
        "no space before the parameter list is still the same function"
    );

    for impostor in [
        "    fn from_system_roots_untrusted() -> Self {",
        "    fn from_system_roots_no_verify() -> Self {",
        "    fn from_system_roots2() -> Self {",
    ] {
        assert!(
            !declares_from_system_roots(impostor),
            "a look-alike must NOT inherit the carve-out: {impostor}"
        );
    }

    // End to end: a file containing only the impostor gets no sanctioned
    // lines, so a banned call inside it stays visible to the scan.
    let impostor_body = "fn from_system_roots_untrusted() -> Self {\n    \
         RootCertStore::empty()\n}\n";
    let sanctioned = sanctioned_from_system_roots_lines(
        "crates/ironclaw_host_runtime/src/sandbox_process/tls_intercept.rs",
        impostor_body,
    );
    assert!(
        sanctioned.is_empty(),
        "an impostor function must receive no sanctioned lines, so its \
         `RootCertStore::empty()` is still reported; got {sanctioned:?}"
    );
}

/// `declares_from_system_roots`'s identifier-boundary check must follow
/// Rust's real identifier-continuation rule (`XID_Continue`), not just ASCII
/// alphanumeric/underscore. A combining mark (e.g. U+0301 COMBINING ACUTE
/// ACCENT) is a valid `XID_Continue` character to rustc — `fn
/// from_system_roots\u{0301}(...)` compiles as a distinct identifier from
/// `from_system_roots`, visually near-identical — but `char::is_alphanumeric()`
/// does not classify combining marks as alphanumeric, so the old
/// alphanumeric-only check would treat this look-alike as an exact match and
/// hand its whole body the `RootCertStore::empty()` carve-out, exactly the
/// hole the exact-identifier fix (the prefix-match fix above) exists to
/// close for ASCII look-alikes.
#[test]
fn the_carve_out_rejects_a_unicode_combining_mark_look_alike() {
    let impostor = "    fn from_system_roots\u{0301}() -> Self {";
    assert!(
        !declares_from_system_roots(impostor),
        "a combining-mark look-alike must NOT inherit the carve-out \
         (XID_Continue-aware boundary check required): {impostor}"
    );

    // End to end: a file containing only the combining-mark impostor gets
    // no sanctioned lines, so a banned call inside it stays visible.
    let impostor_body = "fn from_system_roots\u{0301}() -> Self {\n    RootCertStore::empty()\n}\n";
    let sanctioned = sanctioned_from_system_roots_lines(
        "crates/ironclaw_host_runtime/src/sandbox_process/tls_intercept.rs",
        impostor_body,
    );
    assert!(
        sanctioned.is_empty(),
        "a combining-mark impostor must receive no sanctioned lines, so its \
         `RootCertStore::empty()` is still reported; got {sanctioned:?}"
    );
}

/// Regression for the fifth near-miss on this exact class of gate (see the
/// module's commit history — this file's boundary checks have silently
/// failed to bind four times already): `mod tests_helper;` sharing a line
/// right after a `#[cfg(test)]` marker must NOT be mistaken for the real
/// `mod tests;`/`mod tests {` inline-test-module marker. A prefix check
/// (`starts_with("mod tests")`) would truncate the scan there, hiding any
/// real production code — including a banned escape-hatch spelling — that
/// follows `mod tests_helper;` in the same file. Reproduced directly against
/// `truncate_at_inline_test_module` (confirmed to fail before the exact-match
/// fix landed) and end to end through `scan_dir` against a fabricated fixture
/// tree, mirroring `gate_fails_when_a_tests_rs_files_cfg_test_wiring_is_missing`'s
/// tempdir approach.
#[test]
fn truncate_does_not_mistake_a_look_alike_module_name_for_the_real_test_module() {
    let contents = "\
#[cfg(test)]
mod tests_helper;

pub(crate) fn build_dangerous() -> rustls::ClientConfig {
    rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(()))
}
";
    let truncated = truncate_at_inline_test_module(contents);
    assert!(
        truncated.contains("dangerous()"),
        "a look-alike `mod tests_helper;` must not truncate the scan and hide \
         the real production code (including a banned `dangerous()` call) \
         that follows it; truncated={truncated:?}"
    );
}

/// End-to-end proof of the same fix through the real `scan_dir` gate: a
/// `mod tests_helper;` module declared right after `#[cfg(test)]`, followed
/// by a banned escape-hatch spelling in genuine production code, must be
/// caught — not silently exempted because the scan truncated early.
#[test]
fn gate_catches_a_banned_pattern_hidden_behind_a_look_alike_tests_helper_module() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let sandbox_dir = sandbox_process_dir(root);
    std::fs::create_dir_all(&sandbox_dir).expect("create sandbox_process dir");

    std::fs::write(
        sandbox_dir.join("tls_intercept.rs"),
        "pub(crate) fn noop() {}\n\n\
         #[cfg(test)]\n\
         mod tests_helper;\n\n\
         pub(crate) fn build() -> u8 {\n    \
             let _ = rustls::ClientConfig::builder().dangerous();\n    \
             0\n\
         }\n",
    )
    .expect("write tls_intercept.rs");

    let mut hits = Vec::new();
    scan_dir(root, &sandbox_dir, &mut hits).expect("scan must succeed");
    assert!(
        !hits.is_empty(),
        "a `mod tests_helper;` look-alike must not truncate the scan before \
         the banned `.dangerous()` call that follows it in production code"
    );
}

/// Proves the standalone-test-file exclusion (`is_standalone_test_file`) is
/// real, the same way the two tests above prove it for the inline-test-module
/// and sanctioned-call-site exclusions: `scan_dir`'s real run against
/// `ca/tests.rs` and `credential_firewall/tests.rs` currently reports zero
/// hits regardless of whether this exclusion fires, because neither file
/// happens to contain a banned pattern today — so without a direct check,
/// this predicate could be inverted or deleted and the main gate test would
/// still pass. Exercised directly against both its true positive and true
/// negative shapes, plus both path separators.
#[test]
fn is_standalone_test_file_recognizes_both_path_separators_and_only_tests_rs() {
    assert!(is_standalone_test_file(
        "crates/ironclaw_host_runtime/src/sandbox_process/ca/tests.rs"
    ));
    assert!(is_standalone_test_file(
        "crates\\ironclaw_host_runtime\\src\\sandbox_process\\ca\\tests.rs"
    ));
    assert!(!is_standalone_test_file(
        "crates/ironclaw_host_runtime/src/sandbox_process/ca.rs"
    ));
    assert!(!is_standalone_test_file(
        "crates/ironclaw_host_runtime/src/sandbox_process/tls_intercept.rs"
    ));
}

/// Unit coverage for [`is_cfg_test_attribute`]'s recognizer, independent of
/// the file-scanning end-to-end tests below: the exact spellings the module
/// doc calls out as things a correct check must handle (plain `#[cfg(test)]`,
/// `#[cfg(all(test, ...))]`) and the ones it must NOT treat as an
/// unconditional test gate (`any(...)`, `not(test)`, an unrelated cfg, or
/// plain non-attribute code).
#[test]
fn is_cfg_test_attribute_recognizes_the_documented_shapes() {
    assert!(is_cfg_test_attribute("#[cfg(test)]"));
    assert!(is_cfg_test_attribute("#[cfg(all(test, unix))]"));
    assert!(is_cfg_test_attribute("#[cfg(all(unix, test))]"));
    assert!(is_cfg_test_attribute("#[cfg(all(test,windows))]"));

    // `any(...)` does not guarantee test-only: the module could still
    // compile in a non-test build if the other branch is satisfied.
    assert!(!is_cfg_test_attribute("#[cfg(any(test, feature = \"x\"))]"));
    // A negation must not be confused with a positive test gate.
    assert!(!is_cfg_test_attribute("#[cfg(not(test))]"));
    assert!(!is_cfg_test_attribute("#[cfg(unix)]"));
    assert!(!is_cfg_test_attribute("mod tests;"));
    assert!(!is_cfg_test_attribute("#[allow(dead_code)]"));
}

/// End-to-end proof that the exemption is now conditioned on real wiring,
/// not the filename alone — the exact planted-violation scenario from the
/// module doc: a parent module declares `mod tests;` with NO preceding
/// `#[cfg(test)]` (so that file's content, including its "test" module,
/// compiles into every build, including release), and the sibling
/// `tests.rs` contains a banned escape-hatch spelling. Before this fix,
/// `is_standalone_test_file` alone decided exemption and this reported
/// clean; the gate must now report the hit.
///
/// Uses a fabricated fixture tree under a tempdir (not the real repo files)
/// so this test can plant an unwired `mod tests;` without ever touching
/// real source — the same property `sanctioned_call_site_is_scoped_to_the_
/// one_function_not_the_whole_file` above already relies on by constructing
/// its own `impostor_body` string rather than mutating `tls_intercept.rs`.
#[test]
fn gate_fails_when_a_tests_rs_files_cfg_test_wiring_is_missing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let sandbox_dir = sandbox_process_dir(root);
    let tls_dir = sandbox_dir.join("tls_intercept");
    std::fs::create_dir_all(&tls_dir).expect("create tls_intercept dir");

    // The planted violation: `mod tests;` with no preceding `#[cfg(test)]`.
    std::fs::write(
        sandbox_dir.join("tls_intercept.rs"),
        "pub(crate) fn noop() {}\n\nmod tests;\n",
    )
    .expect("write tls_intercept.rs");
    // The escape-hatch spelling planted inside the file the old, filename-only
    // check would have wrongly exempted.
    std::fs::write(
        tls_dir.join("tests.rs"),
        "fn build() { let _ = rustls::ClientConfig::builder().dangerous(); }\n",
    )
    .expect("write tls_intercept/tests.rs");

    let mut hits = Vec::new();
    scan_dir(root, &sandbox_dir, &mut hits).expect("scan must succeed");
    assert!(
        !hits.is_empty(),
        "a tests.rs file whose parent does not gate it behind #[cfg(test)] \
         must be scanned like production code, not silently exempted — the \
         gate must report the planted `.dangerous()` call"
    );
}

/// The other half of the same proof: the identical banned spelling in the
/// identical `tests.rs` location must still be exempt once the parent's
/// wiring is genuinely correct (`#[cfg(test)]` immediately before
/// `mod tests;`, matching `ca.rs`/`credential_firewall.rs`/`tls_intercept.rs`'s
/// real convention) — the fix must not turn into a blanket false-positive
/// generator against every legitimately test-only `tests.rs` file.
#[test]
fn gate_still_exempts_a_correctly_cfg_test_wired_tests_rs_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let sandbox_dir = sandbox_process_dir(root);
    let tls_dir = sandbox_dir.join("tls_intercept");
    std::fs::create_dir_all(&tls_dir).expect("create tls_intercept dir");

    std::fs::write(
        sandbox_dir.join("tls_intercept.rs"),
        "pub(crate) fn noop() {}\n\n#[cfg(test)]\nmod tests;\n",
    )
    .expect("write tls_intercept.rs");
    std::fs::write(
        tls_dir.join("tests.rs"),
        "fn build() { let _ = rustls::ClientConfig::builder().dangerous(); }\n",
    )
    .expect("write tls_intercept/tests.rs");

    let mut hits = Vec::new();
    scan_dir(root, &sandbox_dir, &mut hits).expect("scan must succeed");
    assert!(
        hits.is_empty(),
        "a correctly #[cfg(test)]-gated tests.rs file must still be exempt, \
         got: {hits:?}"
    );
}

/// Proves the parent-wiring check tolerates the specific shapes the module
/// doc calls out as things that must not defeat it — blank lines, a doc
/// comment, and another (non-`cfg`) attribute between `#[cfg(test)]` and
/// `mod tests;` — while still rejecting the shapes that must NOT count as
/// verified wiring: a missing parent file, and an inline `mod tests { ... }`
/// body instead of an external-file `mod tests;` declaration.
#[test]
fn parent_gates_tests_module_behind_cfg_test_handles_documented_edge_cases() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let sandbox_dir = sandbox_process_dir(root);
    std::fs::create_dir_all(&sandbox_dir).expect("create sandbox_process dir");
    let relative = "crates/ironclaw_host_runtime/src/sandbox_process/widget/tests.rs";

    // Whitespace, a doc comment, and another attribute in between: still
    // verified.
    std::fs::write(
        sandbox_dir.join("widget.rs"),
        "#[cfg(test)]\n\n/// explains the test module\n#[allow(dead_code)]\nmod tests;\n",
    )
    .expect("write widget.rs");
    assert!(
        parent_gates_tests_module_behind_cfg_test(root, relative).expect("read must succeed"),
        "blank lines, a doc comment, and another attribute between \
         #[cfg(test)] and `mod tests;` must not defeat the check"
    );

    // No parent file at all: not verified, but not an error either — the
    // caller treats `Ok(false)` as "scan it."
    std::fs::remove_file(sandbox_dir.join("widget.rs")).expect("remove widget.rs");
    assert!(
        !parent_gates_tests_module_behind_cfg_test(root, relative).expect("must not error"),
        "a missing parent file must not be silently treated as verified"
    );

    // An inline `mod tests { ... }` body is a different construct entirely
    // from an external-file `mod tests;` declaration — must not verify.
    std::fs::write(
        sandbox_dir.join("widget.rs"),
        "#[cfg(test)]\nmod tests {\n    // inline, not the external tests.rs file\n}\n",
    )
    .expect("write widget.rs");
    assert!(
        !parent_gates_tests_module_behind_cfg_test(root, relative).expect("read must succeed"),
        "an inline `mod tests {{ ... }}` body must not verify an external \
         tests.rs file's wiring"
    );
}

/// Regression for the sixth near-miss on this gate: the banned-pattern match
/// is a plain `str::contains`, so it is exact-spelling-only. Valid,
/// semantically identical Rust written with extra whitespace — `.dangerous
/// ()` instead of `.dangerous()`, `RootCertStore :: empty ( )` instead of
/// `RootCertStore::empty()` — compiles to the exact same escape hatch but
/// does not match the literal substring, so this gate reported clean while
/// scanning code that genuinely hand-rolls a permissive connector.
#[test]
fn gate_catches_a_banned_call_written_with_extra_whitespace() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let sandbox_dir = sandbox_process_dir(root);
    std::fs::create_dir_all(&sandbox_dir).expect("create sandbox_process dir");

    std::fs::write(
        sandbox_dir.join("tls_intercept.rs"),
        "pub(crate) fn build_permissive_store() -> rustls::RootCertStore {\n    \
             rustls::RootCertStore :: empty ( )\n}\n",
    )
    .expect("write tls_intercept.rs");

    let mut hits = Vec::new();
    scan_dir(root, &sandbox_dir, &mut hits).expect("scan must succeed");
    assert!(
        !hits.is_empty(),
        "a whitespace-varied spelling of the banned call \
         (`RootCertStore :: empty ( )`) must still be caught, not silently \
         passed because the exact-substring match didn't line up"
    );
}

/// Same near-miss as above, for the `.dangerous(` spelling specifically —
/// kept in a separate fixture from `RootCertStore :: empty ( )` so the
/// function name chosen for the fixture can't itself accidentally contain
/// the banned substring (as `build_dangerous()` would for `dangerous(`).
#[test]
fn gate_catches_dangerous_written_with_extra_whitespace() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let sandbox_dir = sandbox_process_dir(root);
    std::fs::create_dir_all(&sandbox_dir).expect("create sandbox_process dir");

    std::fs::write(
        sandbox_dir.join("tls_intercept.rs"),
        "pub(crate) fn build_permissive_config() -> rustls::ClientConfig {\n    \
             rustls::ClientConfig::builder().dangerous ()\n}\n",
    )
    .expect("write tls_intercept.rs");

    let mut hits = Vec::new();
    scan_dir(root, &sandbox_dir, &mut hits).expect("scan must succeed");
    assert!(
        !hits.is_empty(),
        "a whitespace-varied spelling of the banned call (`.dangerous ()`) \
         must still be caught, not silently passed because the \
         exact-substring match didn't line up"
    );
}

/// Regression for the seventh near-miss: `truncate_at_inline_test_module`
/// truncates the scan at the `mod tests { ... }` marker line itself, not at
/// the end of that module's body. The module doc documents "files in this
/// crate keep `mod tests` at the end of the file" as an *assumption*, not
/// something the scanner verifies — so production code placed textually
/// after a closing `}` that ends an inline `mod tests { ... }` block, but
/// still on a line at or after the truncation offset, was silently dropped
/// from the scan along with the real test module. Plants a banned call in
/// exactly that position: after the inline test module's own closing brace.
#[test]
fn gate_catches_production_code_that_follows_an_inline_test_module_body() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let sandbox_dir = sandbox_process_dir(root);
    std::fs::create_dir_all(&sandbox_dir).expect("create sandbox_process dir");

    std::fs::write(
        sandbox_dir.join("tls_intercept.rs"),
        "pub(crate) fn noop() {}\n\n\
         #[cfg(test)]\n\
         mod tests {\n    \
             fn harmless() {\n        \
                 let _ = 1 + 1;\n    \
             }\n\
         }\n\n\
         pub(crate) fn build_dangerous() -> rustls::ClientConfig {\n    \
             rustls::ClientConfig::builder().dangerous()\n\
         }\n",
    )
    .expect("write tls_intercept.rs");

    let mut hits = Vec::new();
    scan_dir(root, &sandbox_dir, &mut hits).expect("scan must succeed");
    assert!(
        !hits.is_empty(),
        "a banned call in production code placed after an inline \
         `mod tests {{ ... }}` block's own closing brace must still be \
         caught, not silently dropped along with the test module it follows"
    );
}

/// Regression for the ninth near-miss: the banned-pattern match used to run
/// **per line** (`code_only.lines().enumerate()`), so valid Rust with the
/// call itself split across a line break — `.dangerous\n()` or
/// `RootCertStore::\nempty()` — never appeared whole on any single line and
/// slipped past every per-line check, including the whitespace-widened one
/// (widening a match only ever happened *within* a line). `scan_dir` now
/// matches across the whole stripped file at once via [`Haystack`], a
/// single whitespace-free buffer with a per-character line-number index, so
/// a call split across any number of line breaks is still caught, and still
/// reported at the source line the match begins on.
#[test]
fn gate_catches_a_banned_call_split_across_a_line_break() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let sandbox_dir = sandbox_process_dir(root);
    std::fs::create_dir_all(&sandbox_dir).expect("create sandbox_process dir");

    std::fs::write(
        sandbox_dir.join("tls_intercept.rs"),
        "pub(crate) fn build_permissive_config() -> rustls::ClientConfig {\n    \
             rustls::ClientConfig::builder().dangerous\n        ()\n}\n",
    )
    .expect("write tls_intercept.rs");

    let mut hits = Vec::new();
    scan_dir(root, &sandbox_dir, &mut hits).expect("scan must succeed");
    assert!(
        !hits.is_empty(),
        "a banned call split across a line break (`.dangerous\\n()`) must \
         still be caught, not silently passed because the match never \
         crossed a line boundary"
    );
}

/// The sanctioned `RootCertStore::empty()` carve-out must still apply when
/// that exact call is itself split across a line break inside
/// `from_system_roots`'s own body — the multiline matcher must not turn the
/// legitimate call site into a false positive.
#[test]
fn gate_still_exempts_the_sanctioned_call_when_split_across_a_line_break() {
    let relative = "crates/ironclaw_host_runtime/src/sandbox_process/tls_intercept.rs";
    let contents = "pub(crate) fn from_system_roots() -> Result<Self, ()> {\n    \
        let mut store = rustls::RootCertStore::\n        empty();\n    Ok(Self(store))\n}\n";
    let production_only = truncate_at_inline_test_module(contents);
    let code_only = strip_comments_and_strings(&production_only);
    let sanctioned_lines = sanctioned_from_system_roots_lines(relative, &code_only);
    assert!(
        !sanctioned_lines.is_empty(),
        "a `RootCertStore::empty()` call split across a line break inside \
         from_system_roots's own body must still be recognized as the \
         sanctioned call site"
    );
}

/// Regression for the eighth near-miss: `truncate_at_inline_test_module`
/// counted brace depth from the **raw** line bytes, before
/// `strip_comments_and_strings` ever runs (that stripping only happens
/// afterward, in `scan_dir`, on the already-truncated output). A string
/// literal inside the inline test module's own body that contains a literal
/// `{` (with no matching `}` on the same line) throws off that raw count —
/// the body never reaches `depth == 0`, so the "read until the inline
/// module's own closing brace" loop keeps consuming lines past the test
/// module's real end, swallowing whatever production code follows,
/// including a banned escape-hatch call — the same class of hole the
/// seventh near-miss closed for the marker-line case, but reachable here
/// even though the marker line itself is matched correctly. Counting depth
/// from the comment/string-stripped line closes it.
#[test]
fn gate_catches_production_code_after_a_test_module_body_containing_a_literal_brace_in_a_string() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let sandbox_dir = sandbox_process_dir(root);
    std::fs::create_dir_all(&sandbox_dir).expect("create sandbox_process dir");

    std::fs::write(
        sandbox_dir.join("tls_intercept.rs"),
        "pub(crate) fn noop() {}\n\n\
         #[cfg(test)]\n\
         mod tests {\n    \
             fn harmless() {\n        \
                 let _ = \"{\";\n    \
             }\n\
         }\n\n\
         pub(crate) fn build_dangerous() -> rustls::ClientConfig {\n    \
             rustls::ClientConfig::builder().dangerous()\n\
         }\n",
    )
    .expect("write tls_intercept.rs");

    let mut hits = Vec::new();
    scan_dir(root, &sandbox_dir, &mut hits).expect("scan must succeed");
    assert!(
        !hits.is_empty(),
        "a banned call in production code placed after an inline test \
         module whose body contains a string literal with an unbalanced \
         literal brace must still be caught, not silently dropped because \
         raw byte brace-counting mistook the string's contents for real \
         code structure"
    );
}
