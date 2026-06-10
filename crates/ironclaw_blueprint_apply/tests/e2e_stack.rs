//! End-to-end stack test (epic #3036): blueprint parse → lockfile → apply
//! engine idempotence → harness parse, all driven together. This is the
//! single "does the whole thing hang together" test for the integration
//! branch.

use std::cell::RefCell;
use std::collections::BTreeMap;

use ironclaw_blueprint::{Blueprint, parse};
use ironclaw_blueprint_apply::{
    Actor, ApplyMode, ApplyScope, BlueprintApplyService, Change, ChangeAction, Domain,
    DomainReconciler, ReconcileError, structural_hash,
};

const BLUEPRINT: &str = r#"
api_version = "ironclaw.config/v1"
kind = "Blueprint"

[scope]
user = "self"

[system_prompt]
text_ref = "files/system_prompt.md"

[providers]
default_llm = "anthropic"
[providers.anthropic]
model = "claude-opus-4-7"
api_key = "${secret:anthropic_api_key}"

[[extensions]]
id = "github-mcp"

[[missions]]
id = "weekly-sweep"
brief_ref = "files/sweep.md"

[harness]
id = "chain-incident-response"
"#;

const HARNESS: &str = r#"
api_version = "ironclaw.harness/v1"
kind = "Harness"
id = "chain-incident-response"
name = "Chain Incident Response"
trust = "user_trusted"

[prompt_overlay]
text_ref = "prompts/system.md"

[runtime_constraints]
max_profile = "Sandboxed"

[[required_extensions]]
id = "ethereum-rpc"

[capability_surface]
allow = ["ethereum-rpc.*", "memory.write"]
deny = ["shell.run"]
"#;

/// Minimal settings reconciler: maps key -> structural hash.
#[derive(Default)]
struct MemRepo {
    store: RefCell<BTreeMap<String, String>>,
}

impl DomainReconciler for MemRepo {
    fn domain(&self) -> Domain {
        Domain::SystemPrompt
    }

    fn plan(&self, bp: &Blueprint, _: &ApplyScope) -> Result<Vec<Change>, ReconcileError> {
        let mut changes = Vec::new();
        if let Some(prompt) = &bp.system_prompt {
            let after = structural_hash(prompt)
                .map_err(|e| ReconcileError::new(Domain::SystemPrompt, e.to_string()))?;
            let before = self.store.borrow().get("system_prompt").cloned();
            let action = match &before {
                None => ChangeAction::Create,
                Some(cur) if *cur == after => ChangeAction::NoOp,
                Some(_) => ChangeAction::Update,
            };
            changes.push(Change {
                domain: Domain::SystemPrompt,
                key: "system_prompt".to_string(),
                action,
                before_hash: before,
                after_hash: Some(after),
            });
        }
        Ok(changes)
    }

    fn apply(
        &self,
        _: &Blueprint,
        _: &ApplyScope,
        changes: &[Change],
    ) -> Result<(), ReconcileError> {
        for change in changes {
            if change.action.is_write()
                && let Some(after) = &change.after_hash
            {
                self.store
                    .borrow_mut()
                    .insert(change.key.clone(), after.clone());
            }
        }
        Ok(())
    }
}

#[test]
fn full_stack_blueprint_lockfile_apply_and_harness() {
    // 1. Parse the blueprint (secret handle accepted, no inline material).
    let blueprint = parse(BLUEPRINT).expect("blueprint parses");
    assert_eq!(blueprint.extensions.len(), 1);
    assert_eq!(
        blueprint.harness.as_ref().and_then(|h| h.id.as_deref()),
        Some("chain-incident-response")
    );

    // 2. Resolve the lockfile against real files in a tempdir.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("files")).expect("mkdir");
    std::fs::write(
        dir.path().join("files/system_prompt.md"),
        b"You are Acme.\n",
    )
    .expect("write");
    std::fs::write(dir.path().join("files/sweep.md"), b"Sweep weekly.\n").expect("write");
    let lock = blueprint.resolve_lockfile(dir.path()).expect("lockfile");
    assert_eq!(lock.files.len(), 2);

    // 3. Apply through the service: first apply writes, second is a no-op.
    let service = BlueprintApplyService::new(vec![Box::new(MemRepo::default())]);
    let actor = Actor::user("self");
    let first = service
        .apply(&blueprint, &actor, ApplyMode::Apply)
        .expect("apply");
    assert_eq!(
        first.write_count(),
        1,
        "first apply creates the system prompt"
    );
    let second = service
        .apply(&blueprint, &actor, ApplyMode::Apply)
        .expect("re-apply");
    assert!(second.is_noop(), "second apply is idempotent");

    // 4. Parse the harness the blueprint references.
    let harness = ironclaw_harness::parse(HARNESS).expect("harness parses");
    assert_eq!(harness.id, "chain-incident-response");
    assert_eq!(harness.required_extensions.len(), 1);
    assert_eq!(
        harness
            .runtime_constraints
            .and_then(|c| c.max_profile)
            .as_deref(),
        Some("Sandboxed")
    );
}
