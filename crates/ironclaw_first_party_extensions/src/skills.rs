//! First-party skill management capability handlers.
//!
//! Host runtime adapts already-authorized capability invocations into
//! [`SkillManagementCapabilityRequest`]; this module receives scoped mounts
//! and an explicit filesystem handle only.

use std::sync::{Arc, LazyLock};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{MountView, ResourceScope, RuntimeDispatchErrorKind};
use ironclaw_skills::{
    InstalledSkillMetadataSource, SkillContentRequest, SkillInstallFile, SkillInstallRequest,
    SkillInstallSource, SkillManagementContext, SkillManagementError, SkillManagementErrorKind,
    SkillRemoveRequest, SkillUpdateRequest, install_skill, list_skills, read_skill_content,
    remove_skill, skill_summary_json, update_skill,
};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillManagementCapabilityKind {
    List,
    Install,
    Update,
    SetAutoActivate,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedInstallFile {
    path: String,
    contents: Vec<u8>,
}

#[tracing::instrument(
    level = "debug",
    skip(request),
    fields(kind = ?request.kind, scope = ?request.scope)
)]
pub async fn dispatch(
    request: &SkillManagementCapabilityRequest<'_>,
) -> Result<Value, SkillManagementCapabilityError> {
    match request.kind {
        SkillManagementCapabilityKind::List => dispatch_list(request).await,
        SkillManagementCapabilityKind::Install => dispatch_install(request).await,
        SkillManagementCapabilityKind::Update => dispatch_update(request).await,
        SkillManagementCapabilityKind::SetAutoActivate => dispatch_set_auto_activate(request).await,
        SkillManagementCapabilityKind::Remove => dispatch_remove(request).await,
    }
}

#[tracing::instrument(level = "debug", skip(request))]
async fn dispatch_list(
    request: &SkillManagementCapabilityRequest<'_>,
) -> Result<Value, SkillManagementCapabilityError> {
    let context = management_context(request)?;
    let skills = list_skills(&context).await.map_err(capability_error)?;
    tracing::debug!(
        skill_count = skills.len(),
        "skill management list completed"
    );
    Ok(json!({
        "skills": Value::from_iter(skills.iter().map(skill_summary_json)),
        "count": skills.len(),
    }))
}

#[tracing::instrument(
    level = "debug",
    skip(request),
    fields(
        has_content = request.input.get("content").is_some(),
        has_requested_name = request.input.get("name").is_some(),
    )
)]
async fn dispatch_install(
    request: &SkillManagementCapabilityRequest<'_>,
) -> Result<Value, SkillManagementCapabilityError> {
    if request.input.get("url").is_some() {
        tracing::debug!("skill management install received unresolved url input");
        return Err(input_error());
    }
    let content = request
        .input
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            tracing::debug!("skill management install missing string content input");
            input_error()
        })?;
    validate_skill_content_safety(content)?;
    let parsed_files = parse_install_files(request.input)?;
    let files = parsed_files
        .iter()
        .map(|file| SkillInstallFile {
            relative_path: file.path.as_str(),
            contents: file.contents.as_slice(),
        })
        .collect::<Vec<_>>();
    let name = request.input.get("name").and_then(Value::as_str);
    let source = parse_install_source(request.input)?;
    let source_url = request.input.get("source_url").and_then(Value::as_str);
    let context = management_context(request)?;
    let installed = install_skill(
        &context,
        SkillInstallRequest {
            name,
            content,
            files: &files,
            source,
            source_url,
        },
    )
    .await
    .map_err(capability_error)?;
    tracing::debug!(
        skill_name = %installed.name,
        scoped_path = %installed.scoped_path,
        bundle_file_count = files.len(),
        "skill management install completed"
    );

    Ok(json!({
        "installed": true,
        "name": installed.name,
        "path": installed.scoped_path,
        "source": installed.source.as_str(),
        "files_installed": files.len(),
    }))
}

#[tracing::instrument(
    level = "debug",
    skip(request),
    fields(
        has_name = request.input.get("name").is_some(),
        has_content = request.input.get("content").is_some(),
    )
)]
async fn dispatch_update(
    request: &SkillManagementCapabilityRequest<'_>,
) -> Result<Value, SkillManagementCapabilityError> {
    let name = request
        .input
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            tracing::debug!("skill management update missing string name input");
            input_error()
        })?;
    let content = request
        .input
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            tracing::debug!("skill management update missing string content input");
            input_error()
        })?;
    reject_extra_fields(request.input, &["name", "content"])?;
    validate_skill_content_safety(content)?;
    let context = management_context(request)?;
    let updated = update_skill(&context, SkillUpdateRequest { name, content })
        .await
        .map_err(capability_error)?;
    tracing::debug!(
        skill_name = %updated.name,
        "skill management update completed"
    );

    Ok(json!({
        "updated": true,
        "name": updated.name,
    }))
}

#[tracing::instrument(
    level = "debug",
    skip(request),
    fields(
        has_name = request.input.get("name").is_some(),
        has_enabled = request.input.get("enabled").is_some(),
    )
)]
async fn dispatch_set_auto_activate(
    request: &SkillManagementCapabilityRequest<'_>,
) -> Result<Value, SkillManagementCapabilityError> {
    let name = request
        .input
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            tracing::debug!("skill management auto-activate missing string name input");
            input_error()
        })?;
    let enabled = request
        .input
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            tracing::debug!("skill management auto-activate missing boolean enabled input");
            input_error()
        })?;
    reject_extra_fields(request.input, &["name", "enabled"])?;
    let context = management_context(request)?;
    let current = read_skill_content(&context, SkillContentRequest { name })
        .await
        .map_err(capability_error)?;
    let updated_content = ironclaw_skills::set_skill_auto_activate(&current.content, enabled);
    validate_skill_content_safety(&updated_content)?;
    let updated = update_skill(
        &context,
        SkillUpdateRequest {
            name,
            content: &updated_content,
        },
    )
    .await
    .map_err(capability_error)?;
    tracing::debug!(
        skill_name = %updated.name,
        enabled,
        "skill management auto-activate update completed"
    );

    Ok(json!({
        "updated": true,
        "name": updated.name,
        "auto_activate": enabled,
    }))
}

#[tracing::instrument(
    level = "debug",
    skip(request),
    fields(has_name = request.input.get("name").is_some())
)]
async fn dispatch_remove(
    request: &SkillManagementCapabilityRequest<'_>,
) -> Result<Value, SkillManagementCapabilityError> {
    let name = request
        .input
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            tracing::debug!("skill management remove missing string name input");
            input_error()
        })?;
    reject_extra_fields(request.input, &["name"])?;
    let context = management_context(request)?;
    let removed = remove_skill(&context, SkillRemoveRequest { name })
        .await
        .map_err(capability_error)?;
    tracing::debug!(
        skill_name = %removed.name,
        "skill management remove completed"
    );

    Ok(json!({
        "removed": true,
        "name": removed.name,
    }))
}

fn reject_extra_fields(
    input: &Value,
    allowed: &[&str],
) -> Result<(), SkillManagementCapabilityError> {
    let Some(object) = input.as_object() else {
        return Err(input_error());
    };
    if object.keys().all(|key| allowed.contains(&key.as_str())) {
        Ok(())
    } else {
        Err(input_error())
    }
}

fn validate_skill_content_safety(content: &str) -> Result<(), SkillManagementCapabilityError> {
    static SKILL_CONTENT_SAFETY: LazyLock<ironclaw_safety::Sanitizer> =
        LazyLock::new(ironclaw_safety::Sanitizer::new);
    ironclaw_safety::validate_trusted_trigger_prompt(&*SKILL_CONTENT_SAFETY, content)
        .map_err(|_| SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::InputEncode))
}

fn management_context(
    request: &SkillManagementCapabilityRequest<'_>,
) -> Result<SkillManagementContext, SkillManagementCapabilityError> {
    let Some(mounts) = request.mounts else {
        tracing::debug!("skill management request missing filesystem mounts");
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

fn input_error() -> SkillManagementCapabilityError {
    SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
}

fn parse_install_files(
    input: &Value,
) -> Result<Vec<ParsedInstallFile>, SkillManagementCapabilityError> {
    let Some(files) = input.get("files") else {
        return Ok(Vec::new());
    };
    let files = files.as_array().ok_or_else(input_error)?;
    let mut parsed = Vec::with_capacity(files.len());
    for file in files {
        let path = file
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(input_error)?
            .to_string();
        let contents = if let Some(encoded) = file.get("bytes_base64") {
            let encoded = encoded.as_str().ok_or_else(input_error)?;
            BASE64_STANDARD.decode(encoded).map_err(|_| input_error())?
        } else {
            file.get("bytes")
                .and_then(Value::as_array)
                .ok_or_else(input_error)?
                .iter()
                .map(|value| {
                    let byte = value.as_u64().ok_or_else(input_error)?;
                    u8::try_from(byte).map_err(|_| input_error())
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        parsed.push(ParsedInstallFile { path, contents });
    }
    Ok(parsed)
}

fn parse_install_source(
    input: &Value,
) -> Result<SkillInstallSource, SkillManagementCapabilityError> {
    match input.get("source").and_then(Value::as_str) {
        None => Ok(SkillInstallSource::User),
        Some(value) if value == InstalledSkillMetadataSource::InstalledUrl.as_str() => {
            Ok(SkillInstallSource::InstalledUrl)
        }
        Some(_) => Err(input_error()),
    }
}

fn capability_error(error: SkillManagementError) -> SkillManagementCapabilityError {
    let skill_error_kind = error.kind();
    let kind = match error.kind() {
        SkillManagementErrorKind::InvalidInput => RuntimeDispatchErrorKind::InputEncode,
        SkillManagementErrorKind::FilesystemDenied => RuntimeDispatchErrorKind::FilesystemDenied,
        SkillManagementErrorKind::NotFound
        | SkillManagementErrorKind::Conflict
        | SkillManagementErrorKind::InvalidSkill => RuntimeDispatchErrorKind::OperationFailed,
        SkillManagementErrorKind::Resource => RuntimeDispatchErrorKind::Resource,
    };
    tracing::debug!(
        skill_management_error_kind = ?skill_error_kind,
        runtime_dispatch_error_kind = %kind,
        "skill management error mapped to runtime dispatch error"
    );
    SkillManagementCapabilityError::new(kind)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{InvocationId, MountView, ResourceScope, UserId};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn install_rejects_unresolved_url_input() {
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let mounts = MountView::default();
        let input = json!({"url": "https://example.test/SKILL.md"});
        let request = SkillManagementCapabilityRequest::new(
            SkillManagementCapabilityKind::Install,
            &scope,
            Some(&mounts),
            Arc::new(InMemoryBackend::new()),
            &input,
        );

        let error = dispatch(&request).await.unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::InputEncode);
    }
}
