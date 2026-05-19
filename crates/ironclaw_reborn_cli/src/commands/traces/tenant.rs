//! Auto-split per-audience dispatch for the `traces` CLI surface.
//!
//! Audience: tenant. See `super::run_traces` for the routing.

use super::*;

pub(super) async fn dispatch(cmd: TracesSubcommand) -> anyhow::Result<()> {
    match cmd {
        TracesSubcommand::TenantPolicyGet {
            endpoint,
            bearer_token_env,
            json,
        } => trace_commons_tenant_policy_get(&endpoint, &bearer_token_env, json).await,
        TracesSubcommand::TenantPolicySet {
            endpoint,
            policy_version,
            allowed_consent_scopes,
            allowed_uses,
            bearer_token_env,
            json,
        } => {
            trace_commons_tenant_policy_set(
                &endpoint,
                &bearer_token_env,
                policy_version,
                allowed_consent_scopes,
                allowed_uses,
                json,
            )
            .await
        }
        TracesSubcommand::TenantAccessGrantsList {
            endpoint,
            limit,
            status,
            role,
            principal_ref,
            bearer_token_env,
            json,
        } => {
            trace_commons_tenant_access_grants_list(
                &endpoint,
                &bearer_token_env,
                limit,
                status,
                role,
                principal_ref,
                json,
            )
            .await
        }
        TracesSubcommand::TenantPrincipalRef {
            token_env,
            signed_tenant_id,
            signed_actor_ref,
            json,
        } => trace_commons_tenant_principal_ref(
            token_env.as_deref(),
            signed_tenant_id.as_deref(),
            signed_actor_ref.as_deref(),
            json,
        ),
        TracesSubcommand::TenantAccessGrantCreate {
            endpoint,
            principal_ref,
            role,
            grant_id,
            allowed_consent_scopes,
            allowed_uses,
            issuer,
            audience,
            subject,
            issued_at,
            expires_at,
            reason,
            metadata,
            bearer_token_env,
            json,
        } => {
            trace_commons_tenant_access_grant_create(TraceCommonsTenantAccessGrantCreateOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                principal_ref,
                role,
                grant_id,
                allowed_consent_scopes,
                allowed_uses,
                issuer,
                audience,
                subject,
                issued_at,
                expires_at,
                reason,
                metadata,
                json,
            })
            .await
        }
        TracesSubcommand::TenantAccessGrantRevoke {
            endpoint,
            grant_id,
            reason,
            bearer_token_env,
            json,
        } => {
            trace_commons_tenant_access_grant_revoke(
                &endpoint,
                &bearer_token_env,
                grant_id,
                reason,
                json,
            )
            .await
        }
        TracesSubcommand::RankerTrainingCandidates {
            endpoint,
            purpose,
            consent_scope,
            status,
            privacy_risk,
            limit,
            output,
            bearer_token_env,
            json,
        } => {
            trace_commons_ranker_training_export(TraceCommonsRankerTrainingExportOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                purpose,
                consent_scope,
                status,
                privacy_risk,
                limit,
                output,
                json,
                path: "/v1/ranker/training-candidates",
                output_label: "ranker training candidates",
                item_field: "candidates",
            })
            .await
        }
        TracesSubcommand::RankerTrainingPairs {
            endpoint,
            purpose,
            consent_scope,
            status,
            privacy_risk,
            limit,
            output,
            bearer_token_env,
            json,
        } => {
            trace_commons_ranker_training_export(TraceCommonsRankerTrainingExportOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                purpose,
                consent_scope,
                status,
                privacy_risk,
                limit,
                output,
                json,
                path: "/v1/ranker/training-pairs",
                output_label: "ranker training pairs",
                item_field: "pairs",
            })
            .await
        }
        TracesSubcommand::AuditEvents {
            endpoint,
            limit,
            bearer_token_env,
            json,
        } => trace_commons_audit_events(&endpoint, &bearer_token_env, limit, json).await,
        TracesSubcommand::ListTraces {
            endpoint,
            purpose,
            consent_scope,
            status,
            limit,
            coverage_tag,
            tool,
            privacy_risk,
            bearer_token_env,
            json,
        } => {
            trace_commons_list_traces(TraceCommonsListTracesOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                purpose,
                consent_scope,
                status,
                limit,
                coverage_tag,
                tool,
                privacy_risk,
                json,
            })
            .await
        }
        TracesSubcommand::PrivacyFilterCanary {
            text,
            timeout_seconds,
            json,
        } => privacy_filter_canary(&text, timeout_seconds, json).await,
        _ => unreachable!("router ensures only audience variants reach this dispatch"),
    }
}
