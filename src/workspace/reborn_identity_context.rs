use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use async_trait::async_trait;
use ironclaw_loop_support::{
    HostIdentityContextBuildError, HostIdentityContextCandidate, HostIdentityContextSource,
    HostIdentityMessageContent, IdentityApplicability, IdentityFileName, identity_message_ref,
};
use ironclaw_memory::DEFAULT_PROMPT_PROTECTED_PATHS;
use ironclaw_turns::{LoopMessageRef, run_profile::LoopRunContext, run_profile::PromptMode};

use crate::{error::WorkspaceError, workspace::paths};

use super::Workspace;

const STABLE_IDENTITY_PATHS: &[&str] = &[
    paths::SOUL,
    paths::AGENTS,
    paths::USER,
    paths::IDENTITY,
    paths::TOOLS,
    paths::BOOTSTRAP,
    paths::ASSISTANT_DIRECTIVES,
];

#[derive(Clone)]
pub struct WorkspaceIdentityContextSource {
    workspace: Arc<Workspace>,
    loaded_identity_content: Arc<RwLock<HashMap<LoopMessageRef, HostIdentityMessageContent>>>,
}

impl WorkspaceIdentityContextSource {
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self {
            workspace,
            loaded_identity_content: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn stable_identity_paths() -> Vec<&'static str> {
        DEFAULT_PROMPT_PROTECTED_PATHS
            .iter()
            .copied()
            .filter(|path| STABLE_IDENTITY_PATHS.contains(path))
            .collect()
    }

    async fn read_identity_content(&self, path: &str) -> Result<Option<String>, WorkspaceError> {
        match self.workspace.read_primary(path).await {
            Ok(document) if document.content.is_empty() => Ok(None),
            Ok(document) => Ok(Some(document.content)),
            Err(WorkspaceError::DocumentNotFound { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn candidate_for_path(
        &self,
        path: &'static str,
    ) -> Result<Option<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        let Some(content) = self
            .read_identity_content(path)
            .await
            .map_err(|_| HostIdentityContextBuildError::SourceUnavailable)?
        else {
            return Ok(None);
        };
        let name = IdentityFileName::new(path)?;
        if summary_only_path(path) {
            return Ok(Some(
                HostIdentityContextCandidate::new_installed_summary_only(
                    name,
                    format!("personal identity file {path} available; raw content withheld"),
                    applicability_for_path(path),
                ),
            ));
        }

        let message_ref = identity_message_ref(&name, &content)
            .map_err(|_| HostIdentityContextBuildError::Internal)?;
        let model_visible_bytes = content.len();
        self.loaded_identity_content
            .write()
            .map_err(|_| HostIdentityContextBuildError::Internal)?
            .insert(
                message_ref.clone(),
                HostIdentityMessageContent {
                    name: name.clone(),
                    content,
                },
            );
        Ok(Some(HostIdentityContextCandidate::new_trusted(
            name,
            message_ref,
            format!("identity file {path} available"),
            applicability_for_path(path),
            model_visible_bytes,
        )))
    }
}

#[async_trait]
impl HostIdentityContextSource for WorkspaceIdentityContextSource {
    async fn load_identity_candidates(
        &self,
        _run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        let mut candidates = Vec::new();
        for path in Self::stable_identity_paths() {
            if let Some(candidate) = self.candidate_for_path(path).await? {
                candidates.push(candidate);
            }
        }
        Ok(candidates)
    }

    async fn resolve_identity_message_content(
        &self,
        _run_context: &LoopRunContext,
        message_ref: &LoopMessageRef,
    ) -> Result<Option<HostIdentityMessageContent>, HostIdentityContextBuildError> {
        self.loaded_identity_content
            .read()
            .map_err(|_| HostIdentityContextBuildError::Internal)
            .map(|loaded| loaded.get(message_ref).cloned())
    }
}

fn applicability_for_path(path: &str) -> IdentityApplicability {
    if path == paths::TOOLS {
        IdentityApplicability::OnCodeAct
    } else {
        IdentityApplicability::Always
    }
}

fn summary_only_path(path: &str) -> bool {
    // LoopRunContext does not yet carry an explicit group-chat/profile-privacy
    // policy bit. Until that lands, keep personal/profile-derived identity
    // files structurally summary-only so raw bytes cannot be promoted into the
    // trusted system prefix by this workspace source.
    matches!(path, paths::USER | paths::ASSISTANT_DIRECTIVES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_identity_context_uses_protected_path_canon() {
        let stable = WorkspaceIdentityContextSource::stable_identity_paths();
        assert_eq!(
            stable,
            vec![
                paths::SOUL,
                paths::AGENTS,
                paths::USER,
                paths::IDENTITY,
                paths::TOOLS,
                paths::BOOTSTRAP,
                paths::ASSISTANT_DIRECTIVES,
            ]
        );
        assert!(
            stable
                .iter()
                .all(|path| DEFAULT_PROMPT_PROTECTED_PATHS.contains(path))
        );
        assert!(!stable.contains(&paths::HEARTBEAT));
        assert!(!stable.contains(&paths::MEMORY));
        assert!(!stable.contains(&paths::PROFILE));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn workspace_identity_context_reads_primary_scope_only() {
        let test_db = test_db().await;

        Workspace::new_with_db("secondary", test_db.db.clone())
            .write(paths::AGENTS, "secondary instructions")
            .await
            .unwrap();
        Workspace::new_with_db("primary", test_db.db.clone())
            .write(paths::AGENTS, "primary instructions")
            .await
            .unwrap();
        let workspace = Arc::new(
            Workspace::new_with_db("primary", test_db.db.clone())
                .with_additional_read_scopes(vec!["secondary".to_string()]),
        );
        let source = WorkspaceIdentityContextSource::new(workspace);
        let context = run_context().await;
        let candidates = source
            .load_identity_candidates(&context, PromptMode::TextOnly)
            .await
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name.as_str(), paths::AGENTS);
        let content = source
            .resolve_identity_message_content(
                &context,
                candidates[0].message_ref.as_ref().expect("trusted ref"),
            )
            .await
            .unwrap()
            .expect("identity content");
        assert_eq!(content.content, "primary instructions");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn workspace_identity_context_resolves_loaded_ref_after_file_mutates() {
        let test_db = test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("primary", test_db.db.clone()));
        workspace
            .write(paths::AGENTS, "original instructions")
            .await
            .unwrap();

        let source = WorkspaceIdentityContextSource::new(workspace.clone());
        let context = run_context().await;
        let candidates = source
            .load_identity_candidates(&context, PromptMode::TextOnly)
            .await
            .unwrap();
        let message_ref = candidates[0].message_ref.as_ref().unwrap().clone();

        workspace
            .write(paths::AGENTS, "mutated instructions")
            .await
            .unwrap();

        let content = source
            .resolve_identity_message_content(&context, &message_ref)
            .await
            .unwrap()
            .expect("loaded identity content");
        assert_eq!(content.name.as_str(), paths::AGENTS);
        assert_eq!(content.content, "original instructions");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn workspace_identity_context_keeps_personal_files_summary_only() {
        let test_db = test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("primary", test_db.db.clone()));
        workspace
            .write(paths::USER, "private user profile")
            .await
            .unwrap();
        workspace
            .write(paths::ASSISTANT_DIRECTIVES, "private assistant directive")
            .await
            .unwrap();

        let source = WorkspaceIdentityContextSource::new(workspace);
        let context = run_context().await;
        let candidates = source
            .load_identity_candidates(&context, PromptMode::TextOnly)
            .await
            .unwrap();

        assert_eq!(candidates.len(), 2);
        for candidate in candidates {
            assert!(matches!(
                candidate.name.as_str(),
                paths::USER | paths::ASSISTANT_DIRECTIVES
            ));
            assert!(
                candidate.message_ref.is_none(),
                "{} must not expose raw prompt content without an explicit privacy policy bit",
                candidate.name
            );
            assert!(!candidate.safe_summary.contains("private"));
        }
    }

    #[cfg(feature = "libsql")]
    struct TestDb {
        db: Arc<dyn crate::db::Database>,
        _dir: tempfile::TempDir,
    }

    #[cfg(feature = "libsql")]
    async fn test_db() -> TestDb {
        let dir = tempfile::tempdir().expect("create temp dir");
        let db_path = dir.path().join("test.db");
        let backend = crate::db::libsql::LibSqlBackend::new_local(&db_path)
            .await
            .expect("create db");
        crate::db::Database::run_migrations(&backend)
            .await
            .expect("run migrations");
        TestDb {
            db: Arc::new(backend),
            _dir: dir,
        }
    }

    #[cfg(feature = "libsql")]
    async fn run_context() -> LoopRunContext {
        use ironclaw_host_api::{TenantId, ThreadId};
        use ironclaw_turns::{
            RunProfileResolutionRequest, RunProfileResolver, TurnId, TurnRunId, TurnScope,
            run_profile::InMemoryRunProfileResolver,
        };

        let resolved_run_profile = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        let scope = TurnScope::new(
            TenantId::new("tenant-workspace-identity").unwrap(),
            None,
            None,
            ThreadId::new("thread-workspace-identity").unwrap(),
        );
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
    }
}
