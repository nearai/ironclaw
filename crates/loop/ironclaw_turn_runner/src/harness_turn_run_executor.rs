//! Explicitly configured ACP harness execution for selected run profiles.

use std::{
    collections::HashSet,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    ContentBlock, ContentChunk, InitializeRequest, LoadSessionRequest, NewSessionRequest,
    PermissionOptionKind, PromptRequest, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SelectedPermissionOutcome, SessionId, SessionNotification,
    SessionUpdate, TextContent,
};
use agent_client_protocol::{Agent, ConnectionTo, Lines};
use async_trait::async_trait;
use ironclaw_host_api::failure::categories::{
    HOST_STAGE_UNAVAILABLE_INPUT_CATEGORY, TRANSCRIPT_WRITE_FAILED_CATEGORY,
};
use ironclaw_loop_contracts::{
    AssistantReply, FinalizeAssistantMessage, LoopCompleted, LoopCompletionKind, LoopExit,
};
use ironclaw_processes::ProcessTransitionPort;
use ironclaw_threads::{MessageKind, SessionThreadService, ThreadMessageId, ThreadScope};
use ironclaw_turns::{SanitizedFailure, TurnError, runner::ClaimedTurnRun};
use tracing::debug;

use crate::{
    agent_placement::{AgentLineSink, AgentLineStream, AgentPlacement},
    turn_runner::HostFactory,
    turn_scheduler::{TurnRunExecutor, TurnRunExecutorError},
};

const SESSION_ID_FILE: &str = ".ironclaw-acp-session";

/// Explicit routing and resource limits for the ACP executor.
#[derive(Clone)]
pub struct HarnessTurnRunConfig {
    pub run_profile_ids: HashSet<String>,
    pub timeout: Duration,
    pub max_update_bytes: usize,
    pub placement: Arc<dyn AgentPlacement>,
}

pub struct HarnessExecutorConfig {
    timeout: Duration,
    max_update_bytes: usize,
    placement: Arc<dyn AgentPlacement>,
}

impl HarnessTurnRunConfig {
    pub fn into_routing_parts(self) -> (HashSet<String>, HarnessExecutorConfig) {
        (
            self.run_profile_ids,
            HarnessExecutorConfig {
                timeout: self.timeout,
                max_update_bytes: self.max_update_bytes,
                placement: self.placement,
            },
        )
    }
}

/// ACP-only turn executor. Profile selection belongs to the neutral executor
/// router, so this implementation contains no knowledge of other loops.
pub struct HarnessTurnRunExecutor {
    host_factory: Arc<dyn HostFactory>,
    thread_service: Arc<dyn SessionThreadService>,
    thread_scope: ThreadScope,
    config: HarnessExecutorConfig,
}

impl HarnessTurnRunExecutor {
    pub fn new(
        host_factory: Arc<dyn HostFactory>,
        thread_service: Arc<dyn SessionThreadService>,
        thread_scope: ThreadScope,
        config: HarnessExecutorConfig,
    ) -> Result<Self, String> {
        if config.timeout.is_zero() {
            return Err("ACP harness timeout must be greater than zero".to_string());
        }
        if config.max_update_bytes == 0 {
            return Err("ACP harness update bound must be greater than zero".to_string());
        }
        Ok(Self {
            host_factory,
            thread_service,
            thread_scope,
            config,
        })
    }

    async fn execute_harness(&self, claimed: ClaimedTurnRun) -> Result<(), TurnRunExecutorError> {
        let prompt = self.load_prompt(&claimed).await?;
        let mut process = self
            .config
            .placement
            .spawn(&claimed.state.scope.thread_id)
            .await
            .map_err(|_| failure("driver_unavailable"))?;
        let workspace = process.workspace().to_path_buf();
        let working_directory = process.working_directory().to_path_buf();
        let transport = process
            .take_transport()
            .map_err(|_| failure("driver_unavailable"));

        let result = match transport {
            Ok((outgoing, incoming)) => {
                let protocol = self.run_acp(
                    prompt,
                    workspace,
                    working_directory,
                    Lines::new(outgoing, incoming),
                );
                match tokio::time::timeout(self.config.timeout, protocol).await {
                    Ok(result) => result,
                    Err(_) => Err(failure("interrupted_unexpectedly")),
                }
            }
            Err(error) => Err(error),
        };
        process
            .terminate()
            .await
            .map_err(|_| failure("driver_failed"))?;
        let reply = result?;

        let host = self
            .host_factory
            .create_host(&claimed)
            .await
            .map_err(|_| failure("host_creation_failed"))?;
        let reply_ref = host
            .finalize_assistant_message(FinalizeAssistantMessage {
                reply: AssistantReply { content: reply },
            })
            .await
            .map_err(|_| failure(TRANSCRIPT_WRITE_FAILED_CATEGORY))?;
        let exit_id = ironclaw_turns::LoopExitId::new(format!(
            "exit:{}-harness-completed",
            claimed.state.run_id
        ))
        .map_err(|_| failure("driver_protocol_violation"))?;
        let exit = LoopExit::Completed(LoopCompleted {
            completion_kind: LoopCompletionKind::FinalReply,
            reply_message_refs: vec![reply_ref],
            result_refs: Vec::new(),
            final_checkpoint_id: None,
            model_usage: None,
            exit_id,
        });
        self.fallback
            .apply_exit(&claimed, exit)
            .await
            .map_err(|()| failure("exit_application_failed"))
    }

    async fn load_prompt(&self, claimed: &ClaimedTurnRun) -> Result<String, TurnRunExecutorError> {
        let raw_message_id = claimed
            .state
            .accepted_message_ref
            .as_str()
            .strip_prefix("msg:")
            .ok_or_else(|| failure("driver_invalid_request"))?;
        let message_id = ThreadMessageId::parse(raw_message_id)
            .map_err(|_| failure("driver_invalid_request"))?;
        let scope = ironclaw_loop_host::ThreadScopeResolver::resolve_for_turn(
            &self.thread_scope,
            &claimed.state.scope,
            claimed.state.actor.as_ref(),
        );
        let message = self
            .thread_service
            .read_thread_message(&scope, &claimed.state.scope.thread_id, message_id)
            .await
            .map_err(|_| failure(HOST_STAGE_UNAVAILABLE_INPUT_CATEGORY))?
            .ok_or_else(|| failure(HOST_STAGE_UNAVAILABLE_INPUT_CATEGORY))?;
        if message.kind != MessageKind::User {
            return Err(failure("driver_invalid_request"));
        }
        message
            .content
            .ok_or_else(|| failure("driver_invalid_request"))
    }

    async fn run_acp(
        &self,
        prompt: String,
        workspace: std::path::PathBuf,
        working_directory: std::path::PathBuf,
        transport: Lines<AgentLineSink, AgentLineStream>,
    ) -> Result<String, TurnRunExecutorError> {
        let output = Arc::new(Mutex::new(BoundedOutput::new(self.config.max_update_bytes)));
        let output_for_updates = Arc::clone(&output);
        let session_file = workspace.join(SESSION_ID_FILE);

        agent_client_protocol::Client
            .builder()
            .name("ironclaw-harness-v0")
            .on_receive_notification(
                async move |notification: SessionNotification, _cx| {
                    if let SessionUpdate::AgentMessageChunk(ContentChunk {
                        content: ContentBlock::Text(text),
                        ..
                    }) = notification.update
                        && let Ok(mut output) = output_for_updates.lock()
                    {
                        output.push(&text.text);
                    }
                    Ok(())
                },
                agent_client_protocol::on_receive_notification!(),
            )
            .on_receive_request(
                async move |request: RequestPermissionRequest, responder, _connection| {
                    debug!(
                        session_id = %request.session_id,
                        "auto-approving ACP harness permission request"
                    );
                    let response = match request.options.iter().find(|option| {
                        matches!(
                            option.kind,
                            PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
                        )
                    }) {
                        Some(option) => {
                            RequestPermissionResponse::new(RequestPermissionOutcome::Selected(
                                SelectedPermissionOutcome::new(option.option_id.clone()),
                            ))
                        }
                        None => RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
                    };
                    responder.respond(response)
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_with(transport, |connection: ConnectionTo<Agent>| async move {
                connection
                    .send_request(InitializeRequest::new(ProtocolVersion::V1))
                    .block_task()
                    .await?;
                let session_id = match load_session_id(&session_file).await {
                    Ok(Some(session_id)) => {
                        connection
                            .send_request(LoadSessionRequest::new(
                                session_id.clone(),
                                &working_directory,
                            ))
                            .block_task()
                            .await?;
                        session_id
                    }
                    Ok(None) => {
                        let response = connection
                            .send_request(NewSessionRequest::new(&working_directory))
                            .block_task()
                            .await?;
                        persist_session_id(&session_file, &response.session_id).await?;
                        response.session_id
                    }
                    Err(error) => return Err(error),
                };
                connection
                    .send_request(PromptRequest::new(
                        session_id,
                        vec![ContentBlock::Text(TextContent::new(prompt))],
                    ))
                    .block_task()
                    .await?;
                Ok(())
            })
            .await
            .map_err(|_| failure("driver_protocol_violation"))?;

        let output = output
            .lock()
            .map_err(|_| failure("driver_protocol_violation"))?
            .render();
        if output.trim().is_empty() {
            return Err(failure("invalid_model_output"));
        }
        Ok(output)
    }
}

#[async_trait]
impl TurnRunExecutor for HarnessTurnRunExecutor {
    async fn execute_claimed_run(
        &self,
        claimed: ClaimedTurnRun,
        _process_transitions: Arc<dyn ProcessTransitionPort<Error = TurnError>>,
    ) -> Result<(), TurnRunExecutorError> {
        self.execute_harness(claimed).await
    }
}

struct BoundedOutput {
    text: String,
    limit: usize,
    truncated: bool,
}

impl BoundedOutput {
    fn new(limit: usize) -> Self {
        Self {
            text: String::new(),
            limit,
            truncated: false,
        }
    }

    fn push(&mut self, value: &str) {
        if self.text.len() >= self.limit {
            self.truncated = true;
            return;
        }
        let remaining = self.limit - self.text.len();
        if value.len() <= remaining {
            self.text.push_str(value);
        } else {
            let mut end = remaining;
            while end > 0 && !value.is_char_boundary(end) {
                end -= 1;
            }
            self.text.push_str(&value[..end]);
            self.truncated = true;
        }
    }

    fn render(&self) -> String {
        if self.truncated {
            format!("{}\n[ACP output truncated]", self.text)
        } else {
            self.text.clone()
        }
    }
}

async fn load_session_id(path: &Path) -> agent_client_protocol::Result<Option<SessionId>> {
    match tokio::fs::read_to_string(path).await {
        Ok(raw) => {
            let value = raw.trim();
            if value.is_empty() || value.len() > 1024 {
                return Err(agent_client_protocol::Error::internal_error()
                    .data("persisted ACP session id is invalid"));
            }
            Ok(Some(SessionId::new(value.to_string())))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(agent_client_protocol::Error::internal_error().data(format!(
            "persisted ACP session id could not be read: {error}"
        ))),
    }
}

async fn persist_session_id(
    path: &Path,
    session_id: &SessionId,
) -> agent_client_protocol::Result<()> {
    let temporary = path.with_extension("tmp");
    tokio::fs::write(&temporary, session_id.to_string())
        .await
        .map_err(|error| {
            agent_client_protocol::Error::internal_error()
                .data(format!("ACP session id could not be staged: {error}"))
        })?;
    tokio::fs::rename(&temporary, path).await.map_err(|error| {
        agent_client_protocol::Error::internal_error()
            .data(format!("ACP session id could not be persisted: {error}"))
    })
}

fn failure(category: &'static str) -> TurnRunExecutorError {
    let failure = SanitizedFailure::from_trusted_static(category);
    TurnRunExecutorError::from_failure(failure)
}

#[cfg(test)]
mod tests {
    use super::BoundedOutput;

    #[test]
    fn bounded_output_truncates_at_utf8_boundary_with_marker() {
        let mut output = BoundedOutput::new(5);
        output.push("abc😀");
        assert_eq!(output.render(), "abc\n[ACP output truncated]");
    }
}
