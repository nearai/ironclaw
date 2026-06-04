//! Architecture boundary: tool execution must flow through the audited funnel.
//!
//! Every tool invocation that originates from a non-agent caller (gateway
//! handlers, CLI commands, the routine engine, WASM channels, …) is supposed
//! to flow through [`ToolDispatcher::dispatch`] (`src/tools/dispatch.rs`).
//! `dispatch` is the *only* path that builds an `ActionRecord` audit entry and
//! applies the channel tool-permit filter. The lower-level primitive
//! `execute_tool_with_safety` (`src/tools/execute.rs`) — and a raw
//! `Tool::execute` call on a tool trait object — run the safety pipeline but
//! skip the audit record and the channel permit filter.
//!
//! Today, several production call sites reach the primitive (or call
//! `Tool::execute` directly) instead of going through `dispatch`. Tracking
//! issue #4017 documents the resulting audit/permit gap; issue #4019 is the
//! migration that closes it. This test is **#4019 step 1**: it makes the
//! current bypass set explicit and *ratchets* it — the test passes on today's
//! tree, but fails the moment anyone adds a NEW direct tool-execution call
//! site outside the audited funnel.
//!
//! ## Green ratchet, not a red test
//!
//! This is deliberately not a failing test. The [`ALLOWLIST`] below is the
//! exhaustive, hand-audited set of production locations that currently call
//! the un-audited primitive or invoke `Tool::execute` directly. It is also the
//! **#4019 migration checklist**: as steps 3–6 route each caller through
//! `dispatch`, the corresponding `Bypass` entries are deleted from the
//! allowlist, and the list shrinks until only the legitimate executor(s)
//! remain (`dispatch.rs`, `execute.rs`, the worker loops).
//!
//! ## What is scanned
//!
//! Production Rust source only — `#[cfg(test)]` modules, `tests/`, `benches/`,
//! `examples/`, comments, and doc comments are excluded (mirroring the sibling
//! boundary tests in `reborn_dependency_boundaries.rs`). Two call shapes are
//! flagged:
//!   * `execute_tool_with_safety(` and its `pub` String-error wrapper
//!     `execute_tool_simple(` — the un-audited primitives, anywhere.
//!   * `.execute(` on a tool trait object — but only within the
//!     tool-execution subsystems (`src/tools/`, `src/worker/`, `src/agent/`,
//!     `src/bridge/`), where a bare `.execute(` reliably means `Tool::execute`
//!     rather than a DB statement, OS process, or HTTP client. Both same-line
//!     (`tool.execute(`) and multi-line (`tool\n    .execute(`) call shapes are
//!     flagged — the multi-line, leading-dot idiom is the dominant one in the
//!     codebase.
//!
//! See #4019 / #4017.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// A production call site permitted to invoke tool execution outside the
/// audited `ToolDispatcher::dispatch` funnel.
struct AllowedSite {
    /// Workspace-relative file path.
    file: &'static str,
    /// Why this site is allowed. `Executor` sites are the legitimate
    /// terminal executors (the audited funnel, the shared primitive, the
    /// worker loops). `Bypass` sites are the #4019 migration checklist —
    /// each must move through `dispatch` and then be deleted from this list.
    kind: AllowKind,
}

enum AllowKind {
    /// Legitimate terminal executor — stays allowlisted permanently.
    Executor,
    /// Un-migrated bypass tracked by #4019. Delete when migrated.
    Bypass,
}

/// The audited baseline. Every entry here was confirmed by reading the call
/// site. The `Bypass` entries are the #4019 step 3/4/5 migration checklist.
const ALLOWLIST: &[AllowedSite] = &[
    // --- Legitimate executors (permanent) ---
    // The audited funnel itself: builds the ActionRecord, applies the
    // channel tool-permit filter, then calls `tool.execute`.
    AllowedSite {
        file: "src/tools/dispatch.rs",
        kind: AllowKind::Executor,
    },
    // The shared primitive + its String-error wrapper (`execute_tool_simple`).
    // `dispatch` is the audited caller of this; the bypass sites below are not.
    AllowedSite {
        file: "src/tools/execute.rs",
        kind: AllowKind::Executor,
    },
    // Background-job worker agentic loop — the v1 `Worker::execute_tool`
    // equivalent; the loop owns its own sequence tracking.
    AllowedSite {
        file: "src/worker/job.rs",
        kind: AllowKind::Executor,
    },
    // Container worker agentic loop (calls `execute_tool_simple`).
    AllowedSite {
        file: "src/worker/container.rs",
        kind: AllowKind::Executor,
    },
    // --- Un-migrated bypasses: #4019 migration checklist ---
    // Interactive chat tool calls (parallel JoinSet path) — the headline
    // bypass from #4017. // TODO(#4019): migrate through audited dispatch (step 3).
    AllowedSite {
        file: "src/agent/dispatcher.rs",
        kind: AllowKind::Bypass,
    },
    // Scheduler autonomous tool execution.
    // TODO(#4019): migrate through audited dispatch (step 4).
    AllowedSite {
        file: "src/agent/scheduler.rs",
        kind: AllowKind::Bypass,
    },
    // Engine v2 effect bridge (Python orchestrator path).
    // TODO(#4019): migrate through audited dispatch (step 5).
    AllowedSite {
        file: "src/bridge/effect_adapter.rs",
        kind: AllowKind::Bypass,
    },
    // Routine engine tool execution.
    // TODO(#4019): migrate through audited dispatch (step 4).
    AllowedSite {
        file: "src/agent/routine_engine.rs",
        kind: AllowKind::Bypass,
    },
    // CLI `/restart` command runs RestartTool directly.
    // TODO(#4019): migrate through audited dispatch (step 5).
    AllowedSite {
        file: "src/agent/commands.rs",
        kind: AllowKind::Bypass,
    },
    // Tool-builder verification: runs a freshly built tool during the build
    // flow. // TODO(#4019): migrate through audited dispatch (step 5).
    AllowedSite {
        file: "src/tools/builder/core.rs",
        kind: AllowKind::Bypass,
    },
    // Tool-builder test harness: runs a built WASM tool against its test
    // cases. // TODO(#4019): migrate through audited dispatch (step 5).
    AllowedSite {
        file: "src/tools/builder/testing.rs",
        kind: AllowKind::Bypass,
    },
];

/// Directories where a bare `.execute(` reliably denotes `Tool::execute`
/// rather than an unrelated `.execute(` (DB statement, OS process, HTTP
/// client, …). The `execute_tool_with_safety(` matcher is scanned across all
/// of `src/`; the `.execute(`-on-a-tool matcher is restricted to these.
const TOOL_EXECUTION_SUBSYSTEMS: &[&str] = &["src/tools", "src/worker", "src/agent", "src/bridge"];

#[test]
fn tool_execution_flows_through_audited_dispatch_funnel() {
    let root = workspace_root();

    let mut found: BTreeSet<String> = BTreeSet::new();

    // 1. The un-audited primitives anywhere in production `src/`:
    //    `execute_tool_with_safety(` and its `pub` String-error wrapper
    //    `execute_tool_simple(` (a live entry point at `worker/container.rs`).
    collect_callers(&root.join("src"), &root, &is_primitive_call, &mut found);

    // 2. `.execute(` on a tool object, restricted to the tool-execution
    //    subsystems where the call shape is unambiguous.
    for subsystem in TOOL_EXECUTION_SUBSYSTEMS {
        let dir = root.join(subsystem);
        if !dir.exists() {
            continue;
        }
        collect_callers(&dir, &root, &is_tool_execute_call, &mut found);
    }

    let allowed: BTreeSet<String> = ALLOWLIST.iter().map(|site| site.file.to_string()).collect();

    // New bypasses: a production file that calls into tool execution but is
    // not on the allowlist.
    let new_bypasses: Vec<&String> = found.difference(&allowed).collect();
    assert!(
        new_bypasses.is_empty(),
        "New direct tool-execution call site outside the audited funnel — route it \
         through `ToolDispatcher::dispatch` or, if genuinely exempt, add it to the \
         allowlist in this test with a justification. See #4019/#4017.\n\
         Offending file(s):\n{}",
        new_bypasses
            .iter()
            .map(|file| format!("  - {file}"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    // Stale allowlist entries: a `Bypass` site that no longer calls into tool
    // execution has been migrated — delete it so the checklist shrinks. (We do
    // not enforce this on `Executor` entries, which may legitimately stop
    // matching, e.g. after a refactor of the primitive.)
    let stale: Vec<&str> = ALLOWLIST
        .iter()
        .filter(|site| matches!(site.kind, AllowKind::Bypass))
        .map(|site| site.file)
        .filter(|file| !found.contains(*file))
        .collect();
    assert!(
        stale.is_empty(),
        "Allowlisted #4019 bypass(es) no longer call tool execution directly — they have \
         been migrated through `ToolDispatcher::dispatch`. Delete them from the allowlist \
         in this test so it stays an accurate migration checklist:\n{}",
        stale
            .iter()
            .map(|file| format!("  - {file}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The un-audited tool-execution primitives, matched anywhere in `src/`.
///
/// `execute_tool_with_safety(` is the shared primitive; `execute_tool_simple(`
/// is its `pub` String-error wrapper (`src/tools/execute.rs`), which forwards to
/// the same primitive (same audit/permit skip) and is itself a live entry point
/// (`src/worker/container.rs`). A new caller of either is a bypass.
fn is_primitive_call(lines: &[&str], index: usize) -> bool {
    let line = lines[index];
    line.contains("execute_tool_with_safety(") || line.contains("execute_tool_simple(")
}

/// Heuristic for a `Tool::execute` invocation on a trait object.
///
/// Matches a `<receiver>.execute(` call where the receiver identifier is
/// `tool` or ends in `tool` / `_tool` (case-insensitive) — e.g. `tool`,
/// `restart_tool`, `self.tool`. It deliberately excludes the unrelated
/// `.execute(` receivers that share the tool subsystems: SQL handles
/// (`tx.execute(`, `conn.execute(`, `stmt.execute(`), HTTP/process clients,
/// etc. It also excludes the trait/impl method *definitions* (`fn execute(`,
/// `async fn execute(`), which have no receiver and no leading dot.
///
/// Two call shapes are handled:
///   * same-line — `let r = tool.execute(...)`: the receiver is the identifier
///     immediately preceding `.execute(` on the same line.
///   * multi-line — `let r = tool\n    .execute(...)`: the dominant idiom in the
///     codebase, where `.execute(` sits alone on its line with an empty
///     same-line receiver. The receiver is then the trailing identifier of the
///     previous non-blank code line.
fn is_tool_execute_call(lines: &[&str], index: usize) -> bool {
    let line = lines[index];
    let Some(prefix_end) = line.find(".execute(") else {
        return false;
    };
    // Same-line receiver: the identifier immediately preceding `.execute(`.
    let receiver = trailing_identifier(&line[..prefix_end]);
    if !receiver.is_empty() {
        return is_tool_like(&receiver);
    }
    // Empty same-line receiver: this is a leading-dot continuation. Only treat
    // it as a continuation if everything before `.execute(` on this line is
    // whitespace (the dominant multi-line idiom); otherwise the empty receiver
    // came from punctuation like `).execute(` (a method-chain result), which is
    // not a bare tool receiver.
    if !line[..prefix_end].trim().is_empty() {
        return false;
    }
    // Look back to the previous non-blank code line and take its trailing
    // identifier as the receiver.
    let mut prev = index;
    while prev > 0 {
        prev -= 1;
        let candidate = lines[prev].trim_end();
        if candidate.trim().is_empty() {
            continue;
        }
        return is_tool_like(&trailing_identifier(candidate));
    }
    false
}

/// The trailing `[A-Za-z0-9_]` identifier of `text` (empty if `text` does not
/// end in an identifier character).
fn trailing_identifier(text: &str) -> String {
    text.chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

/// Whether an identifier names a tool receiver (`tool` / `*_tool` / `*tool`,
/// case-insensitive).
fn is_tool_like(identifier: &str) -> bool {
    let lower = identifier.to_ascii_lowercase();
    lower == "tool" || lower.ends_with("_tool") || lower.ends_with("tool")
}

/// Recursively scan production `.rs` files under `dir`, collecting the
/// workspace-relative paths of any file with a line matching `predicate`.
///
/// Excludes `#[cfg(test)]` modules (everything from the first line containing
/// `#[cfg(test)]` to end of file, matching the convention that test modules
/// live at the bottom of a file), line comments, and doc comments.
fn collect_callers(
    dir: &Path,
    root: &Path,
    predicate: &dyn Fn(&[&str], usize) -> bool,
    found: &mut BTreeSet<String>,
) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| panic!("failed to read dir entry: {error}"));
        let path = entry.path();
        if path.is_dir() {
            // Skip the conventional non-production directories.
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, "tests" | "benches" | "examples" | "target") {
                continue;
            }
            collect_callers(&path, root, predicate, found);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        if file_has_caller(&contents, predicate) {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            // Normalize to forward slashes so the allowlist is
            // platform-independent.
            found.insert(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// Scan one file's contents for a predicate match against production code.
///
/// Produces the production code-line view (trailing `#[cfg(test)]` module
/// dropped, line/doc comments blanked) and runs `predicate` over it, giving the
/// predicate access to the full window so multi-line call chains are visible.
/// Comments are blanked rather than removed so blank-line-skipping look-back in
/// the predicate still sees the original line geometry.
fn file_has_caller(contents: &str, predicate: &dyn Fn(&[&str], usize) -> bool) -> bool {
    let code_lines = production_code_lines(contents);
    (0..code_lines.len()).any(|index| predicate(&code_lines, index))
}

/// The production code-line view of a file: the trailing `#[cfg(test)]` module
/// is dropped, and line/doc comments are blanked to empty strings.
fn production_code_lines(contents: &str) -> Vec<&str> {
    let lines: Vec<&str> = contents.lines().collect();
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start();
        // A `#[cfg(test)]` guarding a `mod` is the trailing test module —
        // everything from here to EOF is test code, so stop scanning. A
        // `#[cfg(test)]` on a single item (e.g. a test-only `use` near the top
        // of the file) must NOT terminate the scan: skip only the guarded item.
        if trimmed.starts_with("#[cfg(test)]") {
            // Peek at the next non-attribute line to classify the guard.
            let mut peek = index + 1;
            while peek < lines.len() && lines[peek].trim_start().starts_with("#[") {
                peek += 1;
            }
            let guarded = lines.get(peek).map(|l| l.trim_start()).unwrap_or("");
            if guarded.starts_with("mod ")
                || guarded.starts_with("pub mod ")
                || guarded.starts_with("pub(crate) mod ")
            {
                break;
            }
            // Inline test-only item: blank the attribute line and continue.
            out.push("");
            index += 1;
            continue;
        }
        // Blank line comments and doc comments (keep the slot so multi-line
        // look-back preserves line geometry).
        if trimmed.starts_with("//") {
            out.push("");
        } else {
            out.push(line);
        }
        index += 1;
    }
    out
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("architecture crate under crates/")
        .to_path_buf()
}

#[cfg(test)]
mod scanner_fixtures {
    use super::*;

    /// The multi-line, leading-dot idiom — the dominant tool-execute shape in
    /// the codebase. Before the fix this was invisible (empty same-line
    /// receiver → predicate returned `false`).
    #[test]
    fn detects_multiline_tool_execute() {
        let src = "\
fn run(tool: &dyn Tool) {
    let result = tool
        .execute(args, &ctx)
        .await;
}
";
        assert!(file_has_caller(src, &is_tool_execute_call));
    }

    #[test]
    fn detects_multiline_tool_execute_field_receiver() {
        let src = "\
fn run(&self) {
    let result = self.restart_tool
        .execute(args, &ctx)
        .await;
}
";
        assert!(file_has_caller(src, &is_tool_execute_call));
    }

    #[test]
    fn detects_same_line_tool_execute() {
        let src = "    let r = tool.execute(args, &ctx).await;\n";
        assert!(file_has_caller(src, &is_tool_execute_call));
    }

    /// `execute_tool_simple(` is the `pub` String-error wrapper (a live entry
    /// point at `worker/container.rs`); a new caller must be flagged.
    #[test]
    fn detects_execute_tool_simple() {
        let src = "\
fn run() {
    let result = execute_tool_simple(
        &self.tools,
        &self.safety,
    )
    .await;
}
";
        assert!(file_has_caller(src, &is_primitive_call));
    }

    #[test]
    fn detects_execute_tool_with_safety() {
        let src = "    let r = execute_tool_with_safety(&tools, &safety).await;\n";
        assert!(file_has_caller(src, &is_primitive_call));
    }

    /// DB / non-tool receivers on the same line must not be flagged.
    #[test]
    fn ignores_db_receivers_same_line() {
        for src in [
            "    conn.execute(sql).await?;\n",
            "    tx.execute(stmt, params).await?;\n",
            "    statement.execute(&[]).await?;\n",
        ] {
            assert!(
                !file_has_caller(src, &is_tool_execute_call),
                "false positive on: {src}"
            );
        }
    }

    /// A leading-dot `.execute(` whose previous code line ends in a non-tool
    /// identifier (e.g. a DB handle) must not be flagged.
    #[test]
    fn ignores_multiline_db_receiver() {
        let src = "\
fn run(conn: &Conn) {
    let rows = conn
        .execute(sql)
        .await?;
}
";
        assert!(!file_has_caller(src, &is_tool_execute_call));
    }

    /// A method-chain result (`).execute(`) is not a bare tool receiver; the
    /// empty same-line receiver must not trigger the multi-line look-back.
    #[test]
    fn ignores_method_chain_result() {
        let src = "    let r = build_query(tool).execute(args).await;\n";
        assert!(!file_has_caller(src, &is_tool_execute_call));
    }

    /// A `.execute(` hidden in a line comment is not a real call.
    #[test]
    fn ignores_commented_tool_execute() {
        let src = "    // tool.execute(args);\n";
        assert!(!file_has_caller(src, &is_tool_execute_call));
    }

    /// A multi-line call inside a trailing `#[cfg(test)]` module is test code,
    /// not production, and must not be flagged.
    #[test]
    fn ignores_tool_execute_in_test_module() {
        let src = "\
fn prod() {}

#[cfg(test)]
mod tests {
    fn t(tool: &dyn Tool) {
        let r = tool
            .execute(args)
            .await;
    }
}
";
        assert!(!file_has_caller(src, &is_tool_execute_call));
    }
}
