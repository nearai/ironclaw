use ironclaw_host_api::{InvocationId, ResourceScope};
use ironclaw_run_state::{AuthRequiredPayload, RunStateError, RunStateStore, RunStatus};
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthCallbackOutcome {
    pub flow_id: Uuid,
    pub success: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeSignal {
    pub invocation_id: Option<InvocationId>,
    pub credential_name: String,
    pub scope: ResourceScope,
    pub outcome: OAuthCallbackOutcome,
}

#[derive(Debug, Clone)]
pub struct OAuthResumeNotifier {
    sender: broadcast::Sender<ResumeSignal>,
}

impl OAuthResumeNotifier {
    pub fn new(sender: broadcast::Sender<ResumeSignal>) -> Self {
        Self { sender }
    }

    pub fn channel(capacity: usize) -> (Self, broadcast::Receiver<ResumeSignal>) {
        let (sender, receiver) = broadcast::channel(capacity);
        (Self::new(sender), receiver)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ResumeSignal> {
        self.sender.subscribe()
    }

    pub fn notify(&self, credential_name: impl Into<String>, scope: ResourceScope, flow_id: Uuid) {
        self.notify_signal(ResumeSignal {
            invocation_id: None,
            credential_name: credential_name.into(),
            scope,
            outcome: OAuthCallbackOutcome {
                flow_id,
                success: true,
            },
        });
    }

    pub async fn notify_blocked_auth(
        &self,
        run_state: &dyn RunStateStore,
        credential_name: &str,
        scope: &ResourceScope,
        flow_id: Uuid,
    ) -> Result<usize, RunStateError> {
        let mut sent = 0;
        for record in run_state.records_for_scope(scope).await? {
            if record.status != RunStatus::BlockedAuth {
                continue;
            }
            let Some(payload) = record.blocked_payload else {
                continue;
            };
            let payload: AuthRequiredPayload = serde_json::from_value(payload)
                .map_err(|error| RunStateError::Deserialization(error.to_string()))?;
            if payload.credential_name != credential_name || payload.flow_id != flow_id {
                continue;
            }
            self.notify_signal(ResumeSignal {
                invocation_id: Some(record.invocation_id),
                credential_name: credential_name.to_string(),
                scope: record.scope,
                outcome: OAuthCallbackOutcome {
                    flow_id,
                    success: true,
                },
            });
            sent += 1;
        }
        Ok(sent)
    }

    pub fn notify_signal(&self, signal: ResumeSignal) {
        if self.sender.receiver_count() == 0 {
            return;
        }
        let _ = self.sender.send(signal);
    }
}

impl Default for OAuthResumeNotifier {
    fn default() -> Self {
        Self::channel(16).0
    }
}
