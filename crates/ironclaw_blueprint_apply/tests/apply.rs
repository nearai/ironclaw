//! Caller-level apply tests driven through `BlueprintApplyService` with an
//! in-memory reconciler standing in for a typed repo. Exercises the slice-2
//! acceptance criteria: dry-run vs apply, idempotence, structural diff,
//! non-destructive drift, and admin-scope gating.

use std::cell::RefCell;
use std::collections::BTreeMap;

use ironclaw_blueprint::{Blueprint, parse};
use ironclaw_blueprint_apply::{
    Actor, ApplyError, ApplyMode, ApplyScope, BlueprintApplyService, Change, ChangeAction, Domain,
    DomainReconciler, ReconcileError, structural_hash,
};

/// In-memory stand-in for the system-prompt settings repo: maps key -> hash.
#[derive(Default)]
struct InMemorySystemPrompt {
    store: RefCell<BTreeMap<String, String>>,
}

impl InMemorySystemPrompt {
    fn with_seed(seed: &[(&str, &str)]) -> Self {
        let store = seed
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Self {
            store: RefCell::new(store),
        }
    }

    fn desired(&self, blueprint: &Blueprint) -> Result<BTreeMap<String, String>, ReconcileError> {
        let mut desired = BTreeMap::new();
        if let Some(prompt) = &blueprint.system_prompt {
            let hash = structural_hash(prompt)
                .map_err(|e| ReconcileError::new(Domain::SystemPrompt, e.to_string()))?;
            desired.insert("system_prompt".to_string(), hash);
        }
        Ok(desired)
    }
}

impl DomainReconciler for InMemorySystemPrompt {
    fn domain(&self) -> Domain {
        Domain::SystemPrompt
    }

    fn plan(
        &self,
        blueprint: &Blueprint,
        _scope: &ApplyScope,
    ) -> Result<Vec<Change>, ReconcileError> {
        let desired = self.desired(blueprint)?;
        let store = self.store.borrow();
        let mut changes = Vec::new();

        for (key, after) in &desired {
            let before = store.get(key).cloned();
            let action = match &before {
                None => ChangeAction::Create,
                Some(current) if current == after => ChangeAction::NoOp,
                Some(_) => ChangeAction::Update,
            };
            changes.push(Change {
                domain: Domain::SystemPrompt,
                key: key.clone(),
                action,
                before_hash: before,
                after_hash: Some(after.clone()),
            });
        }
        // Drift: keys in the repo but absent from the blueprint.
        for (key, current) in store.iter() {
            if !desired.contains_key(key) {
                changes.push(Change {
                    domain: Domain::SystemPrompt,
                    key: key.clone(),
                    action: ChangeAction::DeleteDeferred,
                    before_hash: Some(current.clone()),
                    after_hash: None,
                });
            }
        }
        Ok(changes)
    }

    fn apply(
        &self,
        _blueprint: &Blueprint,
        _scope: &ApplyScope,
        changes: &[Change],
    ) -> Result<(), ReconcileError> {
        let mut store = self.store.borrow_mut();
        for change in changes {
            match change.action {
                ChangeAction::Create | ChangeAction::Update => {
                    let after = change.after_hash.clone().ok_or_else(|| {
                        ReconcileError::new(Domain::SystemPrompt, "write change missing after_hash")
                    })?;
                    store.insert(change.key.clone(), after);
                }
                // Never delete on drift; never re-write a no-op.
                ChangeAction::NoOp | ChangeAction::DeleteDeferred => {}
            }
        }
        Ok(())
    }
}

const PROMPT_BP: &str = r#"
api_version = "ironclaw.config/v1"
kind = "Blueprint"

[scope]
user = "self"

[system_prompt]
text_ref = "files/system_prompt.md"
"#;

fn service_with(seed: &[(&str, &str)]) -> BlueprintApplyService {
    BlueprintApplyService::new(vec![Box::new(InMemorySystemPrompt::with_seed(seed))])
}

#[test]
fn dry_run_does_not_write_then_apply_creates() {
    let bp = parse(PROMPT_BP).expect("parses");
    let actor = Actor::user("self");

    // The reconciler instance must persist across calls to observe writes, so
    // build the service once and reuse it.
    let reconciler = InMemorySystemPrompt::default();
    let service = BlueprintApplyService::new(vec![Box::new(reconciler)]);

    let dry = service
        .apply(&bp, &actor, ApplyMode::DryRun)
        .expect("dry run");
    assert_eq!(dry.write_count(), 1);
    assert_eq!(dry.changes[0].action, ChangeAction::Create);

    let applied = service.apply(&bp, &actor, ApplyMode::Apply).expect("apply");
    assert_eq!(applied.write_count(), 1);

    // Second apply is a no-op — idempotent.
    let again = service
        .apply(&bp, &actor, ApplyMode::Apply)
        .expect("re-apply");
    assert!(again.is_noop(), "second apply must write nothing");
    assert_eq!(again.changes[0].action, ChangeAction::NoOp);
}

#[test]
fn structural_diff_ignores_source_formatting() {
    // Same AST, different source whitespace/quoting must hash identically.
    let a = parse(PROMPT_BP).expect("parses");
    let reformatted = r#"
api_version   =   "ironclaw.config/v1"
kind = 'Blueprint'
[scope]
user = "self"
[system_prompt]
text_ref = "files/system_prompt.md"
"#;
    let b = parse(reformatted).expect("parses");

    let service = service_with(&[]);
    service
        .apply(&a, &Actor::user("self"), ApplyMode::Apply)
        .expect("apply a");
    let second = service
        .apply(&b, &Actor::user("self"), ApplyMode::Apply)
        .expect("apply b");
    assert!(
        second.is_noop(),
        "reformatted-but-equal blueprint must not produce an Update"
    );
}

#[test]
fn drift_is_reported_not_deleted() {
    // Repo has an extra key the blueprint does not mention.
    let bp = parse(PROMPT_BP).expect("parses");
    let reconciler = InMemorySystemPrompt::with_seed(&[("orphan_setting", "abc123")]);
    let service = BlueprintApplyService::new(vec![Box::new(reconciler)]);

    let report = service
        .apply(&bp, &Actor::user("self"), ApplyMode::Apply)
        .expect("apply");
    let drift: Vec<_> = report.drift().collect();
    assert_eq!(drift.len(), 1);
    assert_eq!(drift[0].key, "orphan_setting");
    assert_eq!(drift[0].action, ChangeAction::DeleteDeferred);

    // Re-applying still reports the orphan — it was never deleted.
    let again = service
        .apply(&bp, &Actor::user("self"), ApplyMode::Apply)
        .expect("re-apply");
    assert_eq!(again.drift().count(), 1, "drift must not be auto-removed");
}

#[test]
fn non_admin_cannot_apply_tenant_scope() {
    let tenant_bp = r#"
api_version = "ironclaw.config/v1"
kind = "Blueprint"
[scope]
tenant = "acme"
[system_prompt]
text_ref = "files/system_prompt.md"
"#;
    let bp = parse(tenant_bp).expect("parses");
    let service = service_with(&[]);

    let err = service
        .apply(&bp, &Actor::user("self"), ApplyMode::DryRun)
        .expect_err("non-admin tenant scope must fail closed");
    assert!(matches!(err, ApplyError::Authority(_)));

    // Admin may apply the same blueprint.
    service
        .apply(&bp, &Actor::admin("root"), ApplyMode::DryRun)
        .expect("admin allowed");
}
