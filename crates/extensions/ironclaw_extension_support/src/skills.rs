//! First-party skill management capability handlers.
//!
//! Host runtime adapts already-authorized capability invocations into
//! [`SkillManagementCapabilityRequest`]; this module receives scoped mounts
//! and an explicit filesystem handle only.

use std::sync::{Arc, LazyLock};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    dispatch::RuntimeDispatchErrorKind,
    mount::MountView,
    resource::{ResourceScope, ResourceUsage},
};
use ironclaw_skills::{
    InstalledSkillMetadataSource, SkillContentRequest, SkillInstallFile, SkillInstallRequest,
    SkillInstallSource, SkillManagementContext, SkillManagementError, SkillManagementErrorKind,
    SkillRemoveRequest, SkillUpdateRequest, install_skill, list_skills, read_skill_content,
    remove_skill, skill_summary_json, update_skill,
};
use serde_json::{Map, Value, json};

mod url_install;

pub use url_install::{SkillUrlFetchContext, is_allowed_code_artifact_host};

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
    /// Resource consumed before the failure. Only the URL-install path sets it
    /// (a denied or panicking fetch still burns egress bytes the host must
    /// account for); the filesystem paths leave it `None` and the host runtime
    /// supplies its own wall-clock accounting.
    usage: Option<ResourceUsage>,
}

impl SkillManagementCapabilityError {
    pub fn new(kind: RuntimeDispatchErrorKind) -> Self {
        Self { kind, usage: None }
    }

    pub fn kind(&self) -> RuntimeDispatchErrorKind {
        self.kind
    }

    #[must_use]
    pub fn with_usage(self, usage: ResourceUsage) -> Self {
        Self {
            usage: Some(usage),
            ..self
        }
    }

    pub fn usage(&self) -> Option<&ResourceUsage> {
        self.usage.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedInstallFile {
    path: String,
    contents: Vec<u8>,
}

/// Normalize a `builtin.skill_install` input before [`dispatch`] sees it.
///
/// Inline-`content` installs pass through untouched. A `url` install is
/// resolved here — the HTTPS/GitHub source is fetched through the mediated
/// egress port on `fetch`, and the fetched SKILL.md plus any bundle files are
/// rewritten into the same `content`/`files` shape the inline form uses, with
/// `source`/`source_url` recording the provenance. Anything else (both, or
/// neither, or `url` combined with `files`/`source`/`source_url`) is an input
/// error.
///
/// `usage` accumulates the fetch's network egress so the host runtime can
/// account for a failed install exactly as it accounts for a successful one.
pub async fn resolve_install_input(
    input: &Value,
    fetch: &SkillUrlFetchContext,
    usage: &mut ResourceUsage,
) -> Result<Value, SkillManagementCapabilityError> {
    let Some(object) = input.as_object() else {
        return Err(SkillManagementCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    };
    let has_content = object.contains_key("content");
    let url = object
        .get("url")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    // `files`, `source` and `source_url` are PROVENANCE fields this resolver
    // sets itself on the url path. A caller may never supply them, on either
    // arm, and the two arms refuse them differently on purpose:
    //
    //   * inline `content` + any of them -> hard `InputEncode`, nothing written.
    //     Accepting them would let a caller forge provenance (claim an inline
    //     skill was installed from a trusted URL) or smuggle arbitrary bundle
    //     files past the fetch. Pinned by
    //     `builtin_skill_install_rejects_hidden_url_install_fields`.
    //   * `url` + any of them -> the url arm below rebuilds a fresh object from
    //     the fetched payload and simply does not carry them over, so the
    //     install succeeds with the caller's files DROPPED (`files_installed`
    //     is 0). Pinned by
    //     `builtin_skill_install_url_path_ignores_caller_supplied_hidden_bundle_files`.
    //
    // Do not "fix" the asymmetry by making the url arm reject, and do not relax
    // the inline arm to accept a bundle: `dispatch_install` reading `files` is
    // not evidence that a *caller* may send it — that support exists for the
    // rewritten payload this resolver constructs. Both were proposed in review
    // on #7141 and both are refuted by the two integration tests named above.
    match (has_content, url) {
        (true, None)
            if !object.contains_key("files")
                && !object.contains_key("source")
                && !object.contains_key("source_url") =>
        {
            Ok(input.clone())
        }
        (false, Some(url)) => {
            let payload = url_install::fetch_skill_url_payload(fetch, url, usage).await?;
            let mut rewritten = Map::new();
            if let Some(name) = object.get("name").cloned() {
                rewritten.insert("name".to_string(), name);
            }
            rewritten.insert("content".to_string(), Value::String(payload.content));
            rewritten.insert(
                "source".to_string(),
                Value::String(
                    InstalledSkillMetadataSource::InstalledUrl
                        .as_str()
                        .to_string(),
                ),
            );
            rewritten.insert("source_url".to_string(), Value::String(url.to_string()));
            if !payload.files.is_empty() {
                rewritten.insert(
                    "files".to_string(),
                    Value::Array(
                        payload
                            .files
                            .into_iter()
                            .map(|file| {
                                json!({
                                    "path": file.path.display().to_string(),
                                    "bytes_base64": BASE64_STANDARD.encode(file.contents),
                                })
                            })
                            .collect(),
                    ),
                );
            }
            Ok(Value::Object(rewritten))
        }
        _ => Err(SkillManagementCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        )),
    }
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
    use ironclaw_host_api::{
        ids::{CapabilityId, InvocationId, UserId},
        mount::MountView,
        resource::ResourceScope,
    };
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

    /// A fetch context with no egress, which must never be used.
    ///
    /// Both cases below are decided from the input shape alone, so reaching the
    /// network at all would itself be the bug — and with `runtime_http_egress:
    /// None` a fetch could not succeed anyway, so a regression that started
    /// taking the url arm fails loudly here instead of going quiet.
    ///
    /// Deliberately kept with no caller: it is the negative control a future
    /// url-arm test reaches for. `dead_code` is allowed rather than the fixture
    /// deleted, because deleting it is what would let such a test quietly wire
    /// a real egress instead.
    #[allow(dead_code)]
    fn unused_fetch_context() -> SkillUrlFetchContext {
        SkillUrlFetchContext {
            capability_id: CapabilityId::new("ironclaw.skill.install").unwrap(),
            scope: ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap(),
            runtime_http_egress: None,
        }
    }
}
