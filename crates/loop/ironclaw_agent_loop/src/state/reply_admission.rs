/// Persistent state owned by `ReplyAdmissionStrategy`.
///
/// Rejected replies are loop-private candidates. The latest rejection is kept
/// until an accepted final reply clears it so checkpoints can resume from the
/// typed control state, while `pending_rejection_rendered` prevents repeating
/// the same control event every prompt.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReplyAdmissionStrategyState {
    #[serde(default)]
    pub rejected_reply_candidates: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_rejection: Option<ReplyAdmissionRejection>,
    #[serde(default)]
    pub pending_rejection_rendered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReplyAdmissionRejection {
    pub reason_code: ReplyAdmissionRejectionReason,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unmet_obligation_refs: Vec<ObligationRef>,
}

impl ReplyAdmissionRejection {
    pub fn stop_condition_not_met() -> Self {
        Self {
            reason_code: ReplyAdmissionRejectionReason::StopConditionNotMet,
            unmet_obligation_refs: Vec::new(),
        }
    }

    pub fn structured_output_required() -> Self {
        Self {
            reason_code: ReplyAdmissionRejectionReason::StructuredOutputRequired,
            unmet_obligation_refs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ObligationRef(String);

impl ObligationRef {
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        if value.is_empty() {
            None
        } else {
            Some(Self(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyAdmissionRejectionReason {
    StopConditionNotMet,
    /// The run's output contract is a JSON schema: plain-text finals are
    /// rejected with a repair hint directing the model to the result tool.
    StructuredOutputRequired,
}
