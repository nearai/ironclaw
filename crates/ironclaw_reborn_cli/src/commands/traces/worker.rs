//! Auto-split per-audience dispatch for the `traces` CLI surface.
//!
//! Audience: worker. See `super::run_traces` for the routing.

use super::*;

pub(super) async fn dispatch(cmd: TracesSubcommand) -> anyhow::Result<()> {
    match cmd {
        TracesSubcommand::WorkerUtilityCredit {
            endpoint,
            event_type,
            credit_points_delta,
            reason,
            external_ref,
            submission_ids,
            bearer_token_env,
            json,
        } => {
            trace_commons_worker_utility_credit(TraceCommonsWorkerUtilityCreditOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                event_type,
                credit_points_delta,
                reason,
                external_ref,
                submission_ids,
                json,
            })
            .await
        }
        TracesSubcommand::WorkerRetentionMaintenance {
            endpoint,
            purpose,
            dry_run,
            purge_expired_before,
            max_export_age_hours,
            skip_export_cache_prune,
            bearer_token_env,
            json,
        } => {
            trace_commons_retention_maintenance_run(TraceCommonsRetentionMaintenanceOptions {
                endpoint: &endpoint,
                bearer_token_env: &bearer_token_env,
                purpose,
                dry_run,
                prune_export_cache: !skip_export_cache_prune,
                max_export_age_hours,
                purge_expired_before,
                json,
            })
            .await
        }
        TracesSubcommand::WorkerVectorIndex {
            endpoint,
            purpose,
            dry_run,
            bearer_token_env,
            json,
        } => {
            trace_commons_vector_index_run(&endpoint, &bearer_token_env, purpose, dry_run, json)
                .await
        }
        TracesSubcommand::WorkerBenchmarkConvert {
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
                path: "/v1/workers/benchmark-convert",
            })
            .await
        }
        TracesSubcommand::WorkerReplayDatasetExport {
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
                path: "/v1/workers/replay-export",
            })
            .await
        }
        TracesSubcommand::WorkerRankerTrainingCandidates {
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
                path: "/v1/workers/ranker/training-candidates",
                output_label: "ranker training candidates",
                item_field: "candidates",
            })
            .await
        }
        TracesSubcommand::WorkerRankerTrainingPairs {
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
                path: "/v1/workers/ranker/training-pairs",
                output_label: "ranker training pairs",
                item_field: "pairs",
            })
            .await
        }
        TracesSubcommand::ProcessEvaluationSubmit(args) => {
            trace_commons_process_evaluation_submit(TraceCommonsProcessEvaluationSubmitOptions {
                endpoint: &args.endpoint,
                bearer_token_env: &args.bearer_token_env,
                submission_id: args.submission_id,
                reason: args.reason,
                evaluator_name: args.evaluator_name,
                evaluator_version: args.evaluator_version,
                labels: args.labels,
                tool_selection: args.tool_selection,
                tool_argument_quality: args.tool_argument_quality,
                tool_ordering: args.tool_ordering,
                verification: args.verification,
                side_effect_safety: args.side_effect_safety,
                overall_score: args.overall_score,
                utility_credit_points_delta: args.utility_credit_points_delta,
                utility_external_ref: args.utility_external_ref,
                json: args.json,
            })
            .await
        }
        _ => unreachable!("router ensures only audience variants reach this dispatch"),
    }
}
