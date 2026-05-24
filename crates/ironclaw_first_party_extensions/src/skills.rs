//! First-party skill management capability handlers.
//!
//! Host runtime adapts already-authorized capability invocations into
//! [`SkillManagementCapabilityRequest`]; this module receives scoped mounts
//! and an explicit filesystem handle only.

use std::sync::Arc;

use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{MountView, ResourceScope, RuntimeDispatchErrorKind};
use ironclaw_skills::{
    SkillInstallRequest, SkillManagementContext, SkillManagementError, SkillManagementErrorKind,
    SkillRemoveRequest, SkillSummary, install_skill, list_skills, remove_skill,
};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillManagementCapabilityKind {
    List,
    Install,
    Remove,
}

#[derive(Clone)]
pub struct SkillManagementCapabilityRequest<'a> {
    pub(crate) kind: SkillManagementCapabilityKind,
    pub(crate) scope: &'a ResourceScope,
    pub(crate) mounts: Option<&'a MountView>,
    pub(crate) filesystem: Arc<dyn RootFilesystem>,
    pub(crate) input: &'a Value,
}

impl<'a> SkillManagementCapabilityRequest<'a> {
    pub fn new(
        kind: SkillManagementCapabilityKind,
        scope: &'a ResourceScope,
        mounts: Option<&'a MountView>,
        filesystem: Arc<dyn RootFilesystem>,
        input: &'a Value,
    ) -> Self {
        Self {
            kind,
            scope,
            mounts,
            filesystem,
            input,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("skill management capability dispatch failed: {kind}")]
pub struct SkillManagementCapabilityError {
    kind: RuntimeDispatchErrorKind,
}

impl SkillManagementCapabilityError {
    pub fn new(kind: RuntimeDispatchErrorKind) -> Self {
        Self { kind }
    }

    pub fn kind(&self) -> RuntimeDispatchErrorKind {
        self.kind
    }
}

pub async fn dispatch(
    request: &SkillManagementCapabilityRequest<'_>,
) -> Result<Value, SkillManagementCapabilityError> {
    match request.kind {
        SkillManagementCapabilityKind::List => dispatch_list(request).await,
        SkillManagementCapabilityKind::Install => dispatch_install(request).await,
        SkillManagementCapabilityKind::Remove => dispatch_remove(request).await,
    }
}

async fn dispatch_list(
    request: &SkillManagementCapabilityRequest<'_>,
) -> Result<Value, SkillManagementCapabilityError> {
    let context = management_context(request)?;
    let skills = list_skills(&context).await.map_err(capability_error)?;
    Ok(json!({
        "skills": skills.iter().map(skill_summary_json).collect::<Vec<_>>(),
        "count": skills.len(),
    }))
}

async fn dispatch_install(
    request: &SkillManagementCapabilityRequest<'_>,
) -> Result<Value, SkillManagementCapabilityError> {
    let content = request
        .input
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(input_error)?;
    let name = request.input.get("name").and_then(Value::as_str);
    let context = management_context(request)?;
    let installed = install_skill(&context, SkillInstallRequest { name, content })
        .await
        .map_err(capability_error)?;

    Ok(json!({
        "installed": true,
        "name": installed.name,
        "path": installed.scoped_path,
    }))
}

async fn dispatch_remove(
    request: &SkillManagementCapabilityRequest<'_>,
) -> Result<Value, SkillManagementCapabilityError> {
    let name = request
        .input
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(input_error)?;
    let context = management_context(request)?;
    let removed = remove_skill(&context, SkillRemoveRequest { name })
        .await
        .map_err(capability_error)?;

    Ok(json!({
        "removed": true,
        "name": removed.name,
    }))
}

fn management_context(
    request: &SkillManagementCapabilityRequest<'_>,
) -> Result<SkillManagementContext, SkillManagementCapabilityError> {
    let Some(mounts) = request.mounts else {
        return Err(SkillManagementCapabilityError::new(
            RuntimeDispatchErrorKind::FilesystemDenied,
        ));
    };
    Ok(SkillManagementContext::new(
        Arc::clone(&request.filesystem),
        mounts.clone(),
        request.scope.clone(),
    ))
}

fn skill_summary_json(skill: &SkillSummary) -> Value {
    json!({
        "name": skill.name,
        "version": skill.version,
        "description": skill.description,
        "source": skill.source.as_str(),
        "keywords": skill.keywords,
        "tags": skill.tags,
        "requires_skills": skill.requires_skills,
    })
}

fn input_error() -> SkillManagementCapabilityError {
    SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
}

fn capability_error(error: SkillManagementError) -> SkillManagementCapabilityError {
    let kind = match error.kind() {
        SkillManagementErrorKind::InvalidInput => RuntimeDispatchErrorKind::InputEncode,
        SkillManagementErrorKind::FilesystemDenied => RuntimeDispatchErrorKind::FilesystemDenied,
        SkillManagementErrorKind::NotFound
        | SkillManagementErrorKind::Conflict
        | SkillManagementErrorKind::InvalidSkill => RuntimeDispatchErrorKind::Guest,
        SkillManagementErrorKind::Resource => RuntimeDispatchErrorKind::Resource,
    };
    SkillManagementCapabilityError::new(kind)
}
