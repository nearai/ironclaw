//! End-to-end lifecycle test for the skill `setup_marker` exclusion.
//!
//! Drives a real agent turn through the skill-selection pipeline to
//! verify that a one-time setup skill:
//!
//! 1. **Activates** on the first matching message (marker absent) —
//!    its distinctive prompt content appears in the LLM system prompt
//! 2. **Is excluded** on a second matching message after the marker
//!    file has been written to the workspace — its prompt content is
//!    absent from the LLM system prompt
//!
//! This is the integration-tier cover for the unit tests in
//! `crates/ironclaw_skills/src/selector.rs::tests::test_setup_marker_*`
//! and the v2 equivalent in
//! `crates/ironclaw_engine/src/executor/orchestrator.rs::handle_list_skills`.
//!
//! ## Why assert on the LLM system prompt content, not on `active_skill_names()`
//!
//! The v1 and v2 engine paths emit `StatusUpdate::SkillActivated`
//! events at different layers — v1 from `src/agent/agent_loop.rs`,
//! v2 from the Python orchestrator via `EventKind::SkillActivated`.
//! Testing through the status-event surface would couple the test to
//! whichever path the rig's default configuration selects.
//!
//! The **actual effect** of skill selection is that the skill's
//! prompt content gets injected into the LLM system prompt. That is
//! the contract the selector exists to enforce, and it's the same
//! contract on both paths. We assert on the captured LLM request
//! messages (via `rig.captured_llm_requests()`), which makes the test
//! agnostic to the internal plumbing: if the skill is selected, its
//! distinctive marker string appears in the system prompt; if it's
//! excluded, the string is absent.

#![cfg(feature = "libsql")]

mod support;

mod setup_marker_lifecycle {
    use crate::support::test_rig::TestRigBuilder;
    use std::time::Duration;
    use tempfile::TempDir;

    /// Distinctive string embedded in the test skill's body so we can
    /// grep for it in the captured LLM system prompt. Must not appear
    /// anywhere else in the codebase or the committed skills.
    const SKILL_MARKER_STRING: &str = "LIFECYCLE-TEST-SKILL-BODY-MARKER-Z7Q";

    fn write_lifecycle_skill(
        skills_dir: &std::path::Path,
        name: &str,
        keyword: &str,
        marker: &str,
    ) {
        let dir = skills_dir.join(name);
        std::fs::create_dir_all(&dir).expect("create skill dir");
        let content = format!(
            r#"---
name: {name}
version: 0.1.0
description: Lifecycle test skill — should only activate once.
activation:
  setup_marker: {marker}
  keywords:
    - {keyword}
  max_context_tokens: 500
---

# {name}

{SKILL_MARKER_STRING}

This is a lifecycle test skill. In a real setup skill this body would
contain the onboarding steps. Here it's intentionally minimal — we
just need the manifest to parse and load, and the marker string
above to be injectable into the LLM system prompt when the skill
is selected.
"#
        );
        std::fs::write(dir.join("SKILL.md"), content).expect("write SKILL.md");
    }

    async fn build_rig(skills_dir: &std::path::Path) -> crate::support::test_rig::TestRig {
        // NB: we deliberately do NOT enable engine_v2 here. The
        // integration test focuses on the v1 Rust selector path
        // (`select_active_skills` -> `prefilter_skills` with workspace
        // `exists()` check). The v2 path uses a parallel filter in
        // `handle_list_skills` that is covered by its own unit test
        // surface (MemoryDoc title match on the skill's metadata).
        TestRigBuilder::new()
            .with_skills_dir(skills_dir.to_path_buf())
            .build()
            .await
    }

    /// Count how many times the marker string appears across all
    /// captured LLM request messages (system + user + assistant).
    /// Each selected skill injects its body into the system prompt,
    /// so presence of the marker string means "the skill was
    /// selected for at least one turn".
    fn marker_occurrences(requests: &[Vec<ironclaw::llm::ChatMessage>]) -> usize {
        let mut count = 0;
        for request in requests {
            for msg in request {
                if msg.content.contains(SKILL_MARKER_STRING) {
                    count += 1;
                }
            }
        }
        count
    }

    #[tokio::test]
    async fn setup_marker_excludes_skill_after_workspace_marker_written() {
        let skills_root = TempDir::new().expect("create tempdir for skills");
        let skill_name = "lifecycle-setup-test";
        let skill_keyword = "xyzzy-lifecycle-onboard";
        let marker_path = "commitments/.lifecycle-setup-complete";

        write_lifecycle_skill(skills_root.path(), skill_name, skill_keyword, marker_path);

        let rig = build_rig(skills_root.path()).await;

        // Sanity: the skill was loaded from the tempdir skills root.
        let loaded = rig.loaded_skill_names();
        assert!(
            loaded.iter().any(|n| n == skill_name),
            "lifecycle skill must be loaded from the temp skills dir. Loaded: {loaded:?}"
        );

        // ── Phase 1: marker absent — skill should be selected ──────
        let message1 = format!("please handle {skill_keyword} for me");
        rig.send_message(&message1).await;
        let _ = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        let requests_after_turn1 = rig.captured_llm_requests();
        let phase1_count = marker_occurrences(&requests_after_turn1);
        assert!(
            phase1_count >= 1,
            "Phase 1: setup skill should be selected when marker is absent \
             (expected its body marker string in >= 1 LLM request, got {phase1_count}). \
             Captured {} LLM requests.",
            requests_after_turn1.len(),
        );

        // ── Phase 2: write the marker file via the workspace ───────
        let workspace = rig
            .workspace()
            .expect("rig must expose workspace for libsql backend")
            .clone();
        workspace
            .write(marker_path, "# lifecycle test marker\n")
            .await
            .expect("write marker file");
        let exists = workspace.exists(marker_path).await.expect("exists check");
        assert!(exists, "marker file must be readable after write");

        // ── Phase 3: same keyword, new message — skill MUST be excluded ──
        let message2 = format!("again, please handle {skill_keyword}");
        rig.send_message(&message2).await;
        let _ = rig.wait_for_responses(2, Duration::from_secs(15)).await;

        let requests_after_turn2 = rig.captured_llm_requests();
        let phase2_count = marker_occurrences(&requests_after_turn2);

        // The skill was included in turn 1's system prompt, so
        // `phase1_count` is the baseline. After turn 2 fires, the
        // count must NOT increase — the skill should NOT have been
        // injected into turn 2's prompt.
        assert_eq!(
            phase2_count, phase1_count,
            "Phase 3: setup skill must NOT be re-selected after marker exists. \
             Phase 1 count: {phase1_count}. Phase 2 count: {phase2_count}. \
             The skill's body marker string appeared in additional LLM requests \
             on turn 2, meaning the marker exclusion did not fire."
        );

        rig.shutdown();
    }
}
