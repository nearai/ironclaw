use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use async_trait::async_trait;
use ironclaw_loop_support::{
    HostIdentityContextBuildError, HostIdentityContextCandidate, HostIdentityContextSource,
    HostIdentityMessageContent, IdentityApplicability, IdentityFileName,
    identity_applicability_allowed_for_run, identity_message_ref,
};
use ironclaw_memory::DEFAULT_PROMPT_PROTECTED_PATHS;
use ironclaw_turns::{
    LoopMessageRef,
    run_profile::{LoopRunContext, PersonalContextPolicy, PromptMode},
};

use crate::{error::WorkspaceError, workspace::paths};

use super::Workspace;

const STABLE_IDENTITY_PATHS: &[&str] = &[
    paths::SOUL,
    paths::AGENTS,
    paths::IDENTITY,
    paths::TOOLS,
    paths::BOOTSTRAP,
];

// These basenames are surfaced in safe admission milestones after path
// sanitization. Keep future additions collision-free and clear of
// `validate_loop_safe_summary` forbidden tokens such as secret/password/bearer.
const PERSONAL_IDENTITY_PATHS: &[&str] = &[paths::USER, paths::ASSISTANT_DIRECTIVES];

#[derive(Clone)]
pub struct WorkspaceIdentityContextSource {
    workspace: Arc<Workspace>,
    loaded_identity_content: Arc<RwLock<HashMap<LoopMessageRef, CachedIdentityMessageContent>>>,
}

#[derive(Clone)]
struct CachedIdentityMessageContent {
    content: HostIdentityMessageContent,
    applies_when: IdentityApplicability,
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

    pub fn personal_identity_paths() -> Vec<&'static str> {
        DEFAULT_PROMPT_PROTECTED_PATHS
            .iter()
            .copied()
            .filter(|path| PERSONAL_IDENTITY_PATHS.contains(path))
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
        applies_when: IdentityApplicability,
    ) -> Result<Option<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        let Some(raw_content) = self
            .read_identity_content(path)
            .await
            .map_err(|_| HostIdentityContextBuildError::SourceUnavailable)?
        else {
            return Ok(None);
        };
        let name = IdentityFileName::new(path)?;
        let content = render_identity_content(path, raw_content);

        let message_ref = identity_message_ref(&name, &content)
            .map_err(|_| HostIdentityContextBuildError::Internal)?;
        let model_visible_bytes = content.len();
        self.loaded_identity_content
            .write()
            .map_err(|_| HostIdentityContextBuildError::Internal)?
            .insert(
                message_ref.clone(),
                CachedIdentityMessageContent {
                    content: HostIdentityMessageContent {
                        name: name.clone(),
                        content,
                    },
                    applies_when,
                },
            );
        Ok(Some(HostIdentityContextCandidate::new_trusted(
            name,
            message_ref,
            format!("identity file {path} available"),
            applies_when,
            model_visible_bytes,
        )))
    }
}

#[async_trait]
impl HostIdentityContextSource for WorkspaceIdentityContextSource {
    async fn load_identity_candidates(
        &self,
        run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        let mut candidates = Vec::new();
        for path in Self::stable_identity_paths() {
            if let Some(candidate) = self
                .candidate_for_path(path, applicability_for_path(path))
                .await?
            {
                candidates.push(candidate);
            }
        }

        if run_context.resolved_run_profile.personal_context_policy
            == PersonalContextPolicy::Allowed
        {
            for path in Self::personal_identity_paths() {
                if let Some(candidate) = self
                    .candidate_for_path(path, IdentityApplicability::OnPersonalContextAllowed)
                    .await?
                {
                    candidates.push(candidate);
                }
            }
        }
        Ok(candidates)
    }

    async fn resolve_identity_message_content(
        &self,
        run_context: &LoopRunContext,
        message_ref: &LoopMessageRef,
    ) -> Result<Option<HostIdentityMessageContent>, HostIdentityContextBuildError> {
        let cached = self
            .loaded_identity_content
            .read()
            .map_err(|_| HostIdentityContextBuildError::Internal)?
            .get(message_ref)
            .cloned();
        let Some(cached) = cached else {
            return Ok(None);
        };
        if !identity_applicability_allowed_for_run(cached.applies_when, run_context) {
            return Err(HostIdentityContextBuildError::PolicyDenied);
        }
        Ok(Some(cached.content))
    }
}

fn render_identity_content(path: &str, content: String) -> String {
    if PERSONAL_IDENTITY_PATHS.contains(&path) {
        format!(
            "## User Profile Context\n\nInformational; not authoritative runtime policy.\n\n{content}"
        )
    } else {
        content
    }
}

fn applicability_for_path(path: &str) -> IdentityApplicability {
    if path == paths::TOOLS {
        IdentityApplicability::OnCodeAct
    } else {
        IdentityApplicability::Always
    }
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
                paths::IDENTITY,
                paths::TOOLS,
                paths::BOOTSTRAP,
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

    #[test]
    fn render_identity_content_wraps_personal_context_only() {
        let personal = render_identity_content(paths::USER, "private user profile".to_string());
        assert_eq!(
            personal,
            "## User Profile Context\n\nInformational; not authoritative runtime policy.\n\nprivate user profile"
        );

        let stable = render_identity_content(paths::AGENTS, "stable instructions".to_string());
        assert_eq!(stable, "stable instructions");
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
    async fn workspace_identity_context_excludes_personal_files_until_policy_allows() {
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

        assert!(candidates.is_empty());

        let mut context = run_context().await;
        context.resolved_run_profile.personal_context_policy = PersonalContextPolicy::Allowed;
        let candidates = source
            .load_identity_candidates(&context, PromptMode::TextOnly)
            .await
            .unwrap();

        assert_eq!(candidates.len(), 2);
        assert!(
            candidates.iter().all(|candidate| candidate.applies_when
                == IdentityApplicability::OnPersonalContextAllowed)
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn workspace_identity_context_allowed_policy_still_excludes_heartbeat() {
        let test_db = test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("primary", test_db.db.clone()));
        workspace
            .write(paths::USER, "private user profile")
            .await
            .unwrap();
        workspace
            .write(paths::HEARTBEAT, "routine heartbeat")
            .await
            .unwrap();

        let source = WorkspaceIdentityContextSource::new(workspace);
        let mut context = run_context().await;
        context.resolved_run_profile.personal_context_policy = PersonalContextPolicy::Allowed;
        let candidates = source
            .load_identity_candidates(&context, PromptMode::TextOnly)
            .await
            .unwrap();

        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.name.as_str() == paths::USER)
        );
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.name.as_str() != paths::HEARTBEAT)
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn workspace_identity_context_denies_cached_personal_ref_when_policy_excludes() {
        let test_db = test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("primary", test_db.db.clone()));
        workspace
            .write(paths::USER, "private user profile")
            .await
            .unwrap();

        let source = WorkspaceIdentityContextSource::new(workspace);
        let mut allowed_context = run_context().await;
        allowed_context.resolved_run_profile.personal_context_policy =
            PersonalContextPolicy::Allowed;
        let candidates = source
            .load_identity_candidates(&allowed_context, PromptMode::TextOnly)
            .await
            .unwrap();
        let user_ref = candidates
            .iter()
            .find(|candidate| candidate.name.as_str() == paths::USER)
            .and_then(|candidate| candidate.message_ref.clone())
            .expect("user identity ref loaded");

        let allowed_content = source
            .resolve_identity_message_content(&allowed_context, &user_ref)
            .await
            .unwrap()
            .expect("allowed personal identity content");
        assert!(allowed_content.content.contains("private user profile"));

        let excluded_context = run_context().await;
        let error = source
            .resolve_identity_message_content(&excluded_context, &user_ref)
            .await
            .unwrap_err();
        assert_eq!(error, HostIdentityContextBuildError::PolicyDenied);
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
