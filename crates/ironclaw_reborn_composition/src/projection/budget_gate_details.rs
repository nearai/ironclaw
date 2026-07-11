//! Budget approval-gate detail rendering for the WebUI turn-event projection.
//!
//! Renders the human-readable detail rows shown on a budget approval gate
//! (current usage, limits, the post-approval preview, etc.). The preview of the
//! raised limit routes through [`ironclaw_resources::raised_budget_limit`] — the
//! same policy the apply path uses — so "Limit after approval" can never drift
//! from what approval actually sets.

use ironclaw_product_adapters::ApprovalPromptDetailView;
use ironclaw_resources::{
    BudgetGateId, BudgetGateStore, ResourceApprovalNeeded, ResourceDimension, ResourceGovernor,
    ResourceLimits, ResourceValue,
};
use ironclaw_turns::TurnScope;

pub(super) fn budget_gate_details(
    budget_gates: Option<&dyn BudgetGateStore>,
    budget_governor: Option<&dyn ResourceGovernor>,
    scope: &TurnScope,
    gate_ref: &str,
) -> Vec<ApprovalPromptDetailView> {
    let Some(store) = budget_gates else {
        return Vec::new();
    };
    let Some(gate_id) = BudgetGateId::from_gate_ref(gate_ref) else {
        return Vec::new();
    };
    let gate = match store.get(&scope.to_resource_scope(), gate_id) {
        Ok(Some(gate)) => gate,
        Ok(None) => return Vec::new(),
        Err(error) => {
            tracing::debug!(
                %error,
                %gate_ref,
                "budget gate detail lookup failed during WebUI projection"
            );
            return Vec::new();
        }
    };
    let needed = gate.needed;
    let mut details = Vec::new();
    push_detail(
        &mut details,
        "Budget scope",
        budget_account_label(&needed.account),
    );
    push_detail(
        &mut details,
        "Current usage",
        needed.current_usage.display(needed.dimension),
    );
    if !needed.active_reserved.is_zero() {
        push_detail(
            &mut details,
            "Already reserved",
            needed.active_reserved.display(needed.dimension),
        );
    }
    push_detail(
        &mut details,
        "Current limit",
        needed.limit.display(needed.dimension),
    );
    if let Some((approved_limit, limit_increase)) =
        budget_approval_limit_change(budget_governor, &needed)
    {
        push_detail(
            &mut details,
            "Limit after approval",
            approved_limit.display(needed.dimension),
        );
        push_detail(
            &mut details,
            "Limit increase",
            limit_increase.display(needed.dimension),
        );
    }
    push_detail(
        &mut details,
        "Estimated for this step",
        needed.requested.display(needed.dimension),
    );
    push_detail(
        &mut details,
        "Usage after this estimate",
        ResourceValue::checked_sum3(
            &needed.current_usage,
            &needed.active_reserved,
            &needed.requested,
        )
        .map(|total| total.display(needed.dimension))
        .unwrap_or_else(|| "unknown".to_string()),
    );
    push_detail(
        &mut details,
        "Approval means",
        budget_approval_effect_label(needed.dimension, &needed.requested),
    );
    if let Some(period_end) = needed.period_end {
        push_detail(
            &mut details,
            "Budget window resets",
            period_end.to_rfc3339(),
        );
    }
    details
}

fn push_detail(details: &mut Vec<ApprovalPromptDetailView>, label: &str, value: String) {
    if let Ok(detail) = ApprovalPromptDetailView::new(label, value) {
        details.push(detail);
    }
}

fn budget_account_label(account: &ironclaw_resources::ResourceAccount) -> String {
    match account {
        ironclaw_resources::ResourceAccount::Tenant { .. } => "tenant budget".to_string(),
        ironclaw_resources::ResourceAccount::User { .. } => "your user budget".to_string(),
        ironclaw_resources::ResourceAccount::Project { .. } => "project budget".to_string(),
        ironclaw_resources::ResourceAccount::Agent { .. } => "agent budget".to_string(),
        ironclaw_resources::ResourceAccount::Mission { .. } => "mission budget".to_string(),
        ironclaw_resources::ResourceAccount::Thread { .. } => "thread budget".to_string(),
    }
}

fn budget_approval_limit_change(
    governor: Option<&dyn ResourceGovernor>,
    needed: &ResourceApprovalNeeded,
) -> Option<(ResourceValue, ResourceValue)> {
    let governor = governor?;
    let limits = governor
        .account_snapshot(&needed.account)
        .ok()
        .flatten()
        .and_then(|snapshot| snapshot.limits)
        .unwrap_or_default();
    let current_limit = dimension_limit(&limits, needed.dimension).unwrap_or(needed.limit.clone());
    let total = ResourceValue::checked_sum3(
        &needed.current_usage,
        &needed.active_reserved,
        &needed.requested,
    )?;
    // Preview the post-approval limit through the SAME policy the apply path
    // uses (ironclaw_resources::raised_budget_limit), so "Limit after approval"
    // can never drift from what approval actually sets.
    let target = ironclaw_resources::raised_budget_limit(
        Some(current_limit.clone()),
        total,
        limits.thresholds.pause_at,
    )?;
    let approved_limit = current_limit.max_value(&target)?;
    let limit_increase = approved_limit.saturating_sub(&current_limit)?;
    Some((approved_limit, limit_increase))
}

fn dimension_limit(limits: &ResourceLimits, dimension: ResourceDimension) -> Option<ResourceValue> {
    match dimension {
        ResourceDimension::Usd => limits.max_usd.map(ResourceValue::Decimal),
        ResourceDimension::InputTokens => limits.max_input_tokens.map(ResourceValue::Integer),
        ResourceDimension::OutputTokens => limits.max_output_tokens.map(ResourceValue::Integer),
        ResourceDimension::WallClockMs => limits.max_wall_clock_ms.map(ResourceValue::Integer),
        ResourceDimension::OutputBytes => limits.max_output_bytes.map(ResourceValue::Integer),
        ResourceDimension::NetworkEgressBytes => {
            limits.max_network_egress_bytes.map(ResourceValue::Integer)
        }
        ResourceDimension::ProcessCount => limits
            .max_process_count
            .map(u64::from)
            .map(ResourceValue::Integer),
        ResourceDimension::ConcurrencySlots => limits
            .max_concurrency_slots
            .map(u64::from)
            .map(ResourceValue::Integer),
    }
}

fn budget_approval_effect_label(dimension: ResourceDimension, requested: &ResourceValue) -> String {
    format!(
        "raise this account limit enough for this estimated {} step and resume the run",
        requested.display(dimension)
    )
}
