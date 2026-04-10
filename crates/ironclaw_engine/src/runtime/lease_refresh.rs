use std::collections::HashSet;
use std::sync::Arc;

use crate::Capability;
use crate::capability::lease::LeaseManager;
use crate::capability::planner::LeasePlanner;
use crate::capability::registry::CapabilityRegistry;
use crate::traits::effect::EffectExecutor;
use crate::traits::store::Store;
use crate::types::capability::GrantedActions;
use crate::types::error::EngineError;
use crate::types::thread::Thread;

pub(crate) async fn reconcile_dynamic_tool_lease(
    thread: &mut Thread,
    effects: &Arc<dyn EffectExecutor>,
    leases: &Arc<LeaseManager>,
    store: Option<&Arc<dyn Store>>,
    lease_planner: &LeasePlanner,
) -> Result<(), EngineError> {
    let active_leases = leases.active_for_thread(thread.id).await;
    let actions = effects.available_actions(&active_leases).await?;
    if actions.is_empty() {
        return Ok(());
    }

    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(Capability {
        name: "tools".into(),
        description: "Available tools".into(),
        actions,
        knowledge: vec![],
        policies: vec![],
    });

    let Some(grant) = lease_planner
        .plan_for_thread(thread.thread_type, &capabilities)
        .into_iter()
        .find(|grant| grant.capability_name == "tools")
    else {
        return Ok(());
    };

    let desired_actions: HashSet<String> = match grant.granted_actions {
        GrantedActions::All => return Ok(()),
        GrantedActions::Specific(actions) => actions.into_iter().collect(),
    };

    if desired_actions.is_empty() {
        return Ok(());
    }

    if let Some(existing) = active_leases
        .iter()
        .find(|lease| lease.capability_name == "tools")
    {
        if existing.granted_actions.is_all() {
            return Ok(());
        }

        let mut merged: HashSet<String> =
            existing.granted_actions.actions().iter().cloned().collect();
        let before = merged.len();
        merged.extend(desired_actions);
        if merged.len() == before {
            return Ok(());
        }

        let mut merged_actions: Vec<String> = merged.into_iter().collect();
        merged_actions.sort();
        let updated = leases
            .update_granted_actions(existing.id, GrantedActions::Specific(merged_actions))
            .await?;
        if let Some(store) = store {
            store.save_lease(&updated).await?;
        }
        return Ok(());
    }

    let mut actions: Vec<String> = desired_actions.into_iter().collect();
    actions.sort();
    let lease = leases
        .grant(
            thread.id,
            "tools",
            GrantedActions::Specific(actions),
            None,
            None,
        )
        .await?;
    if let Some(store) = store {
        store.save_lease(&lease).await?;
    }
    if !thread.capability_leases.contains(&lease.id) {
        thread.capability_leases.push(lease.id);
    }

    Ok(())
}
