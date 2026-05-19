//! Auto-split per-audience dispatch for the `traces` CLI surface.
//!
//! Audience: admin. See `super::run_traces` for the routing.

use super::*;

pub(super) async fn dispatch(cmd: TracesSubcommand) -> anyhow::Result<()> {
    match cmd {
        TracesSubcommand::MaintenanceRun {
            endpoint,
            purpose,
            dry_run,
            backfill_db_mirror,
            index_vectors,
            purge_expired_before,
            max_export_age_hours,
            skip_export_cache_prune,
            reconcile_db_mirror,
            verify_audit_chain,
            bearer_token_env,
            json,
        } => {
            let options = TraceCommonsMaintenanceOptions {
                purpose,
                dry_run,
                backfill_db_mirror,
                index_vectors,
                reconcile_db_mirror,
                verify_audit_chain,
                prune_export_cache: !skip_export_cache_prune,
                max_export_age_hours,
                purge_expired_before,
            };
            trace_commons_maintenance_run(&endpoint, &bearer_token_env, options, json).await
        }
        TracesSubcommand::RetentionJobsList {
            endpoint,
            limit,
            status,
            bearer_token_env,
            json,
        } => {
            trace_commons_retention_jobs_list(&endpoint, &bearer_token_env, limit, status, json)
                .await
        }
        TracesSubcommand::RetentionJobItems {
            endpoint,
            retention_job_id,
            limit,
            action,
            status,
            bearer_token_env,
            json,
        } => {
            trace_commons_retention_job_items(
                &endpoint,
                &bearer_token_env,
                retention_job_id,
                limit,
                action,
                status,
                json,
            )
            .await
        }
        TracesSubcommand::ExportAccessGrantsList {
            endpoint,
            limit,
            status,
            dataset_kind,
            bearer_token_env,
            json,
        } => {
            trace_commons_export_access_grants_list(
                &endpoint,
                &bearer_token_env,
                limit,
                status,
                dataset_kind,
                json,
            )
            .await
        }
        TracesSubcommand::ExportJobsList {
            endpoint,
            limit,
            status,
            dataset_kind,
            bearer_token_env,
            json,
        } => {
            trace_commons_export_jobs_list(
                &endpoint,
                &bearer_token_env,
                limit,
                status,
                dataset_kind,
                json,
            )
            .await
        }
        TracesSubcommand::BenchmarkConvert {
            endpoint,
            purpose,
            limit,
            consent_scope,
            status,
            privacy_risk,
            external_ref,
            bearer_token_env,
            json,
        } => {
            trace_commons_benchmark_convert(TraceCommonsBenchmarkConvertOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                purpose,
                limit,
                consent_scope,
                status,
                privacy_risk,
                external_ref,
                json,
                path: "/v1/benchmarks/convert",
            })
            .await
        }
        TracesSubcommand::BenchmarkLifecycleUpdate {
            endpoint,
            conversion_id,
            registry_status,
            registry_ref,
            published_at,
            evaluation_status,
            evaluator_ref,
            evaluated_at,
            score,
            pass_count,
            fail_count,
            reason,
            bearer_token_env,
            json,
        } => {
            trace_commons_benchmark_lifecycle_update(TraceCommonsBenchmarkLifecycleUpdateOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                conversion_id,
                registry_status,
                registry_ref,
                published_at,
                evaluation_status,
                evaluator_ref,
                evaluated_at,
                score,
                pass_count,
                fail_count,
                reason,
                json,
            })
            .await
        }
        TracesSubcommand::ReplayDatasetExport {
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
            trace_commons_replay_dataset_export(TraceCommonsReplayDatasetExportOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                purpose,
                consent_scope,
                status,
                privacy_risk,
                limit,
                output,
                json,
                method: Method::GET,
                path: "/v1/datasets/replay",
            })
            .await
        }
        TracesSubcommand::ReplayExportManifests {
            endpoint,
            bearer_token_env,
            json,
        } => trace_commons_replay_export_manifests(&endpoint, &bearer_token_env, json).await,
        TracesSubcommand::AnalyticsSummary {
            endpoint,
            bearer_token_env,
            json,
        } => trace_commons_analytics_summary(&endpoint, &bearer_token_env, json).await,
        TracesSubcommand::OperationalSummary {
            endpoint,
            bearer_token_env,
            json,
        } => trace_commons_operational_summary(&endpoint, &bearer_token_env, json).await,
        TracesSubcommand::ConfigStatus {
            endpoint,
            bearer_token_env,
        } => trace_commons_config_status(&endpoint, &bearer_token_env).await,
        _ => unreachable!("router ensures only audience variants reach this dispatch"),
    }
}
