//! Auto-split per-audience dispatch for the `traces` CLI surface.
//!
//! Audience: contributor. See `super::run_traces` for the routing.

use super::*;

pub(super) async fn dispatch(cmd: TracesSubcommand) -> anyhow::Result<()> {
    match cmd {
        TracesSubcommand::OptIn {
            endpoint,
            user_scope,
            bearer_token_env,
            upload_token_issuer_url,
            upload_token_issuer_allowed_hosts,
            upload_token_audience,
            upload_token_tenant_id,
            upload_token_workload_token_env,
            upload_token_invite_code,
            upload_token_issuer_timeout_ms,
            include_message_text,
            include_tool_payloads,
            scope,
            selected_tools,
            allow_pii_review_bypass,
            min_submission_score,
        } => opt_in(OptInOptions {
            endpoint,
            user_scope,
            bearer_token_env,
            upload_token_issuer_url,
            upload_token_issuer_allowed_hosts,
            upload_token_audience,
            upload_token_tenant_id,
            upload_token_workload_token_env,
            upload_token_invite_code,
            upload_token_issuer_timeout_ms,
            include_message_text,
            include_tool_payloads,
            scope,
            selected_tools,
            allow_pii_review_bypass,
            min_submission_score,
        }),
        TracesSubcommand::OptOut { user_scope } => opt_out(user_scope.as_deref()),
        TracesSubcommand::Status { json, user_scope } => {
            show_policy_status(json, user_scope.as_deref())
        }
        TracesSubcommand::Preview {
            recorded_trace,
            include_message_text,
            include_tool_payloads,
            scope,
            channel,
            engine_version,
            contributor_id,
            credit_account_ref,
            output,
            enqueue,
        } => {
            preview_recorded_trace(PreviewOptions {
                recorded_trace,
                include_message_text,
                include_tool_payloads,
                scope,
                channel,
                engine_version,
                contributor_id,
                credit_account_ref,
                output,
                enqueue,
            })
            .await
        }
        TracesSubcommand::Enqueue { envelope } => {
            let envelope = load_envelope(&envelope)?;
            let policy = read_policy()?;
            enqueue_envelope_with_policy(
                &envelope,
                &policy,
                TraceContributionAcceptance::QueueFromPreview,
            )?;
            println!(
                "Queued redacted trace contribution {}",
                envelope.submission_id
            );
            Ok(())
        }
        TracesSubcommand::FlushQueue { limit } => flush_queue(limit).await,
        TracesSubcommand::QueueStatus { json, scope } => show_queue_status(json, scope.as_deref()),
        TracesSubcommand::Credit {
            json,
            notice,
            notice_scope,
            ack,
            snooze_hours,
        } => show_credit(json, notice, notice_scope.as_deref(), ack, snooze_hours).await,
        TracesSubcommand::Submit {
            envelope,
            endpoint,
            bearer_token_env,
        } => submit_envelope(&envelope, &endpoint, &bearer_token_env).await,
        TracesSubcommand::ListSubmissions { json, summary } => {
            list_submissions(json, summary).await
        }
        TracesSubcommand::Revoke {
            submission_id,
            endpoint,
            bearer_token_env,
        } => revoke_submission(submission_id, endpoint.as_deref(), &bearer_token_env).await,
        TracesSubcommand::IngestHealth { endpoint, json } => {
            trace_commons_ingest_health(&endpoint, json).await
        }
        TracesSubcommand::Profile { command } => match command {
            TracesProfileSubcommand::Token { user_scope, json } => {
                profile_token(user_scope.as_deref(), json).await
            }
            TracesProfileSubcommand::Set {
                handle,
                bio,
                user_scope,
            } => profile_set(user_scope.as_deref(), &handle, bio.as_deref()).await,
            TracesProfileSubcommand::Withdraw { user_scope } => {
                profile_withdraw(user_scope.as_deref()).await
            }
        },
    }
}
