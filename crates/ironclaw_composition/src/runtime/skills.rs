use ironclaw_first_party_extension_ports::{
    SkillActivationMode as FirstPartySkillActivationMode, SkillActivationPlan,
    SkillActivationRequest as FirstPartySkillActivationRequest,
    SkillBundleAsset as FirstPartySkillBundleAsset, SkillBundleAssetReadError, SkillExecutionPlan,
};
use ironclaw_loop_host::{SkillBundleId, SkillBundleSource, SkillSourceKind};
use ironclaw_turns::run_profile::LoopRunContext;

use super::{AssistantReply, IronClawRuntimeError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IronClawSkillExecutionPlan {
    activations: Vec<IronClawSkillActivation>,
    rewritten_message: String,
    feedback: Vec<String>,
    active_bundles: Vec<IronClawSkillBundle>,
    first_party_plan: SkillActivationPlan,
    run_context: LoopRunContext,
}

impl IronClawSkillExecutionPlan {
    pub fn activations(&self) -> &[IronClawSkillActivation] {
        &self.activations
    }

    pub fn rewritten_message(&self) -> &str {
        &self.rewritten_message
    }

    pub fn feedback(&self) -> &[String] {
        &self.feedback
    }

    pub fn active_bundles(&self) -> &[IronClawSkillBundle] {
        &self.active_bundles
    }

    pub(super) fn first_party_plan(&self) -> &SkillActivationPlan {
        &self.first_party_plan
    }

    pub(super) fn run_context(&self) -> &LoopRunContext {
        &self.run_context
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IronClawSkillExecutionResult {
    pub plan: IronClawSkillExecutionPlan,
    pub reply: AssistantReply,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IronClawSkillActivation {
    pub name: String,
    pub source: Option<IronClawSkillSourceKind>,
    pub mode: IronClawSkillActivationMode,
    bundle_id: Option<SkillBundleId>,
}

impl IronClawSkillActivation {
    pub(super) fn to_first_party_request(&self) -> FirstPartySkillActivationRequest {
        FirstPartySkillActivationRequest {
            name: self.name.clone(),
            source: self.source.map(SkillSourceKind::from),
            bundle_id: self.bundle_id.clone(),
            mode: FirstPartySkillActivationMode::from(self.mode),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IronClawSkillBundle {
    pub source: IronClawSkillSourceKind,
    pub skill_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IronClawSkillSourceKind {
    System,
    TenantShared,
    User,
}

impl From<SkillSourceKind> for IronClawSkillSourceKind {
    fn from(value: SkillSourceKind) -> Self {
        match value {
            SkillSourceKind::System => Self::System,
            SkillSourceKind::TenantShared => Self::TenantShared,
            SkillSourceKind::User => Self::User,
        }
    }
}

impl From<IronClawSkillSourceKind> for SkillSourceKind {
    fn from(value: IronClawSkillSourceKind) -> Self {
        match value {
            IronClawSkillSourceKind::System => Self::System,
            IronClawSkillSourceKind::TenantShared => Self::TenantShared,
            IronClawSkillSourceKind::User => Self::User,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IronClawSkillActivationMode {
    ExplicitMention,
    ActivationCriteria,
    ModelSelected,
}

impl From<FirstPartySkillActivationMode> for IronClawSkillActivationMode {
    fn from(value: FirstPartySkillActivationMode) -> Self {
        match value {
            FirstPartySkillActivationMode::ExplicitMention => Self::ExplicitMention,
            FirstPartySkillActivationMode::ActivationCriteria => Self::ActivationCriteria,
            FirstPartySkillActivationMode::ModelSelected => Self::ModelSelected,
        }
    }
}

impl From<IronClawSkillActivationMode> for FirstPartySkillActivationMode {
    fn from(value: IronClawSkillActivationMode) -> Self {
        match value {
            IronClawSkillActivationMode::ExplicitMention => Self::ExplicitMention,
            IronClawSkillActivationMode::ActivationCriteria => Self::ActivationCriteria,
            IronClawSkillActivationMode::ModelSelected => Self::ModelSelected,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IronClawSkillAsset {
    pub source: IronClawSkillSourceKind,
    pub skill_name: String,
    pub path: String,
    pub bytes: Vec<u8>,
}

impl IronClawSkillAsset {
    pub fn into_utf8(self) -> Result<String, std::string::FromUtf8Error> {
        String::from_utf8(self.bytes)
    }
}

impl IronClawSkillExecutionPlan {
    pub(super) fn from_first_party<S>(value: SkillExecutionPlan<S>) -> Self
    where
        S: SkillBundleSource + ?Sized,
    {
        let first_party_plan = value.activation_plan().clone();
        let active_bundles = first_party_plan
            .activated_bundles()
            .iter()
            .map(IronClawSkillBundle::from)
            .collect();
        Self {
            activations: first_party_plan
                .selection
                .activations
                .iter()
                .cloned()
                .map(IronClawSkillActivation::from)
                .collect(),
            rewritten_message: first_party_plan.selection.rewritten_message.clone(),
            feedback: first_party_plan.selection.feedback.clone(),
            active_bundles,
            first_party_plan,
            run_context: value.run_context().clone(),
        }
    }
}

impl From<FirstPartySkillActivationRequest> for IronClawSkillActivation {
    fn from(value: FirstPartySkillActivationRequest) -> Self {
        Self {
            name: value.name,
            source: value.source.map(IronClawSkillSourceKind::from),
            mode: IronClawSkillActivationMode::from(value.mode),
            bundle_id: value.bundle_id,
        }
    }
}

impl From<&SkillBundleId> for IronClawSkillBundle {
    fn from(value: &SkillBundleId) -> Self {
        Self {
            source: IronClawSkillSourceKind::from(value.source_kind()),
            skill_name: value.name().to_string(),
        }
    }
}

impl From<FirstPartySkillBundleAsset> for IronClawSkillAsset {
    fn from(value: FirstPartySkillBundleAsset) -> Self {
        Self {
            source: IronClawSkillSourceKind::from(value.bundle_id.source_kind()),
            skill_name: value.bundle_id.name().to_string(),
            path: value.path.as_str().to_string(),
            bytes: value.bytes,
        }
    }
}

pub(super) fn skill_asset_error(error: SkillBundleAssetReadError) -> IronClawRuntimeError {
    IronClawRuntimeError::SkillExecution(error.to_string())
}
