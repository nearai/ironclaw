//! Harness-port-seam P1 Change 4: RED-first integration-tier pin for
//! `wrap_local_dev_surface_disclosure` — one of the production port layers
//! the OLD harness (`apply_synthetic_capability_wrappers` hand-rebuilding
//! three of the seven production wrap layers) never applied at all, so it was
//! invisible to every integration test before this seam PR.
//!
//! Ground truth (verified against
//! `crates/ironclaw_reborn_composition/src/runtime/local_dev/surface_disclosure.rs`,
//! NOT the plan doc's "must deny rather than execute" framing, which does not
//! match the code): `wrap_local_dev_surface_disclosure` never hides or denies
//! a capability. It is a description/schema ANNOTATION layer, disabled unless
//! the workspace mount view carries a confirmed `/host` alias
//! (`LocalDevSurfaceDisclosure::enabled`). When enabled, it appends a
//! "confirmed scoped roots" note to the `description`/`parameters` of the
//! local-dev scoped-path capabilities (`read_file`, `write_file`, `list_dir`,
//! `glob`, `grep`, `apply_patch`) and a local-host-shell note to
//! `builtin.shell` — so the model is told which host paths are genuinely
//! mounted instead of guessing raw host paths. This test pins THAT behavior,
//! end to end, through the TraceLlm seam (`RebornScriptedReply`).
//!
//! Every harness before this seam PR built its workspace mounts via
//! `workspace_mounts()` (`/workspace` alias only, never `/host`), so no
//! integration test could previously observe this layer even in principle —
//! confirming the plan's "previously-absent layer" framing, independent of
//! its (wrong) denial description.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

/// Flat wire name for `builtin.read_file` (`.` -> `__`, `reply.rs`'s
/// `RebornScriptedReply::tool_call` encoding).
const FLAT_READ_FILE_TOOL_NAME: &str = "builtin__read_file";

/// Flat wire name for `builtin.shell` (same `.` -> `__` encoding).
const FLAT_SHELL_TOOL_NAME: &str = "builtin__shell";

/// Substring of `LocalDevSurfaceDisclosure`'s `confirmed_host_roots_note`
/// output stable across `local_dev_mounts.rs` mount-alias wording changes.
const SCOPED_ROOTS_NOTE_NEEDLE: &str = "Available scoped roots";

/// Substring of `LOCAL_DEV_LOCAL_HOST_SHELL_NOTE`, the fixed local-host-shell
/// annotation `apply_to_surface_fields` appends unconditionally to
/// `builtin.shell`'s description once the layer is enabled at all (unlike the
/// scoped-path capabilities, `builtin.shell` gets no `scoped_roots_note`
/// gate of its own — its branch returns immediately after appending).
const SHELL_LOCAL_HOST_NOTE_NEEDLE: &str = "Runs on the local host with local-dev shell";

/// A harness with a confirmed `/host` mount (Change 4's new
/// `.with_confirmed_host_mount()` backend) must surface the scoped-roots note
/// on `read_file`'s captured tool definition. Before the harness-port-seam
/// switch, `create_recording_capability_port` never called
/// `wrap_local_dev_surface_disclosure` at all, so this assertion fails for
/// the right reason on the OLD harness (RED) regardless of the mount grant —
/// the switch (not just the new mount backend) is what turns it GREEN.
#[tokio::test]
async fn confirmed_host_mount_adds_scoped_roots_note_to_read_file() {
    let h = RebornIntegrationHarness::test_default()
        .with_confirmed_host_mount()
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("confirmed-host-mount harness builds");

    h.submit_turn("hello").await.expect("turn completes");

    h.assert_model_tool_description_contains(FLAT_READ_FILE_TOOL_NAME, SCOPED_ROOTS_NOTE_NEEDLE)
        .await
        .expect("confirmed /host mount must surface the scoped-roots disclosure note");
}

/// Negative control: the plain `BuiltinHttpTools` backend's workspace mounts
/// carry only `/workspace` (no confirmed `/host` alias), so
/// `LocalDevSurfaceDisclosure::enabled()` is false and the note never
/// appears — proves the positive assertion above discriminates on the mount
/// grant, not on `read_file` always carrying the note.
#[tokio::test]
async fn workspace_only_mount_excludes_scoped_roots_note() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("workspace-only harness builds");

    h.submit_turn("hello").await.expect("turn completes");

    h.assert_model_tool_description_excludes(FLAT_READ_FILE_TOOL_NAME, SCOPED_ROOTS_NOTE_NEEDLE)
        .await
        .expect("without a confirmed host mount the disclosure note must not appear");
}

/// `builtin.shell` takes a DIFFERENT branch in `apply_to_surface_fields`
/// (`capability_id.as_str() == SHELL_CAPABILITY_ID`, checked before the
/// scoped-path capability match): it appends `LOCAL_DEV_LOCAL_HOST_SHELL_NOTE`
/// unconditionally rather than gating on `scoped_roots_note`. But the whole
/// port is still gated on `LocalDevSurfaceDisclosure::enabled()`
/// (`scoped_roots_note.is_some()`, i.e. a confirmed `/host` mount) in
/// `wrap_local_dev_surface_disclosure` — without a confirmed host mount the
/// wrapper is skipped entirely and `builtin.shell` never gets annotated. This
/// pins the enabled case; the negative control below pins the disabled case.
#[tokio::test]
async fn confirmed_host_mount_adds_local_host_shell_note_to_shell() {
    let h = RebornIntegrationHarness::test_default()
        .with_confirmed_host_mount()
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("confirmed-host-mount harness builds");

    h.submit_turn("hello").await.expect("turn completes");

    h.assert_model_tool_description_contains(FLAT_SHELL_TOOL_NAME, SHELL_LOCAL_HOST_NOTE_NEEDLE)
        .await
        .expect("confirmed /host mount must surface the local-host shell note");
}

/// Negative control: without a confirmed `/host` mount `enabled()` is false,
/// so `wrap_local_dev_surface_disclosure` returns the inner port unwrapped
/// and `builtin.shell`'s description is never touched.
#[tokio::test]
async fn workspace_only_mount_excludes_local_host_shell_note() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("workspace-only harness builds");

    h.submit_turn("hello").await.expect("turn completes");

    h.assert_model_tool_description_excludes(FLAT_SHELL_TOOL_NAME, SHELL_LOCAL_HOST_NOTE_NEEDLE)
        .await
        .expect("without a confirmed host mount the local-host shell note must not appear");
}

/// Change 4's second pin: the input-ref/result-ref round trip crosses ONE
/// staged store. A full `builtin.time` dispatch (register -> invoke ->
/// completed) only succeeds if the SAME shared `LocalDevCapabilityIo`
/// resolves the input ref it staged and accepts the result write under a
/// correlated ref — the invariant `RefreshingLocalDevCapabilityPortTestParts`
/// documents ("input_resolver AND result_writer must be two `Arc::clone`s of
/// the SAME shared io object"). A harness wiring two independently-sourced io
/// objects would fail this dispatch outright (unresolvable input ref), not
/// merely diverge on assertions.
#[tokio::test]
async fn shared_capability_io_round_trips_input_and_result_refs() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call("builtin.time", json!({})),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("builtin-http-tools harness builds");

    h.submit_turn("what time is it")
        .await
        .expect("turn completes");

    h.assert_tool_invoked("builtin.time").await.expect(
        "builtin.time must dispatch to completion through the shared \
             input_resolver/result_writer io",
    );
}
