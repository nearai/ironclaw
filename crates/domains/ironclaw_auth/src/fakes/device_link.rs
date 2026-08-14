//! A device-link driver double that walks a real handshake.
//!
//! **Why it models rather than stubs.** The step machine's whole risk surface
//! is sequencing: a poll that advances state instead of reading it, a
//! duplicated transition, a submit against a frame that already moved. A
//! double that answered every call with a canned step would make all three
//! invisible. This one holds per-flow phases, refuses a submit the phase did
//! not ask for, treats `poll` as a pure read while waiting on the user, and
//! records every call so a test can assert the adapter was *not* re-invoked.
//!
//! It is not a vendor: there is no protocol here. It is the smallest state
//! machine that makes the host's sequencing assertions meaningful.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;

use crate::{
    AuthFlowId, CredentialAccountId, CredentialAccountLabel, DeviceLinkBeginRequest,
    DeviceLinkCancelReason, DeviceLinkCancelRequest, DeviceLinkDriver, DeviceLinkDriverError,
    DeviceLinkLinkedAccount, DeviceLinkPollRequest, DeviceLinkStepOutcome, DeviceLinkSubmitRequest,
    DeviceLinkVendorUserRef,
};
use ironclaw_extension_contracts::device_link::{
    DeviceLinkDisplayKind, DeviceLinkErrorCode, DeviceLinkInputKind, DeviceLinkPayload,
    DeviceLinkStep,
};

/// How many polls the double serves before it asks for the password. Small so
/// a test can walk the whole sequence without sleeping.
const POLLS_BEFORE_INPUT: u32 = 1;

/// One call the double served, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceLinkDriverCall {
    Begin(AuthFlowId),
    Poll(AuthFlowId),
    Submit(AuthFlowId, DeviceLinkInputKind),
    Cancel(AuthFlowId, DeviceLinkCancelReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// A payload is on screen; `remaining` polls still answer "not yet".
    Displayed {
        remaining: u32,
    },
    /// The vendor asked for a password and is waiting on the human.
    AwaitingPassword,
    Completed,
    Abandoned,
}

#[derive(Default)]
struct DriverState {
    phases: HashMap<AuthFlowId, Phase>,
    calls: Vec<DeviceLinkDriverCall>,
    payloads: u32,
}

/// A [`DeviceLinkDriver`] double with an observable state machine.
#[derive(Default)]
pub struct RecordingDeviceLinkDriver {
    state: Mutex<DriverState>,
    /// When set, the next vendor call fails with this code instead of
    /// advancing — how a test drives the failure arms without a real vendor.
    next_failure: Mutex<Option<(DeviceLinkErrorCode, bool)>>,
    account_id: CredentialAccountId,
}

impl RecordingDeviceLinkDriver {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(DriverState::default()),
            next_failure: Mutex::new(None),
            account_id: CredentialAccountId::new(),
        }
    }

    /// The account this double reports on completion. A test asserts the flow
    /// record carries exactly this id, which is what proves auth reported
    /// completion against a real credential rather than a bare step.
    pub fn account_id(&self) -> CredentialAccountId {
        self.account_id
    }

    /// Every call served, in order. The sequencing assertions read this.
    pub fn calls(&self) -> Vec<DeviceLinkDriverCall> {
        self.lock_state().calls.clone()
    }

    /// Fail the next vendor call with `code`.
    pub fn fail_next_call(&self, code: DeviceLinkErrorCode, restartable: bool) {
        *self
            .next_failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((code, restartable));
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, DriverState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn take_failure(&self) -> Option<DeviceLinkDriverError> {
        self.next_failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .map(|(code, restartable)| DeviceLinkDriverError::Vendor { code, restartable })
    }

    fn completed_outcome(&self) -> DeviceLinkStepOutcome {
        DeviceLinkStepOutcome {
            step: DeviceLinkStep::Completed {
                account_label: "Fixture personal account".to_string(),
                vendor_user_ref: "+15550000000".to_string(),
            },
            account: Some(DeviceLinkLinkedAccount {
                account_id: self.account_id,
                label: CredentialAccountLabel::new("fixture-personal-account")
                    .expect("fixture account label is valid"), // safety: fixed test-support fixture literal satisfies the validated label grammar
                vendor_user_ref: DeviceLinkVendorUserRef::new("+15550000000")
                    .expect("fixture vendor user ref is valid"), // safety: fixed test-support fixture literal is nonempty, bounded, and control-free
                link_revision: 1,
            }),
        }
    }
}

#[async_trait]
impl DeviceLinkDriver for RecordingDeviceLinkDriver {
    async fn begin(
        &self,
        request: DeviceLinkBeginRequest,
    ) -> Result<DeviceLinkStepOutcome, DeviceLinkDriverError> {
        if let Some(failure) = self.take_failure() {
            return Err(failure);
        }
        let mut state = self.lock_state();
        state
            .calls
            .push(DeviceLinkDriverCall::Begin(request.flow_id));
        state.payloads = state.payloads.saturating_add(1);
        let payload = format!("fixture-device-link-token-{}", state.payloads);
        state.phases.insert(
            request.flow_id,
            Phase::Displayed {
                remaining: POLLS_BEFORE_INPUT,
            },
        );
        Ok(DeviceLinkStepOutcome {
            step: DeviceLinkStep::Display {
                kind: DeviceLinkDisplayKind::QrCode,
                payload: DeviceLinkPayload::new(payload).expect("fixture payload is valid"), // safety: formatted fixture token is nonempty and bounded for every u32 counter value
                expires_in: Duration::from_secs(30),
            },
            account: None,
        })
    }

    async fn poll(
        &self,
        request: DeviceLinkPollRequest,
    ) -> Result<DeviceLinkStepOutcome, DeviceLinkDriverError> {
        if let Some(failure) = self.take_failure() {
            return Err(failure);
        }
        let mut state = self.lock_state();
        state
            .calls
            .push(DeviceLinkDriverCall::Poll(request.flow_id));
        let Some(phase) = state.phases.get_mut(&request.flow_id) else {
            return Err(DeviceLinkDriverError::UnknownFlow);
        };
        match *phase {
            Phase::Displayed { remaining: 0 } => {
                *phase = Phase::AwaitingPassword;
                Ok(DeviceLinkStepOutcome {
                    step: DeviceLinkStep::InputRequired {
                        kind: DeviceLinkInputKind::Password,
                        label: "Account password".to_string(),
                        hint: None,
                    },
                    account: None,
                })
            }
            Phase::Displayed { remaining } => {
                *phase = Phase::Displayed {
                    remaining: remaining - 1,
                };
                Ok(DeviceLinkStepOutcome {
                    step: DeviceLinkStep::AwaitingVendor {
                        retry_in: Duration::from_secs(3),
                    },
                    account: None,
                })
            }
            // A pure read while the human is typing. Advancing here would be
            // the exact race the per-link serialization rule exists for.
            Phase::AwaitingPassword => Ok(DeviceLinkStepOutcome {
                step: DeviceLinkStep::AwaitingVendor {
                    retry_in: Duration::from_secs(3),
                },
                account: None,
            }),
            Phase::Completed => Ok(self.completed_outcome()),
            Phase::Abandoned => Err(DeviceLinkDriverError::UnknownFlow),
        }
    }

    async fn submit(
        &self,
        request: DeviceLinkSubmitRequest,
    ) -> Result<DeviceLinkStepOutcome, DeviceLinkDriverError> {
        let kind = request.input.kind();
        if let Some(failure) = self.take_failure() {
            let mut state = self.lock_state();
            state
                .calls
                .push(DeviceLinkDriverCall::Submit(request.flow_id, kind));
            return Err(failure);
        }
        let mut state = self.lock_state();
        state
            .calls
            .push(DeviceLinkDriverCall::Submit(request.flow_id, kind));
        let Some(phase) = state.phases.get_mut(&request.flow_id) else {
            return Err(DeviceLinkDriverError::UnknownFlow);
        };
        match (*phase, kind) {
            (Phase::AwaitingPassword, DeviceLinkInputKind::Password) => {
                *phase = Phase::Completed;
                drop(state);
                Ok(self.completed_outcome())
            }
            // Fail closed on a value the phase did not ask for, so a host that
            // routed the wrong input cannot pass by accident.
            _ => Err(DeviceLinkDriverError::Vendor {
                code: DeviceLinkErrorCode::InvalidInput,
                restartable: true,
            }),
        }
    }

    async fn cancel(&self, request: DeviceLinkCancelRequest) -> Result<(), DeviceLinkDriverError> {
        let mut state = self.lock_state();
        state.calls.push(DeviceLinkDriverCall::Cancel(
            request.flow_id,
            request.reason,
        ));
        state.phases.insert(request.flow_id, Phase::Abandoned);
        Ok(())
    }
}
