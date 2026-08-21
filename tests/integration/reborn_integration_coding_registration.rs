//! Reborn integration — pinned coding registration seam (issue #7392 slice 3).
//!
//! Drives the production first-party surface end to end through the REAL turn
//! stack (product workflow → turn coordinator → agent loop → real
//! `ironclaw_llm` decorator chain → scripted model at the vendor-SDK seam).
//!
//! 1. The model-visible tool surface advertises EXACTLY the pinned names
//!    `read`/`write`/`edit`/`glob`/`grep` with the pinned fixture schemas
//!    and supported descriptions on the provider payload. The legacy
//!    coding tools (`read_file`/`write_file`/`list_dir`/`apply_patch`,
//!    `result_read`) are absent, and the derived `builtin__glob` /
//!    `builtin__grep` spellings are gone: the benchmark surface is the exact
//!    pinned coding surface.
//! 2. A scripted `read` → `edit` (with the returned hashline tag) → `read`
//!    chain flows the exact pinned coding output shapes back as tool results
//!    (hashline header `[file#TAG]`, numbered rows; edit success header +
//!    preview), and the edit really mutates the workspace file.
//! 3. The derived spelling (`builtin__read`) of an overridden pinned coding
//!    tool does
//!    NOT resolve after the clean cutover: only the exact advertised name
//!    (`read`) resolves, and a model that insists on the derived encoding
//!    fails the turn with the unknown-tool category.
//! 4. A gated coding `write` raises a real `BlockedApproval` gate through the
//!    ordinary approval path and persists after approval.
//!
//! The production-shaped harness selects the canonical pinned coding package
//! via its focused coding-tools profile; there is no old/new factory split.
//!
//! Stack note: every test here runs on a dedicated 16 MiB-stack thread
//! ([`run_async_test_with_stack`]), mirroring `process_port.rs`'s
//! `live_shell_uses_local_process_port` and `reborn_sandbox_shell_turn.rs`.
//! The pinned-coding harness builds through the production-shaped composition
//! (`build_production_shaped`), whose debug async-state-machine chain alone
//! consumes >2 MiB of stack — over the default 2 MiB libtest thread stack —
//! BEFORE any turn runs or tool definitions are read. It is a deep-but-bounded
//! flat chain, not recursion (the deepest build leaf is reached once, at
//! depth 0; the golden default-surface tests ride the lighter hand-built
//! runtime path and do not overflow). CI covers the whole integration tier
//! with an 8 MiB `RUST_MIN_STACK` lane env; locally the 16 MiB thread matches
//! the existing convention for this exact build class.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;
use std::future::Future;
use support::pinned_coding_contract::{tool_prompt, tool_schema};

/// The six pinned coding tools and their provider names (must match the
/// fixture manifest's `tool_names` subset for `read`/`write`/`edit`/`glob`/
/// `grep`; `bash` is the OMP-ported process tool with an IronClaw-narrowed
/// schema and description).
const PINNED_CODING_TOOLS: [(&str, &str); 6] = [
    ("builtin.read", "read"),
    ("builtin.write", "write"),
    ("builtin.edit", "edit"),
    ("builtin.glob", "glob"),
    ("builtin.grep", "grep"),
    ("builtin.bash", "bash"),
];

/// The model-visible description for `tool`. `read` intentionally advertises
/// only the IronClaw-implemented subset; the others use pinned prompt bytes.
/// `bash` uses the OMP template rendered with IronClaw's surface flags.
fn pinned_description(tool: &str) -> String {
    if tool == "read" {
        ironclaw_extension_support::coding::pinned::pinned_assets::CODING_READ_DESCRIPTION
            .to_string()
    } else if tool == "bash" {
        ironclaw_extension_support::coding::pinned::pinned_assets::CODING_BASH_DESCRIPTION
            .to_string()
    } else {
        tool_prompt(tool)
    }
}

fn coding_schema(tool: &str) -> serde_json::Value {
    if tool == "read" {
        serde_json::from_str(
            ironclaw_extension_support::coding::pinned::pinned_assets::CODING_READ_SCHEMA,
        )
        .expect("IronClaw read schema is valid JSON")
    } else if tool == "bash" {
        serde_json::from_str(
            ironclaw_extension_support::coding::pinned::pinned_assets::CODING_BASH_SCHEMA,
        )
        .expect("IronClaw bash schema is valid JSON")
    } else {
        tool_schema(tool)
    }
}

fn output_text(value: &serde_json::Value) -> String {
    value
        .get("output")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("pinned coding tool result carries an output text: {value}"))
        .to_string()
}

/// The pinned coding surface advertises exactly the pinned names, schemas, and
/// descriptions on the model-visible provider payload, with the legacy
/// coding tools and their derived `builtin__*` spellings absent.
#[test]
fn coding_surface_advertises_exact_names_schemas_and_descriptions() {
    run_async_test_with_stack(
        "coding_surface_advertises_exact_names_schemas_and_descriptions",
        || async {
            let h = RebornIntegrationHarness::test_default()
                .with_coding_tools()
                .script([RebornScriptedReply::text("surface captured")])
                .build()
                .await
                .expect("harness builds");
            h.submit_turn("list your tools")
                .await
                .expect("turn completes");
            h.assert_system_prompt_contains("id: builtin.edit")
                .await
                .expect("the edit descriptor survives prompt materialization");

            let definitions = h.scripted_llm.captured_tool_definitions();
            let definitions = definitions.into_iter().flatten().collect::<Vec<_>>();
            assert!(
                !definitions.is_empty(),
                "the model request must carry tool definitions"
            );

            let mut seen = std::collections::HashMap::new();
            for definition in &definitions {
                seen.entry(definition.name.clone())
                    .and_modify(|count| *count += 1)
                    .or_insert(1);
                if let Some((_, pinned_name)) = PINNED_CODING_TOOLS
                    .iter()
                    .find(|(_, pinned_name)| *pinned_name == definition.name)
                {
                    let tool = pinned_name;
                    assert_eq!(
                        definition.parameters,
                        coding_schema(tool),
                        "schema for coding tool {tool} must match its registered contract"
                    );
                    assert_eq!(
                        definition.description,
                        pinned_description(tool),
                        "description for coding tool {tool} must byte-match the pinned fixture prompt"
                    );
                }
            }

            // The six pinned coding names are advertised EXACTLY once each.
            for (_, pinned_name) in PINNED_CODING_TOOLS {
                assert_eq!(
                    seen.get(pinned_name),
                    Some(&1),
                    "coding tool {pinned_name} must be advertised exactly once"
                );
            }

            // Derived spellings of the pinned coding tools and the retired
            // coding tools are absent.
            for retired in [
                "builtin__read",
                "builtin__write",
                "builtin__edit",
                "builtin__glob",
                "builtin__grep",
                "builtin__read_file",
                "builtin__write_file",
                "builtin__list_dir",
                "builtin__apply_patch",
                "builtin__result_read",
            ] {
                assert!(
                    !seen.contains_key(retired),
                    "retired tool {retired} must not remain after the atomic cutover"
                );
            }
        },
    );
}

/// The pinned `bash` tool executes through the selected local-host process
/// port, and its OMP-shaped output reaches the model through normal capability
/// dispatch rather than the legacy shell handler.
#[test]
fn coding_bash_executes_through_the_process_port() {
    run_async_test_with_stack("coding_bash_executes_through_the_process_port", || async {
        let h = RebornIntegrationHarness::test_default()
            .with_coding_tools()
            .script([
                RebornScriptedReply::tool_call(
                    "bash",
                    json!({ "command": "printf 'bash-port-ok'" }),
                ),
                RebornScriptedReply::text("command complete"),
            ])
            .build()
            .await
            .expect("harness builds");

        h.submit_turn("run the bash command")
            .await
            .expect("turn completes");
        h.assert_tool_invoked("builtin.bash")
            .await
            .expect("bash dispatches through the capability port");

        let bash_output = output_text(
            &h.tool_result_output("builtin.bash")
                .await
                .expect("bash result"),
        );
        assert!(
            bash_output.starts_with("bash-port-ok\n\nWall time: "),
            "bash result preserves command output and the OMP wall-time notice: {bash_output}"
        );
        assert!(
            bash_output.ends_with(" seconds"),
            "bash wall-time notice includes seconds: {bash_output}"
        );
    });
}

/// A scripted `read` → `edit` (anchored on the read's hashline tag) → `read`
/// chain through the real capability path: exact pinned coding output shapes
/// flow back as tool results and the edit really mutates the workspace file.
#[test]
fn coding_read_edit_read_chain_flows_exact_shapes() {
    run_async_test_with_stack("coding_read_edit_read_chain_flows_exact_shapes", || async {
        let content = "line1\nline2\nline3\n";
        let changed = "line1\nCHANGED\nline3\n";
        let tag = ironclaw_extension_support::coding::pinned::harness::compute_file_hash(content);

        let h = RebornIntegrationHarness::test_default()
            .with_coding_tools()
            .script([
                RebornScriptedReply::tool_call("read", json!({ "path": "/workspace/foo.txt" })),
                RebornScriptedReply::tool_call(
                    "edit",
                    json!({ "input": format!("[/workspace/foo.txt#{tag}]\nPUT 2:\n+CHANGED\n") }),
                ),
                RebornScriptedReply::tool_call("read", json!({ "path": "/workspace/foo.txt" })),
                RebornScriptedReply::text("edited"),
            ])
            .build()
            .await
            .expect("harness builds");
        // Seed the workspace file the coding tools will read/edit (the harness
        // workspace root backing the /workspace mount).
        let path = h
            .capability_recorder
            .workspace_file_path("foo.txt")
            .expect("host-runtime harness exposes the workspace root");
        std::fs::write(&path, content).expect("seed workspace file");
        h.submit_turn("read, edit, read the file")
            .await
            .expect("turn completes");

        // Read #1 saw the ORIGINAL content (numbered rows, hashline header).
        h.assert_tool_result_contains("[foo.txt#")
            .await
            .expect("read result carries the hashline header");
        h.assert_tool_result_contains("1:line1")
            .await
            .expect("read result carries numbered rows");
        h.assert_tool_result_contains("2:line2")
            .await
            .expect("the first read saw the original line 2");

        // The edit result is the exact success shape: refreshed snapshot header
        // + preview of the new line.
        let edit_output = output_text(
            &h.tool_result_output("builtin.edit")
                .await
                .expect("edit result"),
        );
        assert!(
            edit_output.starts_with("[/workspace/foo.txt#"),
            "edit output leads with the new snapshot header: {edit_output}"
        );
        assert!(
            edit_output.contains("2:CHANGED"),
            "edit preview shows the new line: {edit_output}"
        );

        // Read #2 sees the edited content (and only the edited content).
        let read2 = output_text(
            &h.tool_result_output("builtin.read")
                .await
                .expect("read result"),
        );
        assert!(
            read2.starts_with("[foo.txt#"),
            "read output leads with the hashline header: {read2}"
        );
        assert!(
            read2.contains("1:line1") && read2.contains("2:CHANGED") && read2.contains("3:line3"),
            "read #2 shows the edited file: {read2}"
        );
        assert!(
            !read2.contains("2:line2"),
            "read #2 must not show the stale line: {read2}"
        );

        // The edit really mutated the workspace file through RootFilesystem.
        h.assert_workspace_file_contains("foo.txt", changed)
            .await
            .expect("the edit persisted to the workspace file");
    });
}

/// The derived spelling of an overridden capability (`builtin__read`) does
/// NOT resolve after the clean cutover — the exact advertised name `read` is
/// the only resolvable spelling. The invalid call does not dispatch
/// `builtin.read`, and the model can recover on its next response.
#[test]
fn coding_derived_spelling_does_not_resolve() {
    run_async_test_with_stack("coding_derived_spelling_does_not_resolve", || async {
        let h = RebornIntegrationHarness::test_default()
            .with_coding_tools()
            .script([
                RebornScriptedReply::tool_call("builtin__read", json!({ "path": "foo.txt" })),
                RebornScriptedReply::text("the encoded spelling is unavailable"),
            ])
            .build()
            .await
            .expect("harness builds");

        h.submit_turn("read the file by its encoded name")
            .await
            .expect("the model recovers after the outside-surface result");
        h.assert_tool_not_invoked("builtin.read")
            .await
            .expect("the derived spelling must not dispatch the pinned read engine");
    });
}

/// A large pinned coding result is persisted before the model sees its
/// bounded preview, and the same run can recover a 3 KiB byte range through
/// `read artifact://` without recursively spilling that continuation into a
/// second artifact.
#[test]
fn coding_large_read_spills_and_is_readable_by_artifact_selector() {
    run_async_test_with_stack(
        "coding_large_read_spills_and_is_readable_by_artifact_selector",
        || async {
            let content = (0..2_000)
                .map(|line| format!("payload-{line:04}-{}\n", "x".repeat(32)))
                .collect::<String>();
            let h = RebornIntegrationHarness::test_default()
                .with_coding_tools()
                .script([
                    RebornScriptedReply::tool_call("read", json!({ "path": "large.txt" })),
                    RebornScriptedReply::tool_call(
                        "read",
                        json!({ "path": "artifact://0:bytes:0-3071" }),
                    ),
                    RebornScriptedReply::text("artifact recovered"),
                ])
                .build()
                .await
                .expect("harness builds");
            let path = h
                .capability_recorder
                .workspace_file_path("large.txt")
                .expect("host-runtime harness exposes the workspace root");
            std::fs::write(&path, content).expect("seed large workspace file");

            h.submit_turn("read the large file, then recover its first 3 KiB artifact range")
                .await
                .expect("turn completes");
            let artifact_read = h
                .tool_result_output("builtin.read")
                .await
                .expect("artifact read result");
            assert!(
                artifact_read.get("artifact_ref").is_none(),
                "an artifact continuation must remain inline instead of creating another artifact: {artifact_read}"
            );
            let recovered = output_text(&artifact_read);
            assert!(
                recovered.starts_with("[large.txt#") && recovered.contains("1:payload-0000-"),
                "artifact byte selector returns the start of the exact spilled output: {recovered}"
            );
            assert_eq!(recovered.len(), 3 * 1024);
        },
    );
}

/// The approval gate applies to the NEW capabilities: a scripted coding
/// `write` parks on a real `BlockedApproval` gate and only persists after
/// approval.
#[test]
fn coding_gated_write_requires_approval() {
    run_async_test_with_stack("coding_gated_write_requires_approval", || async {
        let group = RebornIntegrationGroup::coding_tools_with_approvals()
            .await
            .expect("coding approvals group builds");
        let h = group
            .thread("coding-gated-write")
            .script([
                RebornScriptedReply::tool_call(
                    "write",
                    json!({ "path": "/workspace/gated.txt", "content": "approved payload" }),
                ),
                RebornScriptedReply::text("file written"),
            ])
            .build()
            .await
            .expect("thread builds");

        let (run_id, gate_ref) = h
            .submit_turn_until_blocked("write the gated file")
            .await
            .expect("coding write raises a real approval gate");
        h.approve_gate(run_id, &gate_ref)
            .await
            .expect("gate approves");
        h.wait_for_status(run_id, ironclaw_turns::TurnStatus::Completed)
            .await
            .expect("run completes after resume");

        h.assert_workspace_file_contains("gated.txt", "approved payload")
            .await
            .expect("the approved coding write persisted to the workspace file");
    });
}

/// Runs the async test body on a dedicated 16 MiB-stack thread, mirroring
/// `tests/integration/process_port.rs`'s `run_with_larger_stack` and
/// `reborn_sandbox_shell_turn.rs`: the pinned-coding harness builds through
/// the production-shaped composition (`build_production_shaped`), whose
/// debug async-state-machine chain alone consumes >2 MiB of stack — over the
/// default 2 MiB libtest thread stack (see the module doc's stack note).
fn run_async_test_with_stack<F, Fut>(name: &'static str, test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + 'static,
{
    let handle = std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio test runtime")
                .block_on(test());
        })
        .expect("spawn stack-sized test thread");
    if let Err(panic) = handle.join() {
        std::panic::resume_unwind(panic);
    }
}
