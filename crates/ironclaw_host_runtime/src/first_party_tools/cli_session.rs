use std::time::Duration;

use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{
    EffectKind, PermissionMode, ResourceCeiling, ResourceEstimate, ResourceProfile,
    RuntimeDispatchErrorKind, SandboxQuota,
};
use serde_json::{Value, json};

use crate::{
    CommandExecutionRequest, FirstPartyCapabilityError, FirstPartyCapabilityRequest,
    RuntimeProcessError,
};

use super::{FIRST_PARTY_MAX_OUTPUT_BYTES, first_party_capability_manifest};

#[path = "cli_session_core.rs"]
mod cli_session_core;

pub const CLI_SESSION_CAPABILITY_ID: &str = "builtin.cli_session";

const SESSION_WALL_CLOCK_DEFAULT_MS: u64 = 5_000;
const SESSION_WALL_CLOCK_MAX_MS: u64 = 30_000;
// Fixed, not model-adjustable: every tmux control op (new-session -d,
// send-keys, capture-pane, kill-session) is near-instant — the exec itself
// never blocks on the launched command's own runtime, only on tmux setting
// up/reading the pane.
const SESSION_TIMEOUT_SECS: u64 = 20;

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        CLI_SESSION_CAPABILITY_ID,
        "Drive a persistent tmux session inside the caller's sandbox container: 'start' launches a \
         detached session running one command, 'send' types text into it followed by Enter, 'read' \
         returns the currently rendered pane text (not full scrollback), 'kill' ends it. Sessions are \
         namespaced per call and die when the sandbox container stops (idle-timeout or restart) — they \
         do not survive a container recycle. 'start' and 'read' report every currently live session \
         name under active_sessions.",
        vec![
            EffectKind::DispatchCapability,
            EffectKind::SpawnProcess,
            EffectKind::ExecuteCode,
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::Network,
        ],
        PermissionMode::Ask,
        Some(ResourceProfile {
            default_estimate: ResourceEstimate::default()
                .set_wall_clock_ms(SESSION_WALL_CLOCK_DEFAULT_MS)
                .set_output_bytes(4096)
                .set_process_count(1),
            hard_ceiling: Some(ResourceCeiling {
                max_usd: None,
                max_input_tokens: None,
                max_output_tokens: None,
                max_wall_clock_ms: Some(SESSION_WALL_CLOCK_MAX_MS),
                max_output_bytes: Some(FIRST_PARTY_MAX_OUTPUT_BYTES),
                sandbox: Some(SandboxQuota {
                    process_count: Some(1),
                    ..SandboxQuota::default()
                }),
            }),
        }),
    )
}

pub(super) async fn dispatch(
    request: &FirstPartyCapabilityRequest,
) -> Result<(Value, Duration), FirstPartyCapabilityError> {
    let parsed =
        cli_session_core::parse_cli_session_request(&request.input).map_err(session_error)?;
    let command = cli_session_core::build_tmux_command(&parsed);
    let action = parsed.action;
    let session_name = parsed.session.as_str().to_string();

    let output = request
        .services
        .process
        .run_command(CommandExecutionRequest {
            scope: request.scope.clone(),
            mounts: request.mounts.clone(),
            command,
            workdir: None,
            timeout_secs: Some(SESSION_TIMEOUT_SECS),
            output_limit_bytes: Some(FIRST_PARTY_MAX_OUTPUT_BYTES),
            extra_env: Default::default(),
            // Every tmux control op is a foreground, blocking exec (Phase A
            // Task A4's background-job path is for `builtin.shell`'s own
            // long-running-command support, not for cli_session verbs).
            background: false,
        })
        .await
        .map_err(process_error)?;

    let (primary_output, active_sessions) = cli_session_core::split_session_footer(&output.output);
    let mut value = json!({
        "action": action_str(action),
        "session": session_name,
        "output": primary_output,
        "exit_code": output.exit_code,
        "success": output.exit_code == 0,
    });
    if let Some(sessions) = active_sessions {
        value["active_sessions"] = json!(sessions);
    }
    Ok((value, output.duration))
}

fn action_str(action: cli_session_core::CliSessionAction) -> &'static str {
    match action {
        cli_session_core::CliSessionAction::Start => "start",
        cli_session_core::CliSessionAction::Send => "send",
        cli_session_core::CliSessionAction::Read => "read",
        cli_session_core::CliSessionAction::Kill => "kill",
    }
}

fn session_error(error: cli_session_core::CliSessionError) -> FirstPartyCapabilityError {
    let cli_session_core::CliSessionError::InvalidParameters(reason) = error;
    FirstPartyCapabilityError::with_safe_summary(RuntimeDispatchErrorKind::InputEncode, reason)
}

fn process_error(error: RuntimeProcessError) -> FirstPartyCapabilityError {
    let (kind, reason) = match error {
        RuntimeProcessError::Timeout(duration) => (
            RuntimeDispatchErrorKind::Resource,
            format!(
                "cli_session command timed out after {}s",
                duration.as_secs()
            ),
        ),
        RuntimeProcessError::ExecutionFailed(reason) => (
            RuntimeDispatchErrorKind::Executor,
            format!("cli_session execution failed: {reason}"),
        ),
    };
    FirstPartyCapabilityError::with_safe_summary(kind, reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommandExecutionOutput, RuntimeProcessError, RuntimeProcessPort};
    use async_trait::async_trait;
    use ironclaw_host_api::{CapabilityId, ResourceScope};
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingProcessPort {
        requests: Mutex<Vec<CommandExecutionRequest>>,
    }

    #[async_trait]
    impl RuntimeProcessPort for RecordingProcessPort {
        async fn run_command(
            &self,
            request: CommandExecutionRequest,
        ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
            self.requests.lock().unwrap().push(request);
            Ok(CommandExecutionOutput {
                output: "session output".to_string(),
                saved_output: None,
                exit_code: 0,
                sandboxed: true,
                duration: Duration::from_millis(4),
            })
        }
    }

    #[tokio::test]
    async fn dispatch_forwards_the_built_tmux_command_to_the_process_port() {
        let port = std::sync::Arc::new(RecordingProcessPort::default());
        let request = FirstPartyCapabilityRequest::request_for_test(
            CapabilityId::new(CLI_SESSION_CAPABILITY_ID).unwrap(),
            ResourceScope::system(),
            json!({"action": "start", "session": "devserver", "command": "npm run dev"}),
            None,
        );
        // request_for_test wires HostProcessPort by default; override for this test.
        let mut request = request;
        request.services.process = port.clone();

        let (output, _duration) = dispatch(&request).await.unwrap();

        assert_eq!(output["session"], json!("ic-devserver"));
        assert_eq!(output["success"], json!(true));
        let requests = port.requests.lock().unwrap();
        assert_eq!(
            requests[0].command,
            "tmux new-session -d -s 'ic-devserver' 'npm run dev'; \
             printf '\\n---IRONCLAW-CLI-SESSIONS---\\n'; \
             tmux list-sessions -F '#S' 2>/dev/null || true"
        );
    }

    #[tokio::test]
    async fn dispatch_rejects_missing_command_before_touching_the_process_port() {
        let port = std::sync::Arc::new(RecordingProcessPort::default());
        let mut request = FirstPartyCapabilityRequest::request_for_test(
            CapabilityId::new(CLI_SESSION_CAPABILITY_ID).unwrap(),
            ResourceScope::system(),
            json!({"action": "start", "session": "devserver"}),
            None,
        );
        request.services.process = port.clone();

        let error = dispatch(&request).await.unwrap_err();

        assert!(port.requests.lock().unwrap().is_empty());
        match error {
            FirstPartyCapabilityError::Dispatch { safe_summary, .. } => {
                assert!(safe_summary.unwrap().contains("'command' is required"));
            }
            other => panic!("expected Dispatch error, got {other:?}"),
        }
    }
}
