//! First-party coding capability handlers.
//!
//! Keep v1-compatible coding families in narrow modules. Host runtime adapts
//! already-authorized capability invocations into [`CodingCapabilityRequest`];
//! this module receives scoped paths and an explicit filesystem handle only.

mod config;
mod diff_preview;
mod file;
mod glob_tool;
mod grep_tool;
mod inputs;
mod paths;
mod state;
mod text;
mod types;

use std::sync::Arc;

use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    CapabilityDisplayOutputPreview, MountView, ResourceScope, RuntimeDispatchErrorKind,
};
use serde_json::Value;

use state::SharedCodingEditLocks;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodingCapabilityKind {
    ReadFile,
    WriteFile,
    ListDir,
    Glob,
    Grep,
    ApplyPatch,
}

#[derive(Clone)]
pub struct CodingCapabilityRequest<'a> {
    pub(crate) kind: CodingCapabilityKind,
    pub(crate) scope: &'a ResourceScope,
    pub(crate) mounts: Option<&'a MountView>,
    pub(crate) filesystem: Arc<dyn RootFilesystem>,
    pub(crate) input: &'a Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingCapabilityOutput {
    pub output: Value,
    pub display_preview: Option<CapabilityDisplayOutputPreview>,
}

impl CodingCapabilityOutput {
    pub fn new(output: Value) -> Self {
        Self {
            output,
            display_preview: None,
        }
    }

    pub fn with_display_preview(
        output: Value,
        display_preview: Option<CapabilityDisplayOutputPreview>,
    ) -> Self {
        Self {
            output,
            display_preview,
        }
    }
}

impl<'a> CodingCapabilityRequest<'a> {
    pub fn new(
        kind: CodingCapabilityKind,
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
#[error("coding capability dispatch failed: {kind}")]
pub struct CodingCapabilityError {
    kind: RuntimeDispatchErrorKind,
    safe_summary: Option<String>,
}

impl CodingCapabilityError {
    pub fn new(kind: RuntimeDispatchErrorKind) -> Self {
        Self {
            kind,
            safe_summary: None,
        }
    }

    pub fn with_safe_summary(
        kind: RuntimeDispatchErrorKind,
        safe_summary: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            safe_summary: Some(bound_safe_summary(safe_summary.into())),
        }
    }

    pub fn kind(&self) -> RuntimeDispatchErrorKind {
        self.kind
    }

    pub fn safe_summary(&self) -> Option<&str> {
        self.safe_summary.as_deref()
    }
}

#[derive(Debug, Default)]
pub struct CodingCapabilityState {
    edit_locks: SharedCodingEditLocks,
}

impl CodingCapabilityState {
    pub async fn dispatch(
        &self,
        request: &CodingCapabilityRequest<'_>,
    ) -> Result<CodingCapabilityOutput, CodingCapabilityError> {
        dispatch(request, &self.edit_locks).await
    }
}

async fn dispatch(
    request: &CodingCapabilityRequest<'_>,
    edit_locks: &SharedCodingEditLocks,
) -> Result<CodingCapabilityOutput, CodingCapabilityError> {
    match request.kind {
        CodingCapabilityKind::ReadFile => file::read_file(request)
            .await
            .map(CodingCapabilityOutput::new),
        CodingCapabilityKind::WriteFile => file::write_file(request, edit_locks).await,
        CodingCapabilityKind::ListDir => file::list_dir(request)
            .await
            .map(CodingCapabilityOutput::new),
        CodingCapabilityKind::Glob => glob_tool::glob(request)
            .await
            .map(CodingCapabilityOutput::new),
        CodingCapabilityKind::Grep => grep_tool::grep(request)
            .await
            .map(CodingCapabilityOutput::new),
        CodingCapabilityKind::ApplyPatch => file::apply_patch(request, edit_locks).await,
    }
}

fn input_error() -> CodingCapabilityError {
    CodingCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
}

fn operation_error() -> CodingCapabilityError {
    CodingCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
}

fn operation_error_with_summary(summary: impl Into<String>) -> CodingCapabilityError {
    CodingCapabilityError::with_safe_summary(RuntimeDispatchErrorKind::OperationFailed, summary)
}

fn bound_safe_summary(summary: String) -> String {
    const MAX_CHARS: usize = 512;
    const ELLIPSIS: &str = "...";
    let summary = summary.trim();
    let mut chars = summary.chars();
    let bounded: String = chars.by_ref().take(MAX_CHARS).collect();
    if chars.next().is_some() {
        let truncated_limit = MAX_CHARS - ELLIPSIS.chars().count();
        let bounded: String = bounded.chars().take(truncated_limit).collect();
        format!("{bounded}{ELLIPSIS}")
    } else {
        bounded
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn coding_tools_do_not_select_runtime_backends() {
        let sources = [
            include_str!("file.rs"),
            include_str!("glob_tool.rs"),
            include_str!("grep_tool.rs"),
            include_str!("paths.rs"),
        ];
        for source in sources {
            assert!(!source.contains("ProcessBackendKind"));
            assert!(!source.contains("FilesystemBackendKind"));
        }
    }

    #[test]
    fn safe_summary_bound_includes_ellipsis_in_limit() {
        let summary = super::bound_safe_summary("x".repeat(600));

        assert_eq!(summary.chars().count(), 512);
        assert!(summary.ends_with("..."));
    }

    #[test]
    fn safe_summary_bound_leaves_exact_limit_unchanged() {
        let input = "x".repeat(512);

        assert_eq!(super::bound_safe_summary(input.clone()), input);
    }
}
