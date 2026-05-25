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
//!   * `execute_tool_with_safety(` — the un-audited primitive, anywhere.
//!   * `.execute(` on a tool trait object — but only within the
//!     tool-execution subsystems (`src/tools/`, `src/worker/`, `src/agent/`,
//!     `src/bridge/`), where a bare `.execute(` reliably means `Tool::execute`
//!     rather than a DB statement, OS process, or HTTP client.
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

    // 1. `execute_tool_with_safety(` anywhere in production `src/`.
    collect_callers(
        &root.join("src"),
        &root,
        &|line| line.contains("execute_tool_with_safety("),
        &mut found,
    );

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

/// Heuristic for a `Tool::execute` invocation on a trait object.
///
/// Matches a `<receiver>.execute(` call where the receiver identifier is
/// `tool` or ends in `tool` / `_tool` (case-insensitive) — e.g. `tool`,
/// `restart_tool`, `self.tool`. This is the shape every current tool-execute
/// call site uses, and it deliberately excludes the unrelated `.execute(`
/// receivers that share the tool subsystems: SQL handles (`tx.execute(`,
/// `conn.execute(`, `stmt.execute(`), HTTP/process clients, etc. It also
/// excludes the trait/impl method *definitions* (`fn execute(`,
/// `async fn execute(`), which have no receiver and no leading dot.
fn is_tool_execute_call(line: &str) -> bool {
    let Some(prefix_end) = line.find(".execute(") else {
        return false;
    };
    // The receiver is the identifier immediately preceding `.execute(`.
    let receiver: String = line[..prefix_end]
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let receiver = receiver.to_ascii_lowercase();
    receiver == "tool" || receiver.ends_with("_tool") || receiver.ends_with("tool")
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
    predicate: &dyn Fn(&str) -> bool,
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
        let lines: Vec<&str> = contents.lines().collect();
        let mut index = 0;
        while index < lines.len() {
            let line = lines[index];
            let trimmed = line.trim_start();
            // A `#[cfg(test)]` guarding a `mod` is the trailing test module —
            // everything from here to EOF is test code, so stop scanning.
            // A `#[cfg(test)]` on a single item (e.g. a test-only `use` near
            // the top of the file) must NOT terminate the scan: skip only the
            // guarded item.
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
                // Inline test-only item: skip the attribute line and continue.
                index += 1;
                continue;
            }
            // Skip line comments and doc comments.
            if trimmed.starts_with("//") {
                index += 1;
                continue;
            }
            if predicate(line) {
                let relative = path.strip_prefix(root).unwrap_or(&path);
                // Normalize to forward slashes so the allowlist is
                // platform-independent.
                found.insert(relative.to_string_lossy().replace('\\', "/"));
                break;
            }
            index += 1;
        }
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("architecture crate under crates/")
        .to_path_buf()
}
