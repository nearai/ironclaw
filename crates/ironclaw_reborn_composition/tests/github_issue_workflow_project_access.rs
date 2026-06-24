//! Caller-level authorization tests for the PRODUCTION
//! `ProjectServiceWorkflowProjectAccess::assert_workflow_config_access`.
//!
//! These drive the REAL composition impl (via its public test-support
//! constructor), not a `FakeProjectAccess`, so they exercise the production
//! authorization semantics that the poller relies on before discovering issues,
//! claiming runs, and burning Triage -> Implementation:
//!
//! - every configured repository must be within the project's declared
//!   `github_issue_workflow.repositories` selector (a misconfigured repo fails
//!   loud up front instead of at the first provider write); and
//! - the configured provider account ref must match the project's
//!   composition-bound GitHub credential account (the "wrong account id"
//!   footgun).
#![cfg(all(feature = "github-issue-workflow-beta", feature = "test-support"))]

mod github_issue_workflow_project_access {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_github_issue_workflow::{
        GithubIssueWorkflowError, GithubProviderAccountRef, GithubRepositorySelector,
        WorkflowConfigAccessRequest,
    };
    use ironclaw_host_api::{ProjectId, TenantId, UserId};
    use ironclaw_product_workflow::{
        ProjectCaller, ProjectService, ProjectServiceError, RebornAddMemberRequest,
        RebornCreateProjectRequest, RebornDeleteProjectRequest, RebornGetProjectRequest,
        RebornListMembersRequest, RebornListMembersResponse, RebornListProjectsRequest,
        RebornListProjectsResponse, RebornProjectInfo, RebornProjectMemberInfo,
        RebornProjectResponse, RebornProjectRole, RebornProjectState, RebornRemoveMemberRequest,
        RebornUpdateMemberRoleRequest, RebornUpdateProjectRequest,
    };
    use ironclaw_reborn_composition::test_support::project_service_workflow_project_access_for_test;
    use serde_json::{Value as JsonValue, json};

    const TENANT: &str = "workflow-tenant";
    const OWNER: &str = "workflow-owner";
    const PROJECT: &str = "workflow-project";
    const BOUND_ACCOUNT: &str = "runtime-github-account";

    /// A project whose `github_issue_workflow` section declares a single
    /// allowable repository. The bound credential account is supplied to the
    /// access checker separately (it is composition config, never project
    /// metadata — see `assert_workflow_config_access` in `config_source.rs`).
    fn project_metadata() -> JsonValue {
        json!({
            "github_issue_workflow": {
                "enabled": true,
                "repositories": [
                    { "owner": "nearai", "repo": "ironclaw" }
                ]
            }
        })
    }

    fn bound_account() -> GithubProviderAccountRef {
        GithubProviderAccountRef {
            provider: "github".to_string(),
            account_id: BOUND_ACCOUNT.to_string(),
        }
    }

    fn config_access_request(
        repositories: Vec<GithubRepositorySelector>,
        provider_account_ref: GithubProviderAccountRef,
    ) -> WorkflowConfigAccessRequest {
        WorkflowConfigAccessRequest {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            creator_user_id: UserId::new(OWNER).expect("user"),
            project_id: ProjectId::new(PROJECT).expect("project"),
            repositories,
            provider_account_ref,
        }
    }

    fn allowed_repo() -> GithubRepositorySelector {
        GithubRepositorySelector::new("nearai", "ironclaw").expect("selector")
    }

    #[tokio::test]
    async fn config_access_allows_declared_repo_and_matching_account() {
        let project_service = Arc::new(FakeProjectService::new(project_metadata()));
        let access = project_service_workflow_project_access_for_test(
            project_service.clone(),
            bound_account(),
        );

        access
            .assert_workflow_config_access(config_access_request(
                vec![allowed_repo()],
                bound_account(),
            ))
            .await
            .expect("a declared repo with the bound account must be allowed");
    }

    #[tokio::test]
    async fn config_access_denies_repo_outside_project_declaration() {
        let project_service = Arc::new(FakeProjectService::new(project_metadata()));
        let access = project_service_workflow_project_access_for_test(
            project_service.clone(),
            bound_account(),
        );

        // `nearai/some-other-repo` is well-formed but NOT in the project's
        // declared repository selector: enablement must fail loud here instead
        // of burning stages before the first provider write.
        let offending =
            GithubRepositorySelector::new("nearai", "some-other-repo").expect("selector");
        let error = access
            .assert_workflow_config_access(config_access_request(vec![offending], bound_account()))
            .await
            .expect_err("a repo outside the project declaration must be denied");

        match error {
            GithubIssueWorkflowError::PolicyDenied { reason } => {
                assert!(
                    reason.contains("nearai/some-other-repo"),
                    "denial must name the offending repo, got: {reason}"
                );
            }
            other => panic!("expected PolicyDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn config_access_denies_non_matching_provider_account() {
        let project_service = Arc::new(FakeProjectService::new(project_metadata()));
        let access = project_service_workflow_project_access_for_test(
            project_service.clone(),
            bound_account(),
        );

        // The repo is allowable, but the account id does not match the project's
        // composition-bound credential account: the "wrong account id" footgun.
        let wrong_account = GithubProviderAccountRef {
            provider: "github".to_string(),
            account_id: "wrong-github-account".to_string(),
        };
        let error = access
            .assert_workflow_config_access(config_access_request(
                vec![allowed_repo()],
                wrong_account,
            ))
            .await
            .expect_err("a non-matching provider account must be denied");

        assert!(
            matches!(error, GithubIssueWorkflowError::PolicyDenied { .. }),
            "expected PolicyDenied, got {error:?}"
        );
    }

    struct FakeProjectService {
        metadata: JsonValue,
        captured_get_projects: Mutex<Vec<(ProjectCaller, RebornGetProjectRequest)>>,
    }

    impl FakeProjectService {
        fn new(metadata: JsonValue) -> Self {
            Self {
                metadata,
                captured_get_projects: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl ProjectService for FakeProjectService {
        async fn list_projects(
            &self,
            _caller: ProjectCaller,
            _request: RebornListProjectsRequest,
        ) -> Result<RebornListProjectsResponse, ProjectServiceError> {
            panic!("list_projects is not used by these tests")
        }

        async fn create_project(
            &self,
            _caller: ProjectCaller,
            _request: RebornCreateProjectRequest,
        ) -> Result<RebornProjectResponse, ProjectServiceError> {
            panic!("create_project is not used by these tests")
        }

        async fn get_project(
            &self,
            caller: ProjectCaller,
            request: RebornGetProjectRequest,
        ) -> Result<RebornProjectResponse, ProjectServiceError> {
            self.captured_get_projects
                .lock()
                .expect("lock")
                .push((caller, request.clone()));
            Ok(RebornProjectResponse {
                project: RebornProjectInfo {
                    project_id: request.project_id,
                    name: "Workflow project".to_string(),
                    description: String::new(),
                    icon: None,
                    color: None,
                    metadata: self.metadata.clone(),
                    state: RebornProjectState::Active,
                    role: RebornProjectRole::Owner,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                },
            })
        }

        async fn update_project(
            &self,
            _caller: ProjectCaller,
            _request: RebornUpdateProjectRequest,
        ) -> Result<RebornProjectResponse, ProjectServiceError> {
            panic!("update_project is not used by these tests")
        }

        async fn delete_project(
            &self,
            _caller: ProjectCaller,
            _request: RebornDeleteProjectRequest,
        ) -> Result<(), ProjectServiceError> {
            panic!("delete_project is not used by these tests")
        }

        async fn list_members(
            &self,
            _caller: ProjectCaller,
            _request: RebornListMembersRequest,
        ) -> Result<RebornListMembersResponse, ProjectServiceError> {
            panic!("list_members is not used by these tests")
        }

        async fn add_member(
            &self,
            _caller: ProjectCaller,
            _request: RebornAddMemberRequest,
        ) -> Result<RebornProjectMemberInfo, ProjectServiceError> {
            panic!("add_member is not used by these tests")
        }

        async fn update_member_role(
            &self,
            _caller: ProjectCaller,
            _request: RebornUpdateMemberRoleRequest,
        ) -> Result<RebornProjectMemberInfo, ProjectServiceError> {
            panic!("update_member_role is not used by these tests")
        }

        async fn remove_member(
            &self,
            _caller: ProjectCaller,
            _request: RebornRemoveMemberRequest,
        ) -> Result<(), ProjectServiceError> {
            panic!("remove_member is not used by these tests")
        }
    }
}
