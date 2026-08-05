//! Gate for the `tracing` metadata-target syntax (#7146).
//!
//! `tracing::warn!(target = "…")` does **not** set the event's metadata target.
//! `=` is the field-assignment operator, so it records a *field* named `target`
//! and leaves `event.metadata().target()` as the emitting module path. The
//! macro syntax that sets the metadata target is `target: "…"`.
//!
//! The two forms differ by one character, and neither the compiler nor clippy
//! says anything about the wrong one, so the failure is silent and in the worst
//! direction: an operator running the documented
//! `RUST_LOG=ironclaw::reborn::cli::serve=debug` filter gets nothing back while
//! the events *are* being emitted, and concludes the code path never ran.
//!
//! #7146 found the tree carrying both forms — 120 field-form sites against 53
//! correct ones — which is drift rather than one mistake, so it needs a gate and
//! not just a sweep. [`tracing_macros_set_the_metadata_target`] is that gate;
//! [`metadata_target_only_follows_the_colon_form`] pins the language fact the
//! gate rests on, so the gate cannot outlive its own premise.

#[allow(dead_code)]
mod ratchet_support;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use ratchet_support::{strip_comments_and_strings, workspace_root};

/// Macro names whose first argument may be a metadata target. Matched on the
/// last path segment, so `tracing::warn!`, a bare `warn!` imported through
/// `use tracing::warn`, and `log::warn!` are all covered — they share the
/// `target:` grammar and the same field-form trap.
const TARGET_BEARING_MACROS: &[&str] =
    &["trace", "debug", "info", "warn", "error", "event", "span"];

/// The one exclusion, and it is this file: the premise probe below emits the
/// field form deliberately. There is no allowlist for production code — a field
/// that genuinely means "target" gets a different name (see
/// `ironclaw_loop_host::tool_disclosure_port`, which spells it `tool`), so the
/// rule stays absolute and cannot rot into a parking lot.
const SELF_FILE: &str = "reborn_tracing_target_syntax.rs";

/// One `<macro>!(target = …)` site: the field form where the metadata form was
/// meant.
#[derive(Debug, PartialEq, Eq)]
struct FieldFormTarget {
    line: usize,
    macro_name: String,
}

/// Scans `source` for target-bearing macro invocations whose **first** argument
/// is `target =` rather than `target:`.
///
/// Deliberately a lexer over the source rather than a `syn` walk: a
/// `tracing::warn!` nested inside another macro's token stream is invisible to
/// `syn` — it never parses an unknown macro's body — but is still a real
/// emission site. Only the first argument is inspected, because that is the only
/// position `tracing`'s grammar reads as a target; a `target` field later in the
/// argument list is an ordinary field and stays legal.
fn field_form_targets(source: &str) -> Vec<FieldFormTarget> {
    let bytes = source.as_bytes();
    let mut found = Vec::new();

    for (bang, _) in source.match_indices('!') {
        // Walk back over the macro path's last segment.
        let mut name_start = bang;
        while name_start > 0 && is_ident_byte(bytes[name_start - 1]) {
            name_start -= 1;
        }
        if name_start == bang {
            continue;
        }
        let macro_name = &source[name_start..bang];
        if !TARGET_BEARING_MACROS.contains(&macro_name) {
            continue;
        }
        // Requiring `(` rules out `!=` and a bare path mention.
        if bytes.get(bang + 1) != Some(&b'(') {
            continue;
        }
        let first_arg = skip_trivia(source, bang + 2);
        if !source[first_arg..].starts_with("target") {
            continue;
        }
        let after_target = first_arg + "target".len();
        if bytes
            .get(after_target)
            .is_some_and(|byte| is_ident_byte(*byte))
        {
            continue;
        }
        // `=` is the field form. `:` is the metadata form and anything else is
        // not a target argument at all.
        if bytes.get(skip_trivia(source, after_target)) == Some(&b'=') {
            found.push(FieldFormTarget {
                line: line_of(source, bang),
                macro_name: macro_name.to_string(),
            });
        }
    }

    found
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Advances past whitespace and `//` / `/* */` comments. The repo-wide scan
/// pre-strips comments; the self-test does not, and rustfmt is free to park one
/// between the parenthesis and the first argument.
fn skip_trivia(source: &str, mut cursor: usize) -> usize {
    let bytes = source.as_bytes();
    loop {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let rest = &source[cursor.min(source.len())..];
        if let Some(stripped) = rest.strip_prefix("//") {
            cursor += 2 + stripped.find('\n').map_or(stripped.len(), |end| end);
        } else if let Some(stripped) = rest.strip_prefix("/*") {
            cursor += 2 + stripped.find("*/").map_or(stripped.len(), |end| end + 2);
        } else {
            return cursor;
        }
    }
}

fn line_of(source: &str, offset: usize) -> usize {
    source[..offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

/// Fail-closed traversal: a directory or entry this scan cannot read is a
/// broken gate, not a file to skip — silently narrowing the scan is how a
/// regression gate goes green while enforcing nothing (the same shape the
/// `scanned > 100` floor below guards from the other side).
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("scan cannot read directory {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!("scan cannot read an entry of {}: {error}", dir.display())
        });
        let path = entry.path();
        if path.is_dir() {
            if path
                .file_name()
                .is_some_and(|name| name == "target" || name == "node_modules")
            {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn tracing_macros_set_the_metadata_target() {
    let crates_dir = workspace_root().join("crates");
    let mut files = Vec::new();
    rust_files(&crates_dir, &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "found no Rust files under {} — the scan would pass vacuously",
        crates_dir.display()
    );

    let mut report = String::new();
    let mut violations = 0usize;
    let mut scanned = 0usize;
    for file in &files {
        // This file emits the field form on purpose, to measure what it does.
        if file.file_name().is_some_and(|name| name == SELF_FILE) {
            continue;
        }
        let source = std::fs::read_to_string(file).unwrap_or_else(|error| {
            panic!(
                "scan cannot read {}: {error} — an unreadable production source \
                 must fail the gate, not shrink it",
                file.display()
            )
        });
        scanned += 1;
        // Comments and string bodies are stripped so a doc comment quoting the
        // wrong form is not a build failure. Newlines survive, so line numbers
        // still point at the real site.
        for hit in field_form_targets(&strip_comments_and_strings(&source)) {
            violations += 1;
            let _ = writeln!(
                report,
                "  {}:{} — {}!(target = …)",
                file.display(),
                hit.line,
                hit.macro_name
            );
        }
    }

    assert!(
        scanned > 100,
        "only {scanned} files reached the scan — the walk or the exclusion is \
         wrong, and a scan that reads nothing passes for the wrong reason"
    );
    assert!(
        violations == 0,
        "{violations} tracing call site(s) use `target = …`, which records a \
         FIELD named `target` and leaves the event's metadata target as the \
         module path — so `RUST_LOG` filters naming that target never match. \
         Use `target: …` (colon). See #7146:\n{report}"
    );
}

/// The scan is only worth having if it actually flags the field form, so drive
/// it over a sample carrying every shape that matters. Without this, a scanner
/// that silently matched nothing would report a clean tree forever.
#[test]
fn field_form_scan_flags_only_the_field_form() {
    let sample = concat!(
        "\n",
        "tracing::warn!(target: \"ironclaw::right\", \"colon form sets metadata\");\n",
        "tracing::warn!(target = \"ironclaw::wrong\", \"same-line field form\");\n",
        "tracing::debug!(\n",
        "    target = \"ironclaw::wrong_multiline\",\n",
        "    \"own-line field form — the shape rustfmt produces\",\n",
        ");\n",
        "warn!(target = \"ironclaw::wrong_bare\", \"bare macro via use tracing::warn\");\n",
        "tracing::info!(\n",
        "    // an intervening comment must not hide the operator\n",
        "    target = \"ironclaw::wrong_after_comment\",\n",
        ");\n",
        "tracing::debug!(\"message first\", target = \"a-real-field-not-a-target\");\n",
        "some_other_macro!(target = \"ironclaw::not-a-tracing-macro\");\n",
        "targeted_warn!(target = \"ironclaw::different-macro-name\");\n",
        "let targeting = 1; assert!(targeting != 0);\n",
    );

    let hits = field_form_targets(sample);
    let got: Vec<(usize, &str)> = hits
        .iter()
        .map(|hit| (hit.line, hit.macro_name.as_str()))
        .collect();

    assert_eq!(
        got,
        vec![(3, "warn"), (4, "debug"), (8, "warn"), (9, "info")],
        "the scan must flag every first-argument `target =` in a target-bearing \
         macro, and nothing else"
    );
}

/// Pins the language fact the gate rests on: `target =` records a field and
/// leaves the metadata target as the module path, while `target:` sets it.
///
/// A gate whose premise lives only in a doc comment is one dependency release
/// away from being pointless. This measures both forms through a real
/// subscriber instead of asserting the claim.
#[test]
fn metadata_target_only_follows_the_colon_form() {
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
    use tracing_subscriber::registry::Registry;

    #[derive(Clone, Default)]
    struct TargetCapture(Arc<Mutex<Vec<String>>>);

    impl<S: tracing::Subscriber> Layer<S> for TargetCapture {
        fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(event.metadata().target().to_string());
        }
    }

    let capture = TargetCapture::default();
    let subscriber = Registry::default().with(capture.clone());
    tracing::subscriber::with_default(subscriber, || {
        tracing::warn!(target = "ironclaw::probe::field_form", "field form");
        tracing::warn!(target: "ironclaw::probe::metadata_form", "metadata form");
    });

    let captured = capture
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();

    assert_eq!(
        captured,
        vec![
            module_path!().to_string(),
            "ironclaw::probe::metadata_form".to_string(),
        ],
        "`target = \"…\"` must still resolve to the module path — which is why it \
         is a bug — and `target: \"…\"` must set the metadata target, which is why \
         the #7146 sweep is correct. If this ever changes, revisit #7146 before \
         relaxing the gate."
    );
}
