use ironclaw_first_party_extension_ports::{
    SkillActivationMode as FirstPartySkillActivationMode, SkillActivationPlan,
    SkillActivationRequest as FirstPartySkillActivationRequest,
    SkillBundleAsset as FirstPartySkillBundleAsset, SkillBundleAssetReadError, SkillExecutionPlan,
};
use ironclaw_loop_support::{SkillBundleId, SkillBundleSource, SkillSourceKind};
use ironclaw_turns::run_profile::LoopRunContext;

use super::{AssistantReply, RebornRuntimeError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornSkillExecutionPlan {
    activations: Vec<RebornSkillActivation>,
    rewritten_message: String,
    feedback: Vec<String>,
    active_bundles: Vec<RebornSkillBundle>,
    first_party_plan: SkillActivationPlan,
    run_context: LoopRunContext,
}

impl RebornSkillExecutionPlan {
    pub fn activations(&self) -> &[RebornSkillActivation] {
        &self.activations
    }

    pub fn rewritten_message(&self) -> &str {
        &self.rewritten_message
    }

    pub fn feedback(&self) -> &[String] {
        &self.feedback
    }

    pub fn active_bundles(&self) -> &[RebornSkillBundle] {
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
pub struct RebornSkillExecutionResult {
    pub plan: RebornSkillExecutionPlan,
    pub reply: AssistantReply,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornSkillActivation {
    pub name: String,
    pub source: Option<RebornSkillSourceKind>,
    pub mode: RebornSkillActivationMode,
    bundle_id: Option<SkillBundleId>,
}

impl RebornSkillActivation {
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
pub struct RebornSkillBundle {
    pub source: RebornSkillSourceKind,
    pub skill_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RebornSkillSourceKind {
    System,
    TenantShared,
    User,
}

impl From<SkillSourceKind> for RebornSkillSourceKind {
    fn from(value: SkillSourceKind) -> Self {
        match value {
            SkillSourceKind::System => Self::System,
            SkillSourceKind::TenantShared => Self::TenantShared,
            SkillSourceKind::User => Self::User,
        }
    }
}

impl From<RebornSkillSourceKind> for SkillSourceKind {
    fn from(value: RebornSkillSourceKind) -> Self {
        match value {
            RebornSkillSourceKind::System => Self::System,
            RebornSkillSourceKind::TenantShared => Self::TenantShared,
            RebornSkillSourceKind::User => Self::User,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RebornSkillActivationMode {
    ExplicitMention,
    ActivationCriteria,
}

impl From<FirstPartySkillActivationMode> for RebornSkillActivationMode {
    fn from(value: FirstPartySkillActivationMode) -> Self {
        match value {
            FirstPartySkillActivationMode::ExplicitMention => Self::ExplicitMention,
            FirstPartySkillActivationMode::ActivationCriteria => Self::ActivationCriteria,
        }
    }
}

impl From<RebornSkillActivationMode> for FirstPartySkillActivationMode {
    fn from(value: RebornSkillActivationMode) -> Self {
        match value {
            RebornSkillActivationMode::ExplicitMention => Self::ExplicitMention,
            RebornSkillActivationMode::ActivationCriteria => Self::ActivationCriteria,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornSkillAsset {
    pub source: RebornSkillSourceKind,
    pub skill_name: String,
    pub path: String,
    pub bytes: Vec<u8>,
}

impl RebornSkillAsset {
    pub fn into_utf8(self) -> Result<String, std::string::FromUtf8Error> {
        String::from_utf8(self.bytes)
    }
}

impl RebornSkillExecutionPlan {
    pub(super) fn from_first_party<S>(value: SkillExecutionPlan<S>) -> Self
    where
        S: SkillBundleSource + ?Sized,
    {
        let first_party_plan = value.activation_plan().clone();
        let active_bundles = first_party_plan
            .activated_bundles()
            .iter()
            .map(RebornSkillBundle::from)
            .collect();
        Self {
            activations: first_party_plan
                .selection
                .activations
                .iter()
                .cloned()
                .map(RebornSkillActivation::from)
                .collect(),
            rewritten_message: first_party_plan.selection.rewritten_message.clone(),
            feedback: first_party_plan.selection.feedback.clone(),
            active_bundles,
            first_party_plan,
            run_context: value.run_context().clone(),
        }
    }
}

impl From<FirstPartySkillActivationRequest> for RebornSkillActivation {
    fn from(value: FirstPartySkillActivationRequest) -> Self {
        Self {
            name: value.name,
            source: value.source.map(RebornSkillSourceKind::from),
            mode: RebornSkillActivationMode::from(value.mode),
            bundle_id: value.bundle_id,
        }
    }
}

impl From<&SkillBundleId> for RebornSkillBundle {
    fn from(value: &SkillBundleId) -> Self {
        Self {
            source: RebornSkillSourceKind::from(value.source_kind()),
            skill_name: value.name().to_string(),
        }
    }
}

impl From<FirstPartySkillBundleAsset> for RebornSkillAsset {
    fn from(value: FirstPartySkillBundleAsset) -> Self {
        Self {
            source: RebornSkillSourceKind::from(value.bundle_id.source_kind()),
            skill_name: value.bundle_id.name().to_string(),
            path: value.path.as_str().to_string(),
            bytes: value.bytes,
        }
    }
}

pub(super) fn skill_asset_error(error: SkillBundleAssetReadError) -> RebornRuntimeError {
    RebornRuntimeError::SkillExecution(error.to_string())
}
