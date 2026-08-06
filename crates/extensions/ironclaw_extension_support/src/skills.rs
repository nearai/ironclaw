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
/// Inline installs (`content`, optionally with `files`) pass through untouched. A `url` install is
/// fetched through the mediated egress port and rewritten into that same shape, with
/// `source`/`source_url` recording provenance. Anything else is an input error.
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
    // `source`/`source_url` are provenance this resolver sets itself on the url path, so a caller
    // supplying them is claiming its own output was fetched from somewhere trusted. Refused on the
    // inline arm; on the url arm the rebuild below simply does not carry them over, which also
    // drops any caller-supplied `files`. Do not make the url arm reject instead: a fetch is the
    // authority on what a fetched skill contains.
    //
    // `files` is NOT provenance, it is ordinary caller content, and an agent authoring a skill has
    // to be able to attach the script the skill exists to preserve. Refusing it failed the WHOLE
    // install, not just the file: on the 31-task SkillsBench subset (nearai/benchmarks#287) 18
    // correctly-shaped entries across 9 calls were all refused. The refusal predates #7141, which
    // moved it here verbatim and rightly declined to relax it inside a move-only refactor. What
    // makes accepting it safe is not this shape check but `normalize_safe_relative_path`
    // downstream, which confines each entry to the skill's own directory.
    //
    // Pinned by `builtin_skill_install_{accepts_an_agent_authored_bundle_with_scripts,
    // rejects_a_bundle_that_also_forges_provenance,
    // rejects_a_bundle_file_escaping_its_skill_directory}`.
    match (has_content, url) {
        (true, None) if !object.contains_key("source") && !object.contains_key("source_url") => {
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
        // Storage and execution are different places, so say both. `path` is the read-only,
        // database-backed store; no process can open it. Without this an agent that installed a
        // scripted skill and tried to verify it probed `/skills`, hand-copied the file into its
        // workspace to run it, and then claimed the store path was executable -- which it is not.
        // Ordering, not just a path. Naming the field `..._after_activation` was not enough: an agent
        // read the path out of this result and used it immediately, before activating, and burned two
        // failed calls discovering that the files were not there yet.
        "bundled_files_runnable": (!files.is_empty()).then(|| {
            json!({
                "requires_first": format!("skill_activate with name={}", installed.name),
                "then_at": ironclaw_skills::runnable_skill_dir(&installed.name),
                "note": "The skill root is read-only and never executable. Activating stages the \
                         bundled files into the workspace; only then does the path above exist.",
            })
        }),
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
        // `text` first: a bundle file an AGENT authors is a script, a reference doc or a
        // schema fragment -- all UTF-8. Requiring base64 (or a JSON array of byte
        // integers) for those made the capability effectively unusable by a model: it
        // burns ~33% more tokens, and an encoding slip fails the whole install with
        // InputEncode. Binary payloads keep using `bytes_base64`/`bytes`.
        let contents = if let Some(text) = file.get("text") {
            text.as_str().ok_or_else(input_error)?.as_bytes().to_vec()
        } else if let Some(encoded) = file.get("bytes_base64") {
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

#[cfg(test)]
mod install_files_encoding_tests {
    use super::parse_install_files;
    use serde_json::json;

    /// The encoding an agent actually produces. Before `text` existed, a self-authored
    /// bundle had to be base64'd, and measured on the 31-task SkillsBench subset
    /// (nearai/benchmarks#287) **0 of 27** agent-authored skills shipped any resource
    /// file at all -- the schema did not advertise `files` and the encoding was hostile.
    #[test]
    fn text_files_are_accepted_as_utf8() {
        let input = json!({
            "content": "# skill",
            "files": [
                { "path": "scripts/analyze.py", "text": "import sys\nprint(sys.argv)\n" },
                { "path": "references/units.md", "text": "mg/dL -> mmol/L: x 0.0555\n" }
            ]
        });
        let parsed = parse_install_files(&input).expect("text files parse");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].path, "scripts/analyze.py");
        assert_eq!(
            String::from_utf8(parsed[0].contents.clone()).unwrap(),
            "import sys\nprint(sys.argv)\n"
        );
        assert!(
            String::from_utf8(parsed[1].contents.clone())
                .unwrap()
                .contains("0.0555")
        );
    }

    /// Binary payloads keep working; `text` is additive, not a replacement.
    #[test]
    fn base64_still_accepted_and_text_takes_precedence() {
        let b64 =
            json!({ "content": "# s", "files": [{ "path": "a.bin", "bytes_base64": "aGVsbG8=" }] });
        let parsed = parse_install_files(&b64).expect("base64 parses");
        assert_eq!(parsed[0].contents, b"hello");

        // both present -> `text` wins, since that is the documented preference
        let both = json!({ "content": "# s",
            "files": [{ "path": "a.txt", "text": "plain", "bytes_base64": "aGVsbG8=" }] });
        let parsed = parse_install_files(&both).expect("parses");
        assert_eq!(parsed[0].contents, b"plain");
    }

    /// Absent `files` stays a no-op, so prose-only installs are unchanged.
    #[test]
    fn no_files_key_is_empty_not_an_error() {
        assert!(
            parse_install_files(&json!({ "content": "# s" }))
                .expect("ok")
                .is_empty()
        );
    }

    /// A file entry with no usable encoding must fail rather than install an empty file.
    #[test]
    fn entry_without_any_content_is_rejected() {
        assert!(parse_install_files(&json!({ "files": [{ "path": "x.py" }] })).is_err());
        assert!(parse_install_files(&json!({ "files": [{ "text": "no path" }] })).is_err());
    }
}

/// Which inline inputs [`resolve_install_input`] admits.
///
/// The fetch context carries no egress port, so a case that wrongly routes to the url arm fails
/// closed rather than passing for the wrong reason. Asserted end to end in
/// `ironclaw_host_runtime/tests/first_party_builtin_tools.rs`.
#[cfg(test)]
mod resolve_install_input_tests {
    use ironclaw_host_api::{
        ids::{CapabilityId, InvocationId, UserId},
        resource::{ResourceScope, ResourceUsage},
    };
    use serde_json::json;

    use super::{SkillUrlFetchContext, resolve_install_input};

    fn fetch_context() -> SkillUrlFetchContext {
        SkillUrlFetchContext {
            capability_id: CapabilityId::new("builtin.skill_install").expect("capability id"),
            scope: ResourceScope::local_default(
                UserId::new("ada").expect("user"),
                InvocationId::new(),
            )
            .expect("scope"),
            runtime_http_egress: None,
        }
    }

    async fn resolve(input: serde_json::Value) -> Option<serde_json::Value> {
        let mut usage = ResourceUsage::default();
        resolve_install_input(&input, &fetch_context(), &mut usage)
            .await
            .ok()
    }

    /// The bundle an agent authors is forwarded whole, `files` included.
    #[tokio::test]
    async fn an_inline_bundle_is_forwarded_unchanged() {
        let input = json!({
            "name": "verify-bib",
            "content": "# Verify BibTeX\n\nRun scripts/verify_bib.py\n",
            "files": [{ "path": "scripts/verify_bib.py", "text": "#!/usr/bin/env python3\n" }]
        });
        assert_eq!(
            resolve(input.clone()).await.as_ref(),
            Some(&input),
            "the resolver must not rewrite an inline install; `dispatch` reads `files` from it"
        );
    }

    /// Prose-only installs are unaffected by admitting `files`.
    #[tokio::test]
    async fn prose_only_still_resolves() {
        let input = json!({ "name": "x", "content": "# x" });
        assert_eq!(resolve(input.clone()).await.as_ref(), Some(&input));
    }

    /// Provenance stays the resolver's to set, with or without a bundle attached.
    #[tokio::test]
    async fn forged_provenance_is_refused_with_or_without_files() {
        for input in [
            json!({ "content": "# x", "source": "installed_url" }),
            json!({ "content": "# x", "source_url": "https://e.example/SKILL.md" }),
            json!({
                "content": "# x",
                "files": [{ "path": "scripts/x.py", "text": "print(1)\n" }],
                "source": "installed_url"
            }),
        ] {
            assert!(
                resolve(input.clone()).await.is_none(),
                "must refuse caller-set provenance: {input}"
            );
        }
    }

    /// Neither `content` nor `url`, or both, is still an input error.
    #[tokio::test]
    async fn ambiguous_and_empty_inputs_are_refused() {
        assert!(resolve(json!({ "name": "x" })).await.is_none());
        assert!(
            resolve(json!({ "content": "# x", "url": "https://github.com/o/r" }))
                .await
                .is_none()
        );
        assert!(
            resolve(json!({ "files": [{ "path": "a.py", "text": "" }] }))
                .await
                .is_none(),
            "a bundle with no SKILL.md has nothing to install"
        );
    }
}
