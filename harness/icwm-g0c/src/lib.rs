//! Candidate-neutral deterministic contracts for ICWM Gate 0C architecture spikes.
//!
//! This disposable crate deliberately has no IronClaw or Matrix SDK dependency.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::error::Error;
use std::fmt::{Display, Formatter};

pub const CONTRACT_VERSION: &str = "icwm.g0c.harness.v1";
pub const TRACE_VERSION: &str = "icwm.g0c.harness-trace.v1";

/// A stable content identity derived from typed, length-delimited bytes.
///
/// It deliberately does not hash a serialization format: changing JSON field
/// order, whitespace, or a serializer cannot change this identity.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct StableId(String);

impl StableId {
    pub fn derive(domain: &str, components: &[&[u8]]) -> Self {
        let mut hash = Sha256::new();
        hash.update(
            u64::try_from(CONTRACT_VERSION.len())
                .unwrap_or(u64::MAX)
                .to_be_bytes(),
        );
        hash.update(CONTRACT_VERSION.as_bytes());
        hash.update(
            u64::try_from(domain.len())
                .unwrap_or(u64::MAX)
                .to_be_bytes(),
        );
        hash.update(domain.as_bytes());
        hash.update(
            u64::try_from(components.len())
                .unwrap_or(u64::MAX)
                .to_be_bytes(),
        );
        for component in components {
            hash.update(
                u64::try_from(component.len())
                    .unwrap_or(u64::MAX)
                    .to_be_bytes(),
            );
            hash.update(component);
        }
        Self(format!("{:x}", hash.finalize()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CandidateId {
    pub name: String,
    pub version: String,
    pub stable_id: StableId,
}

impl CandidateId {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        let name = name.into();
        let version = version.into();
        let stable_id = StableId::derive("candidate", &[name.as_bytes(), version.as_bytes()]);
        Self {
            name,
            version,
            stable_id,
        }
    }
}

/// Credential-free inputs. Authentication injection belongs to G0B and cannot
/// be represented by this contract.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum MatrixInput {
    BeginOperation {
        operation_id: StableId,
    },
    StepOperation {
        operation_id: StableId,
        step: String,
    },
    Sync {
        next_batch: String,
        body: Vec<u8>,
    },
    DeviceKeys {
        user_id: String,
        body: Vec<u8>,
    },
    OneTimeKeys {
        user_id: String,
        device_id: String,
        body: Vec<u8>,
    },
    ToDevice {
        sender: String,
        event_type: String,
        body: Vec<u8>,
    },
    RoomEvent {
        room_id: String,
        event_id: String,
        body: Vec<u8>,
    },
    Emit(EffectIntent),
    ConsumeNextResponse,
    Cancel {
        operation_id: StableId,
    },
    StaleWriterAttempt {
        holder_id: String,
        lease_epoch: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum EffectKind {
    MatrixRequest {
        method: String,
        path: String,
        body: Vec<u8>,
    },
    CryptoStoreWrite {
        scope: String,
        record_type: String,
        bytes: Vec<u8>,
    },
    IngressDisposition {
        source_id: String,
        event_id: String,
        disposition: String,
    },
    PreparedCiphertext {
        room_id: String,
        bytes: Vec<u8>,
    },
    Observation {
        name: String,
        value: String,
    },
    Cancellation {
        operation_id: StableId,
    },
    StaleWriterRejected {
        holder_id: String,
        lease_epoch: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EffectIntent {
    pub effect_id: StableId,
    pub kind: EffectKind,
}

impl EffectIntent {
    pub fn new(kind: EffectKind, effect_id: StableId) -> Self {
        Self { effect_id, kind }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecordedEffect {
    pub sequence: u64,
    pub at_ms: u64,
    pub intent: EffectIntent,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseDelivery {
    Complete,
    Partial,
    LostBeforeCandidate,
    LostAfterAdapterHandling,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ScriptedResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub delivery: ResponseDelivery,
    /// Exact prefix exposed for `partial`; forbidden for every other mode.
    pub partial_bytes: Option<usize>,
}

impl ScriptedResponse {
    pub fn matrix(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            body,
            delivery: ResponseDelivery::Complete,
            partial_bytes: None,
        }
    }

    pub fn with_delivery(mut self, delivery: ResponseDelivery) -> Self {
        self.delivery = delivery;
        self
    }

    pub fn partial(mut self, bytes: usize) -> Self {
        self.delivery = ResponseDelivery::Partial;
        self.partial_bytes = Some(bytes);
        self
    }
}

/// Named failure vocabulary shared by all candidate adapters.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Failpoint {
    LedgerBeforeCryptoStoreAppend,
    LedgerAfterCryptoStoreAppend,
    LedgerUnknownCryptoStoreAppend,
    LedgerBeforeIngressAppend,
    LedgerAfterIngressAppend,
    LedgerUnknownIngressAppend,
    CandidateBeforePrepare,
    CandidateAfterPrepare,
    CandidateAfterCommitBeforeAcknowledge,
    ProcessBeforeEffectAppend,
    ProcessAfterEffectAppend,
    ResponseBeforePrepare,
    ResponseAfterCommitBeforeAcknowledge,
    BeforePreparedCiphertextHandoff,
    AfterPreparedCiphertextHandoff,
    StaleWriterAfterTakeover,
}

impl Failpoint {
    fn is_uncertain(self) -> bool {
        matches!(
            self,
            Self::LedgerAfterCryptoStoreAppend
                | Self::LedgerUnknownCryptoStoreAppend
                | Self::LedgerAfterIngressAppend
                | Self::LedgerUnknownIngressAppend
                | Self::CandidateAfterCommitBeforeAcknowledge
                | Self::ProcessAfterEffectAppend
                | Self::ResponseAfterCommitBeforeAcknowledge
                | Self::AfterPreparedCiphertextHandoff
                | Self::StaleWriterAfterTakeover
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartReason {
    CrashRecovery,
    CleanRestart,
    SuccessorGeneration,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CrashRecord {
    pub at_ms: u64,
    pub reached: Option<Failpoint>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseLossPhase {
    BeforeCandidate,
    AfterAdapterHandling,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResponseLossRecord {
    pub at_ms: u64,
    pub phase: ResponseLossPhase,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct VirtualClock {
    now_ms: u64,
}

impl VirtualClock {
    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }

    pub fn set_ms(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
    }

    pub fn advance_ms(&mut self, delta_ms: u64) -> Result<(), HarnessError> {
        self.now_ms = self
            .now_ms
            .checked_add(delta_ms)
            .ok_or(HarnessError::ClockOverflow)?;
        Ok(())
    }
}

pub trait CandidateAdapter {
    type Prepared;
    type PreparedResponse;

    fn contract_version(&self) -> &'static str {
        CONTRACT_VERSION
    }

    fn candidate(&self) -> &CandidateId;
    fn prepare(&mut self, input: MatrixInput) -> Result<Self::Prepared, HarnessError>;
    fn commit(&mut self, prepared: &Self::Prepared) -> Result<Vec<EffectIntent>, HarnessError>;
    fn acknowledge(&mut self, prepared: Self::Prepared) -> Result<(), HarnessError>;
    fn prepare_response(
        &mut self,
        response: ScriptedResponse,
    ) -> Result<Self::PreparedResponse, HarnessError>;
    fn commit_response(
        &mut self,
        prepared: &Self::PreparedResponse,
    ) -> Result<Vec<EffectIntent>, HarnessError>;
    fn acknowledge_response(
        &mut self,
        prepared: Self::PreparedResponse,
    ) -> Result<(), HarnessError>;
    fn crash(&mut self, failpoint: Option<Failpoint>) -> Result<(), HarnessError>;
    fn restart(&mut self, reason: RestartReason) -> Result<(), HarnessError>;
}

#[derive(Clone, Debug)]
pub struct ControlAdapter {
    candidate: CandidateId,
    restart_count: u64,
    active_operations: BTreeSet<StableId>,
    cancelled_operations: BTreeSet<StableId>,
    observed_response_bodies: Vec<Vec<u8>>,
    acknowledged_inputs: u64,
    acknowledged_responses: u64,
}

impl ControlAdapter {
    pub fn new(candidate: CandidateId) -> Self {
        Self {
            candidate,
            restart_count: 0,
            active_operations: BTreeSet::new(),
            cancelled_operations: BTreeSet::new(),
            observed_response_bodies: Vec::new(),
            acknowledged_inputs: 0,
            acknowledged_responses: 0,
        }
    }

    pub fn restart_count(&self) -> u64 {
        self.restart_count
    }

    pub fn is_cancelled(&self, operation_id: &StableId) -> bool {
        self.cancelled_operations.contains(operation_id)
    }

    pub fn observed_response_bodies(&self) -> &[Vec<u8>] {
        &self.observed_response_bodies
    }

    pub fn acknowledged_inputs(&self) -> u64 {
        self.acknowledged_inputs
    }

    pub fn acknowledged_responses(&self) -> u64 {
        self.acknowledged_responses
    }
}

impl CandidateAdapter for ControlAdapter {
    type Prepared = MatrixInput;
    type PreparedResponse = ScriptedResponse;

    fn candidate(&self) -> &CandidateId {
        &self.candidate
    }

    fn prepare(&mut self, input: MatrixInput) -> Result<Self::Prepared, HarnessError> {
        Ok(input)
    }

    fn commit(&mut self, prepared: &Self::Prepared) -> Result<Vec<EffectIntent>, HarnessError> {
        match prepared.clone() {
            MatrixInput::BeginOperation { operation_id } => {
                if !self.active_operations.insert(operation_id.clone()) {
                    return Err(HarnessError::OperationAlreadyActive { operation_id });
                }
                Ok(Vec::new())
            }
            MatrixInput::StepOperation { operation_id, step } => {
                if !self.active_operations.contains(&operation_id) {
                    return Err(HarnessError::OperationNotActive { operation_id });
                }
                Ok(vec![EffectIntent::new(
                    EffectKind::Observation {
                        name: "operation_step".into(),
                        value: step.clone(),
                    },
                    StableId::derive(
                        "operation-step",
                        &[operation_id.as_str().as_bytes(), step.as_bytes()],
                    ),
                )])
            }
            MatrixInput::Emit(intent) => Ok(vec![intent]),
            MatrixInput::ConsumeNextResponse => Err(HarnessError::ControlInputUnsupported {
                input: "response consumption is owned by Harness::drive_inner".into(),
            }),
            MatrixInput::Cancel { operation_id } => {
                if !self.active_operations.remove(&operation_id) {
                    return Err(HarnessError::OperationNotActive { operation_id });
                }
                self.cancelled_operations.insert(operation_id.clone());
                Ok(vec![EffectIntent::new(
                    EffectKind::Cancellation {
                        operation_id: operation_id.clone(),
                    },
                    StableId::derive("cancellation", &[operation_id.as_str().as_bytes()]),
                )])
            }
            MatrixInput::StaleWriterAttempt {
                holder_id,
                lease_epoch,
            } => Ok(vec![EffectIntent::new(
                EffectKind::StaleWriterRejected {
                    holder_id: holder_id.clone(),
                    lease_epoch,
                },
                StableId::derive(
                    "stale-writer",
                    &[holder_id.as_bytes(), &lease_epoch.to_be_bytes()],
                ),
            )]),
            other => Err(HarnessError::ControlInputUnsupported {
                input: format!("{other:?}"),
            }),
        }
    }

    fn acknowledge(&mut self, _prepared: Self::Prepared) -> Result<(), HarnessError> {
        self.acknowledged_inputs = self
            .acknowledged_inputs
            .checked_add(1)
            .ok_or(HarnessError::SequenceOverflow)?;
        Ok(())
    }

    fn prepare_response(
        &mut self,
        response: ScriptedResponse,
    ) -> Result<Self::PreparedResponse, HarnessError> {
        Ok(response)
    }

    fn commit_response(
        &mut self,
        prepared: &Self::PreparedResponse,
    ) -> Result<Vec<EffectIntent>, HarnessError> {
        self.observed_response_bodies.push(prepared.body.clone());
        Ok(Vec::new())
    }

    fn acknowledge_response(
        &mut self,
        _prepared: Self::PreparedResponse,
    ) -> Result<(), HarnessError> {
        self.acknowledged_responses = self
            .acknowledged_responses
            .checked_add(1)
            .ok_or(HarnessError::SequenceOverflow)?;
        Ok(())
    }

    fn crash(&mut self, _failpoint: Option<Failpoint>) -> Result<(), HarnessError> {
        Ok(())
    }

    fn restart(&mut self, _reason: RestartReason) -> Result<(), HarnessError> {
        self.restart_count = self
            .restart_count
            .checked_add(1)
            .ok_or(HarnessError::SequenceOverflow)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultDisposition {
    Supported,
    Failed,
    Uncertain,
    Infeasible,
    NotApplicable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyRecord {
    pub scope: String,
    pub reason: String,
    pub evidence: String,
    pub reviewer_identity: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunPlan {
    pub predeclared_disposition: Option<ResultDisposition>,
    pub disposition_reason: Option<String>,
    #[serde(default)]
    pub capabilities: BTreeMap<String, bool>,
    #[serde(default)]
    pub capability_dispositions: BTreeMap<String, ResultDisposition>,
    #[serde(default)]
    pub capability_disposition_reasons: BTreeMap<String, String>,
    #[serde(default)]
    pub expected_failures: Vec<PolicyRecord>,
    #[serde(default)]
    pub waivers: Vec<PolicyRecord>,
    #[serde(default)]
    pub blacklists: Vec<PolicyRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HarnessReport {
    pub schema_version: String,
    pub candidate: CandidateId,
    pub disposition: ResultDisposition,
    pub disposition_reason: Option<String>,
    pub effects: Vec<RecordedEffect>,
    pub failpoints_reached: Vec<Failpoint>,
    pub crashes: Vec<CrashRecord>,
    pub response_losses: Vec<ResponseLossRecord>,
    pub capabilities: BTreeMap<String, bool>,
    pub capability_dispositions: BTreeMap<String, ResultDisposition>,
    pub capability_disposition_reasons: BTreeMap<String, String>,
    pub expected_failures: Vec<PolicyRecord>,
    pub waivers: Vec<PolicyRecord>,
    pub blacklists: Vec<PolicyRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactEffect {
    pub sequence: u64,
    pub at_ms: u64,
    pub effect_id: String,
    pub kind: String,
    pub semantic_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactIdentity {
    pub name: String,
    pub version: String,
    pub stable_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyGraphEvidence {
    pub sha256: String,
    pub artifact: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationBindings {
    pub dependency_graph: DependencyGraphEvidence,
    pub tested_source_baseline: String,
    pub component_commits: BTreeMap<String, String>,
    pub scenario_hash: String,
    pub vector_hashes: BTreeMap<String, String>,
    pub evidence_hashes: BTreeMap<String, String>,
    pub tier: String,
    pub homeservers: Vec<ArtifactIdentity>,
    pub clients: Vec<ArtifactIdentity>,
}

/// Schema-conforming publication model. Construction is explicit so a trace
/// cannot accidentally masquerade as a fully evidenced result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReport {
    pub schema_version: String,
    pub candidate: ArtifactIdentity,
    pub dependency_graph: DependencyGraphEvidence,
    pub harness_commit: String,
    pub component_commits: BTreeMap<String, String>,
    pub scenario_hash: String,
    pub vector_hashes: BTreeMap<String, String>,
    pub tier: String,
    pub homeservers: Vec<ArtifactIdentity>,
    pub clients: Vec<ArtifactIdentity>,
    pub disposition: ResultDisposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disposition_reason: Option<String>,
    pub effects: Vec<ArtifactEffect>,
    pub failpoints_reached: Vec<String>,
    #[serde(default)]
    pub crashes: Vec<CrashRecord>,
    #[serde(default)]
    pub response_losses: Vec<ResponseLossRecord>,
    pub evidence_hashes: BTreeMap<String, String>,
    #[serde(default)]
    pub capabilities: BTreeMap<String, bool>,
    #[serde(default)]
    pub capability_dispositions: BTreeMap<String, ResultDisposition>,
    #[serde(default)]
    pub capability_disposition_reasons: BTreeMap<String, String>,
    #[serde(default)]
    pub expected_failures: Vec<PolicyRecord>,
    #[serde(default)]
    pub waivers: Vec<PolicyRecord>,
    #[serde(default)]
    pub blacklists: Vec<PolicyRecord>,
}

impl ArtifactReport {
    pub fn validate_publication_bindings(
        &self,
        expected: &PublicationBindings,
    ) -> Result<(), HarnessError> {
        if self.dependency_graph != expected.dependency_graph
            || self.harness_commit != expected.tested_source_baseline
            || self.component_commits.get("ironclaw_source_baseline")
                != Some(&expected.tested_source_baseline)
            || self.component_commits != expected.component_commits
            || self.scenario_hash != expected.scenario_hash
            || self.vector_hashes != expected.vector_hashes
            || self.evidence_hashes != expected.evidence_hashes
            || self.tier != expected.tier
            || self.homeservers != expected.homeservers
            || self.clients != expected.clients
        {
            return Err(HarnessError::ArtifactPublicationMismatch);
        }
        Ok(())
    }

    pub fn apply_execution(&mut self, execution: &HarnessReport) -> Result<(), HarnessError> {
        self.candidate = ArtifactIdentity {
            name: execution.candidate.name.clone(),
            version: execution.candidate.version.clone(),
            stable_id: execution.candidate.stable_id.as_str().to_owned(),
        };
        self.disposition = execution.disposition;
        self.disposition_reason = execution.disposition_reason.clone();
        self.effects = execution
            .effects
            .iter()
            .map(ArtifactEffect::from_recorded)
            .collect::<Result<_, _>>()?;
        self.failpoints_reached = execution
            .failpoints_reached
            .iter()
            .map(|value| serde_json::to_value(value).and_then(serde_json::from_value))
            .collect::<Result<_, _>>()
            .map_err(|_| HarnessError::ArtifactEncoding)?;
        self.crashes = execution.crashes.clone();
        self.response_losses = execution.response_losses.clone();
        self.capabilities = execution.capabilities.clone();
        self.capability_dispositions = execution.capability_dispositions.clone();
        self.capability_disposition_reasons = execution.capability_disposition_reasons.clone();
        self.expected_failures = execution.expected_failures.clone();
        self.waivers = execution.waivers.clone();
        self.blacklists = execution.blacklists.clone();
        Ok(())
    }

    /// Reject a publication whose disposition or effect identities were not
    /// derived from the completed execution trace.
    pub fn validate_against_execution(
        &self,
        execution: &HarnessReport,
    ) -> Result<(), HarnessError> {
        if self.disposition != execution.disposition
            || self.candidate.name != execution.candidate.name
            || self.candidate.version != execution.candidate.version
            || self.candidate.stable_id != execution.candidate.stable_id.as_str()
            || self.effects.len() != execution.effects.len()
            || self.disposition_reason != execution.disposition_reason
            || self.capabilities != execution.capabilities
            || self.capability_dispositions != execution.capability_dispositions
            || self.capability_disposition_reasons != execution.capability_disposition_reasons
            || self.expected_failures != execution.expected_failures
            || self.waivers != execution.waivers
            || self.blacklists != execution.blacklists
            || self.failpoints_reached
                != execution
                    .failpoints_reached
                    .iter()
                    .map(|value| serde_json::to_value(value).and_then(serde_json::from_value))
                    .collect::<Result<Vec<String>, _>>()
                    .map_err(|_| HarnessError::ArtifactEncoding)?
            || self.crashes != execution.crashes
            || self.response_losses != execution.response_losses
            || self
                .effects
                .iter()
                .zip(&execution.effects)
                .any(|(artifact, trace)| {
                    artifact.sequence != trace.sequence
                        || artifact.at_ms != trace.at_ms
                        || artifact.effect_id != trace.intent.effect_id.as_str()
                        || ArtifactEffect::from_recorded(trace)
                            .map_or(true, |derived| derived != *artifact)
                })
        {
            return Err(HarnessError::ArtifactExecutionMismatch);
        }
        Ok(())
    }
}

impl ArtifactEffect {
    fn from_recorded(recorded: &RecordedEffect) -> Result<Self, HarnessError> {
        let encoded = serde_json::to_vec(&recorded.intent.kind)
            .map_err(|_| HarnessError::ArtifactEncoding)?;
        let kind = match recorded.intent.kind {
            EffectKind::MatrixRequest { .. } => "matrix_request",
            EffectKind::CryptoStoreWrite { .. } => "crypto_store_write",
            EffectKind::IngressDisposition { .. } => "ingress_disposition",
            EffectKind::PreparedCiphertext { .. } => "prepared_ciphertext",
            EffectKind::Observation { .. } => "observation",
            EffectKind::Cancellation { .. } => "cancellation",
            EffectKind::StaleWriterRejected { .. } => "stale_writer_rejected",
        };
        Ok(Self {
            sequence: recorded.sequence,
            at_ms: recorded.at_ms,
            effect_id: recorded.intent.effect_id.as_str().to_owned(),
            kind: kind.to_owned(),
            semantic_digest: StableId::derive("effect-semantic", &[&encoded])
                .as_str()
                .to_owned(),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub schema_version: String,
    pub scenario_id: StableId,
    pub name: String,
    pub inputs: Vec<MatrixInput>,
    pub expected_effects: Vec<EffectIntent>,
    pub failpoints: Vec<Failpoint>,
}

impl Scenario {
    pub fn derived_id(&self) -> Result<StableId, HarnessError> {
        fn canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, HarnessError> {
            fn sort(value: serde_json::Value) -> serde_json::Value {
                match value {
                    serde_json::Value::Array(values) => {
                        serde_json::Value::Array(values.into_iter().map(sort).collect())
                    }
                    serde_json::Value::Object(values) => {
                        let sorted: BTreeMap<_, _> = values
                            .into_iter()
                            .map(|(key, value)| (key, sort(value)))
                            .collect();
                        serde_json::Value::Object(sorted.into_iter().collect())
                    }
                    other => other,
                }
            }
            let value = serde_json::to_value(value).map_err(|_| HarnessError::ArtifactEncoding)?;
            serde_json::to_vec(&sort(value)).map_err(|_| HarnessError::ArtifactEncoding)
        }
        let inputs = canonical(&self.inputs)?;
        let expected_effects = canonical(&self.expected_effects)?;
        let failpoints = canonical(&self.failpoints)?;
        Ok(StableId::derive(
            "scenario",
            &[
                self.schema_version.as_bytes(),
                self.name.as_bytes(),
                &inputs,
                &expected_effects,
                &failpoints,
            ],
        ))
    }

    pub fn validate_identity(&self) -> Result<(), HarnessError> {
        if self.scenario_id != self.derived_id()? {
            return Err(HarnessError::ScenarioIdentityMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestVector {
    pub schema_version: String,
    pub vector_id: StableId,
    pub purpose: String,
    pub method: String,
    pub path: String,
    pub credential_free_body: Vec<u8>,
}

impl RequestVector {
    pub fn derived_id(&self) -> StableId {
        StableId::derive(
            "request-vector",
            &[
                self.purpose.as_bytes(),
                self.method.as_bytes(),
                self.path.as_bytes(),
                &self.credential_free_body,
            ],
        )
    }

    pub fn validate_identity(&self) -> Result<(), HarnessError> {
        let expected = self.derived_id();
        if expected != self.vector_id {
            return Err(HarnessError::RequestVectorIdentityMismatch {
                expected,
                actual: self.vector_id.clone(),
            });
        }
        Ok(())
    }
}

pub fn validate_matrix_identifier(value: &str) -> Result<(), HarnessError> {
    validate_matrix_identifier_bytes(value.as_bytes())
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixIdentifierKind {
    UserId,
    RoomId,
    DeviceId,
    SessionId,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionTrigger {
    DirectChat,
    BotMention,
    ReplyToBot,
    Ambient,
}

pub fn validate_typed_matrix_identifier_bytes(
    kind: MatrixIdentifierKind,
    value: &[u8],
) -> Result<(), HarnessError> {
    validate_matrix_identifier_bytes(value)?;
    let decoded = std::str::from_utf8(value).map_err(|_| HarnessError::InvalidIdentifier)?;
    match kind {
        MatrixIdentifierKind::UserId => validate_matrix_sigil_id(decoded, '@'),
        MatrixIdentifierKind::RoomId => validate_matrix_sigil_id(decoded, '!'),
        MatrixIdentifierKind::DeviceId | MatrixIdentifierKind::SessionId => Ok(()),
    }
}

pub fn admit_fixture_message(
    actor: Option<&[u8]>,
    conversation_kind: MatrixIdentifierKind,
    conversation: &[u8],
    trigger: AdmissionTrigger,
) -> Result<bool, HarnessError> {
    validate_typed_matrix_identifier_bytes(conversation_kind, conversation)?;
    let actor = actor.ok_or(HarnessError::MissingActor)?;
    validate_typed_matrix_identifier_bytes(MatrixIdentifierKind::UserId, actor)?;
    let topology_valid = match trigger {
        AdmissionTrigger::DirectChat => conversation_kind == MatrixIdentifierKind::RoomId,
        AdmissionTrigger::BotMention | AdmissionTrigger::ReplyToBot | AdmissionTrigger::Ambient => {
            conversation_kind == MatrixIdentifierKind::RoomId
        }
    };
    if !topology_valid {
        return Err(HarnessError::InvalidAdmissionTopology);
    }
    Ok(!matches!(trigger, AdmissionTrigger::Ambient))
}

fn validate_matrix_sigil_id(value: &str, sigil: char) -> Result<(), HarnessError> {
    let Some(rest) = value.strip_prefix(sigil) else {
        return Err(HarnessError::InvalidIdentifier);
    };
    let Some((local, server)) = rest.split_once(':') else {
        return Err(HarnessError::InvalidIdentifier);
    };
    if local.is_empty() || server.is_empty() || local.chars().any(char::is_whitespace) {
        return Err(HarnessError::InvalidIdentifier);
    }
    let valid_server = if let Some(ipv6) = server.strip_prefix('[') {
        ipv6.split_once(']').is_some_and(|(address, suffix)| {
            !address.is_empty()
                && address.contains(':')
                && (suffix.is_empty()
                    || suffix.strip_prefix(':').is_some_and(|port| {
                        !port.is_empty() && port.chars().all(|c| c.is_ascii_digit())
                    }))
        })
    } else {
        let (host, port) = server
            .rsplit_once(':')
            .map_or((server, None), |(host, port)| (host, Some(port)));
        !host.is_empty()
            && !host.chars().any(char::is_whitespace)
            && port.is_none_or(|port| !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()))
    };
    if valid_server {
        Ok(())
    } else {
        Err(HarnessError::InvalidIdentifier)
    }
}

/// Validate external bytes before allocating a protocol identifier string.
pub fn validate_matrix_identifier_bytes(value: &[u8]) -> Result<(), HarnessError> {
    if value.is_empty() || value.iter().any(|byte| byte.is_ascii_control()) {
        return Err(HarnessError::InvalidIdentifier);
    }
    if value.len() > 255 {
        return Err(HarnessError::IdentifierTooLong { bytes: value.len() });
    }
    std::str::from_utf8(value).map_err(|_| HarnessError::InvalidIdentifier)?;
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub struct StatefulResponder {
    sync_releases: VecDeque<Vec<u8>>,
    device_keys: BTreeMap<(String, Option<String>), Vec<u8>>,
    cross_signing_keys: BTreeMap<String, Vec<u8>>,
    one_time_keys: BTreeMap<(String, String, String), VecDeque<Vec<u8>>>,
    to_device: BTreeMap<String, VecDeque<Vec<u8>>>,
    captured_requests: Vec<RequestVector>,
}

impl StatefulResponder {
    pub fn respond(&mut self, request: &RequestVector) -> Result<Vec<u8>, HarnessError> {
        self.capture_request(request.clone());
        match request.purpose.as_str() {
            "sync" => self
                .release_sync()
                .ok_or(HarnessError::ResponderUnavailable),
            "keys_query" => {
                let body: serde_json::Value = serde_json::from_slice(&request.credential_free_body)
                    .map_err(|_| HarnessError::ResponderRequestInvalid)?;
                let (user_id, devices) = body
                    .get("device_keys")
                    .and_then(serde_json::Value::as_object)
                    .filter(|users| users.len() == 1)
                    .and_then(|users| users.iter().next())
                    .ok_or(HarnessError::ResponderRequestInvalid)?;
                let devices = devices
                    .as_array()
                    .ok_or(HarnessError::ResponderRequestInvalid)?;
                let device_id = match devices.as_slice() {
                    [] => None,
                    [device] => Some(
                        device
                            .as_str()
                            .ok_or(HarnessError::ResponderRequestInvalid)?
                            .to_owned(),
                    ),
                    _ => return Err(HarnessError::ResponderRequestInvalid),
                };
                self.device_keys
                    .get(&(user_id.clone(), device_id))
                    .cloned()
                    .ok_or(HarnessError::ResponderUnavailable)
            }
            "keys_claim" => {
                let body: serde_json::Value = serde_json::from_slice(&request.credential_free_body)
                    .map_err(|_| HarnessError::ResponderRequestInvalid)?;
                let users = body
                    .get("one_time_keys")
                    .and_then(serde_json::Value::as_object)
                    .filter(|users| users.len() == 1)
                    .ok_or(HarnessError::ResponderRequestInvalid)?;
                let (user_id, devices) = users
                    .iter()
                    .next()
                    .ok_or(HarnessError::ResponderRequestInvalid)?;
                let devices = devices
                    .as_object()
                    .filter(|devices| devices.len() == 1)
                    .ok_or(HarnessError::ResponderRequestInvalid)?;
                let (device_id, algorithm) = devices
                    .iter()
                    .next()
                    .ok_or(HarnessError::ResponderRequestInvalid)?;
                let algorithm = algorithm
                    .as_str()
                    .ok_or(HarnessError::ResponderRequestInvalid)?;
                self.claim_one_time_key(user_id, device_id, algorithm)
                    .ok_or(HarnessError::ResponderUnavailable)
            }
            "to_device" => {
                self.send_to_device("captured".into(), request.credential_free_body.clone());
                Ok(b"{}".to_vec())
            }
            "cross_signing" | "signatures_upload" => {
                let body: serde_json::Value = serde_json::from_slice(&request.credential_free_body)
                    .map_err(|_| HarnessError::ResponderRequestInvalid)?;
                let user_id = ["master_key", "self_signing_key", "user_signing_key"]
                    .iter()
                    .find_map(|key| body.get(key)?.get("user_id")?.as_str())
                    .or_else(|| {
                        body.get("signatures")?.as_object().and_then(|users| {
                            (users.len() == 1)
                                .then(|| users.keys().next())
                                .flatten()
                                .map(String::as_str)
                        })
                    })
                    .ok_or(HarnessError::ResponderRequestInvalid)?;
                self.cross_signing_keys
                    .get(user_id)
                    .cloned()
                    .ok_or(HarnessError::ResponderUnavailable)
            }
            _ => Err(HarnessError::ResponderUnsupported {
                purpose: request.purpose.clone(),
            }),
        }
    }
    pub fn queue_sync_release(&mut self, body: Vec<u8>) {
        self.sync_releases.push_back(body);
    }

    pub fn release_sync(&mut self) -> Option<Vec<u8>> {
        self.sync_releases.pop_front()
    }

    pub fn set_device_keys(&mut self, user_id: String, body: Vec<u8>) {
        self.device_keys.insert((user_id, None), body);
    }

    pub fn set_device_key(&mut self, user_id: String, device_id: String, body: Vec<u8>) {
        self.device_keys.insert((user_id, Some(device_id)), body);
    }

    pub fn query_device_keys(&self, user_id: &str) -> Option<&[u8]> {
        self.device_keys
            .get(&(user_id.to_owned(), None))
            .map(Vec::as_slice)
    }

    pub fn set_cross_signing_keys(&mut self, user_id: String, body: Vec<u8>) {
        self.cross_signing_keys.insert(user_id, body);
    }

    pub fn query_cross_signing_keys(&self, user_id: &str) -> Option<&[u8]> {
        self.cross_signing_keys.get(user_id).map(Vec::as_slice)
    }

    pub fn add_one_time_key(
        &mut self,
        user_id: String,
        device_id: String,
        algorithm: String,
        key: Vec<u8>,
    ) {
        self.one_time_keys
            .entry((user_id, device_id, algorithm))
            .or_default()
            .push_back(key);
    }

    pub fn claim_one_time_key(
        &mut self,
        user_id: &str,
        device_id: &str,
        algorithm: &str,
    ) -> Option<Vec<u8>> {
        self.one_time_keys
            .get_mut(&(
                user_id.to_owned(),
                device_id.to_owned(),
                algorithm.to_owned(),
            ))
            .and_then(VecDeque::pop_front)
    }

    pub fn send_to_device(&mut self, device_id: String, body: Vec<u8>) {
        self.to_device.entry(device_id).or_default().push_back(body);
    }

    pub fn receive_to_device(&mut self, device_id: &str) -> Option<Vec<u8>> {
        self.to_device
            .get_mut(device_id)
            .and_then(VecDeque::pop_front)
    }

    pub fn capture_request(&mut self, request: RequestVector) {
        self.captured_requests.push(request);
    }

    pub fn captured_requests(&self) -> &[RequestVector] {
        &self.captured_requests
    }
}

pub struct Harness<A: CandidateAdapter> {
    adapter: A,
    clock: VirtualClock,
    expected_effects: VecDeque<EffectIntent>,
    scripted_responses: VecDeque<ScriptedResponse>,
    ledger: Vec<RecordedEffect>,
    armed_failpoint: Option<Failpoint>,
    failpoints_reached: Vec<Failpoint>,
    crashes: Vec<CrashRecord>,
    response_losses: Vec<ResponseLossRecord>,
    required_failpoints: BTreeSet<Failpoint>,
    run_plan: RunPlan,
    terminal_errors: Vec<(String, bool)>,
    execution_started: bool,
}

impl<A: CandidateAdapter> Harness<A> {
    pub fn from_scenario(
        adapter: A,
        scenario: &Scenario,
        scripted_responses: Vec<ScriptedResponse>,
    ) -> Result<Self, HarnessError> {
        scenario.validate_identity()?;
        Self::new(
            adapter,
            scenario.expected_effects.clone(),
            scripted_responses,
        )
        .with_required_failpoints(scenario.failpoints.clone())
    }

    pub fn new(
        adapter: A,
        expected_effects: Vec<EffectIntent>,
        scripted_responses: Vec<ScriptedResponse>,
    ) -> Self {
        Self {
            adapter,
            clock: VirtualClock::default(),
            expected_effects: expected_effects.into(),
            scripted_responses: scripted_responses.into(),
            ledger: Vec::new(),
            armed_failpoint: None,
            failpoints_reached: Vec::new(),
            crashes: Vec::new(),
            response_losses: Vec::new(),
            required_failpoints: BTreeSet::new(),
            run_plan: RunPlan::default(),
            terminal_errors: Vec::new(),
            execution_started: false,
        }
    }

    pub fn with_run_plan(mut self, run_plan: RunPlan) -> Result<Self, HarnessError> {
        if self.execution_started {
            return Err(HarnessError::RunPlanAfterExecutionStarted);
        }
        if !matches!(
            run_plan.predeclared_disposition,
            None | Some(ResultDisposition::Infeasible | ResultDisposition::NotApplicable)
        ) {
            return Err(HarnessError::InvalidPredeclaredDisposition);
        }
        if run_plan.predeclared_disposition.is_some()
            && run_plan
                .disposition_reason
                .as_deref()
                .is_none_or(str::is_empty)
        {
            return Err(HarnessError::MissingDispositionReason);
        }
        for (capability, disposition) in &run_plan.capability_dispositions {
            if *disposition == ResultDisposition::NotApplicable
                && run_plan
                    .capability_disposition_reasons
                    .get(capability)
                    .is_none_or(String::is_empty)
            {
                return Err(HarnessError::MissingCapabilityDispositionReason {
                    capability: capability.clone(),
                });
            }
        }
        self.run_plan = run_plan;
        Ok(self)
    }

    pub fn adapter(&self) -> &A {
        &self.adapter
    }

    pub fn clock_mut(&mut self) -> &mut VirtualClock {
        &mut self.clock
    }

    pub fn arm_failpoint(&mut self, failpoint: Failpoint) {
        self.armed_failpoint = Some(failpoint);
    }

    pub fn with_required_failpoints(
        mut self,
        failpoints: Vec<Failpoint>,
    ) -> Result<Self, HarnessError> {
        if self.execution_started {
            return Err(HarnessError::RequiredFailpointsAfterExecutionStarted);
        }
        self.required_failpoints = failpoints.into_iter().collect();
        Ok(self)
    }

    pub fn drive(&mut self, input: MatrixInput) -> Result<(), HarnessError> {
        self.execution_started = true;
        let result = self.drive_inner(input);
        if let Err(error) = &result {
            let uncertain = match error {
                HarnessError::InjectedFailpoint { failpoint } => failpoint.is_uncertain(),
                HarnessError::ResponseLostAfterAdapterHandling => true,
                _ => false,
            };
            self.terminal_errors.push((error.to_string(), uncertain));
        }
        result
    }

    fn drive_inner(&mut self, input: MatrixInput) -> Result<(), HarnessError> {
        if matches!(input, MatrixInput::ConsumeNextResponse) {
            let response = self
                .scripted_responses
                .pop_front()
                .ok_or(HarnessError::MissingScriptedResponse)?;
            if response.delivery == ResponseDelivery::LostBeforeCandidate {
                self.response_losses.push(ResponseLossRecord {
                    at_ms: self.clock.now_ms(),
                    phase: ResponseLossPhase::BeforeCandidate,
                });
                return Err(HarnessError::ResponseLostBeforeCandidate);
            }
            if self.armed_failpoint == Some(Failpoint::ResponseBeforePrepare) {
                return self.trigger(Failpoint::ResponseBeforePrepare);
            }
            let lost_after = response.delivery == ResponseDelivery::LostAfterAdapterHandling;
            let response = if response.delivery == ResponseDelivery::Partial {
                let prefix = response
                    .partial_bytes
                    .ok_or(HarnessError::InvalidPartialResponse)?;
                if prefix >= response.body.len() {
                    return Err(HarnessError::InvalidPartialResponse);
                }
                ScriptedResponse {
                    body: response.body[..prefix].to_vec(),
                    delivery: ResponseDelivery::Partial,
                    partial_bytes: Some(prefix),
                    ..response
                }
            } else {
                if response.partial_bytes.is_some() {
                    return Err(HarnessError::InvalidPartialResponse);
                }
                response
            };
            let prepared = self.adapter.prepare_response(response)?;
            let effects = self.adapter.commit_response(&prepared)?;
            self.append_effects(effects)?;
            if self.armed_failpoint == Some(Failpoint::ResponseAfterCommitBeforeAcknowledge) {
                return self.trigger(Failpoint::ResponseAfterCommitBeforeAcknowledge);
            }
            if lost_after {
                self.response_losses.push(ResponseLossRecord {
                    at_ms: self.clock.now_ms(),
                    phase: ResponseLossPhase::AfterAdapterHandling,
                });
                return Err(HarnessError::ResponseLostAfterAdapterHandling);
            }
            return self.adapter.acknowledge_response(prepared);
        }
        if self.armed_failpoint == Some(Failpoint::CandidateBeforePrepare) {
            return self.trigger(Failpoint::CandidateBeforePrepare);
        }
        let prepared = self.adapter.prepare(input)?;
        if self.armed_failpoint == Some(Failpoint::CandidateAfterPrepare) {
            return self.trigger(Failpoint::CandidateAfterPrepare);
        }
        let effects = self.adapter.commit(&prepared)?;
        self.append_effects(effects)?;
        if self.armed_failpoint == Some(Failpoint::CandidateAfterCommitBeforeAcknowledge) {
            return self.trigger(Failpoint::CandidateAfterCommitBeforeAcknowledge);
        }
        self.adapter.acknowledge(prepared)
    }

    pub fn crash(&mut self) -> Result<CrashRecord, HarnessError> {
        self.execution_started = true;
        self.adapter.crash(None)?;
        let record = CrashRecord {
            at_ms: self.clock.now_ms(),
            reached: None,
        };
        self.crashes.push(record.clone());
        Ok(record)
    }

    pub fn restart(&mut self, reason: RestartReason) -> Result<(), HarnessError> {
        self.execution_started = true;
        self.adapter.restart(reason)
    }

    pub fn finish(self) -> Result<HarnessReport, HarnessError> {
        let mut terminal_failures = self.terminal_errors;
        if !self.expected_effects.is_empty() {
            terminal_failures.push((
                HarnessError::MissingEffects {
                    count: self.expected_effects.len(),
                }
                .to_string(),
                false,
            ));
        }
        if !self.scripted_responses.is_empty() {
            terminal_failures.push((
                HarnessError::UnusedScriptedResponses {
                    count: self.scripted_responses.len(),
                }
                .to_string(),
                false,
            ));
        }
        if !self.required_failpoints.is_empty() {
            terminal_failures.push((
                HarnessError::UnreachedFailpoints {
                    count: self.required_failpoints.len(),
                }
                .to_string(),
                false,
            ));
        }
        if let Some(failpoint) = self.armed_failpoint {
            terminal_failures.push((
                HarnessError::ArmedFailpointUnreached { failpoint }.to_string(),
                false,
            ));
        }
        let disposition = if terminal_failures.iter().all(|(_, uncertain)| *uncertain)
            && !terminal_failures.is_empty()
        {
            ResultDisposition::Uncertain
        } else if !terminal_failures.is_empty() {
            ResultDisposition::Failed
        } else {
            self.run_plan
                .predeclared_disposition
                .unwrap_or(ResultDisposition::Supported)
        };
        let disposition_reason = if terminal_failures.is_empty() {
            self.run_plan.disposition_reason.clone()
        } else {
            Some(
                terminal_failures
                    .into_iter()
                    .map(|(reason, _)| reason)
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        };
        Ok(HarnessReport {
            schema_version: TRACE_VERSION.into(),
            candidate: self.adapter.candidate().clone(),
            disposition,
            disposition_reason,
            effects: self.ledger,
            failpoints_reached: self.failpoints_reached,
            crashes: self.crashes,
            response_losses: self.response_losses,
            capabilities: self.run_plan.capabilities,
            capability_dispositions: self.run_plan.capability_dispositions,
            capability_disposition_reasons: self.run_plan.capability_disposition_reasons,
            expected_failures: self.run_plan.expected_failures,
            waivers: self.run_plan.waivers,
            blacklists: self.run_plan.blacklists,
        })
    }

    fn append_effects(&mut self, effects: Vec<EffectIntent>) -> Result<(), HarnessError> {
        for (offset, actual) in effects.iter().enumerate() {
            let Some(expected) = self.expected_effects.get(offset) else {
                return Err(HarnessError::UnexpectedEffect {
                    actual: actual.effect_id.clone(),
                });
            };
            if expected != actual {
                return Err(HarnessError::ReorderedEffect {
                    expected: expected.effect_id.clone(),
                    actual: actual.effect_id.clone(),
                });
            }
        }
        for actual in effects {
            if self.armed_failpoint == Some(Failpoint::ProcessBeforeEffectAppend) {
                return self.trigger(Failpoint::ProcessBeforeEffectAppend);
            }
            if let Some(before) = boundary_failpoint(&actual.kind, true)
                && self.armed_failpoint == Some(before)
            {
                return self.trigger(before);
            }
            self.expected_effects.pop_front();
            let sequence =
                u64::try_from(self.ledger.len()).map_err(|_| HarnessError::SequenceOverflow)?;
            self.ledger.push(RecordedEffect {
                sequence,
                at_ms: self.clock.now_ms(),
                intent: actual,
            });
            let kind = &self
                .ledger
                .last()
                .ok_or(HarnessError::SequenceOverflow)?
                .intent
                .kind;
            let after = boundary_failpoint(kind, false);
            let unknown = match kind {
                EffectKind::CryptoStoreWrite { .. } => {
                    Some(Failpoint::LedgerUnknownCryptoStoreAppend)
                }
                EffectKind::IngressDisposition { .. } => {
                    Some(Failpoint::LedgerUnknownIngressAppend)
                }
                _ => None,
            };
            if self.armed_failpoint == unknown && unknown.is_some() {
                return self.trigger(unknown.ok_or(HarnessError::SequenceOverflow)?);
            }
            if let Some(after) = after
                && self.armed_failpoint == Some(after)
            {
                return self.trigger(after);
            }
            if self.armed_failpoint == Some(Failpoint::ProcessAfterEffectAppend) {
                return self.trigger(Failpoint::ProcessAfterEffectAppend);
            }
        }
        Ok(())
    }

    fn trigger(&mut self, failpoint: Failpoint) -> Result<(), HarnessError> {
        if self.armed_failpoint == Some(failpoint) {
            self.armed_failpoint = None;
        }
        self.record_failpoint(failpoint);
        self.adapter.crash(Some(failpoint))?;
        self.crashes.push(CrashRecord {
            at_ms: self.clock.now_ms(),
            reached: Some(failpoint),
        });
        Err(HarnessError::InjectedFailpoint { failpoint })
    }

    fn record_failpoint(&mut self, failpoint: Failpoint) {
        self.required_failpoints.remove(&failpoint);
        if !self.failpoints_reached.contains(&failpoint) {
            self.failpoints_reached.push(failpoint);
        }
    }
}

fn boundary_failpoint(kind: &EffectKind, before: bool) -> Option<Failpoint> {
    match (kind, before) {
        (EffectKind::CryptoStoreWrite { .. }, true) => {
            Some(Failpoint::LedgerBeforeCryptoStoreAppend)
        }
        (EffectKind::CryptoStoreWrite { .. }, false) => {
            Some(Failpoint::LedgerAfterCryptoStoreAppend)
        }
        (EffectKind::IngressDisposition { .. }, true) => Some(Failpoint::LedgerBeforeIngressAppend),
        (EffectKind::IngressDisposition { .. }, false) => Some(Failpoint::LedgerAfterIngressAppend),
        (EffectKind::PreparedCiphertext { .. }, true) => {
            Some(Failpoint::BeforePreparedCiphertextHandoff)
        }
        (EffectKind::PreparedCiphertext { .. }, false) => {
            Some(Failpoint::AfterPreparedCiphertextHandoff)
        }
        (EffectKind::StaleWriterRejected { .. }, false) => {
            Some(Failpoint::StaleWriterAfterTakeover)
        }
        (EffectKind::StaleWriterRejected { .. }, true) => None,
        _ => None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HarnessError {
    ClockOverflow,
    SequenceOverflow,
    MissingScriptedResponse,
    UnusedScriptedResponses {
        count: usize,
    },
    MissingEffects {
        count: usize,
    },
    UnexpectedEffect {
        actual: StableId,
    },
    ReorderedEffect {
        expected: StableId,
        actual: StableId,
    },
    ControlInputUnsupported {
        input: String,
    },
    IdentifierTooLong {
        bytes: usize,
    },
    InvalidIdentifier,
    InvalidAdmissionTopology,
    InvalidPartialResponse,
    OperationAlreadyActive {
        operation_id: StableId,
    },
    OperationNotActive {
        operation_id: StableId,
    },
    ResponderUnavailable,
    ResponderRequestInvalid,
    ResponderUnsupported {
        purpose: String,
    },
    RequestVectorIdentityMismatch {
        expected: StableId,
        actual: StableId,
    },
    ArtifactExecutionMismatch,
    ArtifactPublicationMismatch,
    ArtifactEncoding,
    MissingDispositionReason,
    MissingCapabilityDispositionReason {
        capability: String,
    },
    InvalidPredeclaredDisposition,
    RunPlanAfterExecutionStarted,
    RequiredFailpointsAfterExecutionStarted,
    MissingActor,
    InjectedFailpoint {
        failpoint: Failpoint,
    },
    ResponseLostBeforeCandidate,
    ResponseLostAfterAdapterHandling,
    UnreachedFailpoints {
        count: usize,
    },
    ArmedFailpointUnreached {
        failpoint: Failpoint,
    },
    ScenarioIdentityMismatch,
}

impl Display for HarnessError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for HarnessError {}
