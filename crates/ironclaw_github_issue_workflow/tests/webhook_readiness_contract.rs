mod webhook_readiness_contract {
    use chrono::{TimeZone, Utc};
    use ironclaw_github_issue_workflow::{
        GithubCommentRef, GithubIssueRef, GithubIssueWebhookAction, GithubIssueWebhookObservation,
        GithubIssueWebhookSnapshot, GithubProviderBindingRef, GithubProviderRef,
        GithubPullRequestRef, GithubPullRequestReviewCommentWebhookAction,
        GithubPullRequestReviewCommentWebhookObservation, GithubWebhookActor,
        GithubWebhookIssueCommentAction, GithubWebhookIssueCommentObservation,
        GithubWebhookObservation, NormalizeGithubWebhookEventInput, WorkflowEventSourceKind,
        normalize_github_webhook_event, primary_pr_binding_ref,
    };

    fn fixed_time(seconds: i64) -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).unwrap()
    }

    fn issue() -> GithubIssueRef {
        GithubIssueRef {
            owner: "nearai".to_string(),
            repo: "ironclaw".to_string(),
            number: 42,
            node_id: Some("issue-node-42".to_string()),
            url: "https://github.com/nearai/ironclaw/issues/42".to_string(),
            default_branch: "main".to_string(),
        }
    }

    fn pull_request() -> GithubPullRequestRef {
        GithubPullRequestRef {
            owner: "nearai".to_string(),
            repo: "ironclaw".to_string(),
            number: 12,
            node_id: Some("pr-node-12".to_string()),
            url: "https://github.com/nearai/ironclaw/pull/12".to_string(),
            head_branch: "codex/fix-42".to_string(),
            head_sha: Some("head-sha-12".to_string()),
        }
    }

    fn comment(node_id: &str, url_suffix: &str) -> GithubCommentRef {
        GithubCommentRef {
            node_id: Some(node_id.to_string()),
            url: format!("https://github.com/nearai/ironclaw/pull/12{url_suffix}"),
        }
    }

    fn actor(login: &str, node_id: &str) -> GithubWebhookActor {
        GithubWebhookActor {
            login: login.to_string(),
            node_id: Some(node_id.to_string()),
        }
    }

    fn issue_binding() -> GithubProviderBindingRef {
        GithubProviderBindingRef {
            provider_ref: GithubProviderRef {
                system: "github".to_string(),
                resource_type: "issue".to_string(),
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
                provider_id: "issue-node-42".to_string(),
                provider_url: Some("https://github.com/nearai/ironclaw/issues/42".to_string()),
            },
            role: "primary".to_string(),
        }
    }

    fn issue_comment_binding(comment: &GithubCommentRef) -> GithubProviderBindingRef {
        GithubProviderBindingRef {
            provider_ref: GithubProviderRef {
                system: "github".to_string(),
                resource_type: "issue_comment".to_string(),
                owner: "nearai".to_string(),
                repo: "ironclaw".to_string(),
                provider_id: comment.node_id.clone().unwrap(),
                provider_url: Some(comment.url.clone()),
            },
            role: "primary".to_string(),
        }
    }

    fn normalize(
        observation: GithubWebhookObservation,
        provider_bindings: Vec<GithubProviderBindingRef>,
    ) -> Vec<ironclaw_github_issue_workflow::WorkflowEventEnvelope<serde_json::Value>> {
        normalize_github_webhook_event(NormalizeGithubWebhookEventInput {
            source_delivery_id: Some("delivery-123".to_string()),
            observed_at: fixed_time(1000),
            actor: Some(actor("reviewer", "actor-reviewer")),
            workflow_actor: Some(actor("ironclaw-bot", "actor-bot")),
            matched_provider_bindings: provider_bindings,
            observation,
        })
        .unwrap()
    }

    #[test]
    fn issues_webhook_normalizes_to_issue_changed_event() {
        let events = normalize(
            GithubWebhookObservation::Issues(GithubIssueWebhookObservation {
                action: GithubIssueWebhookAction::Edited,
                issue: GithubIssueWebhookSnapshot {
                    issue: issue(),
                    title: "Bug report".to_string(),
                    state: "open".to_string(),
                    labels: vec!["bug".to_string()],
                    updated_at: Some(fixed_time(900)),
                    closed_at: None,
                    comment_count: Some(3),
                    body_present: true,
                },
            }),
            vec![issue_binding()],
        );

        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.source_kind, WorkflowEventSourceKind::GithubWebhook);
        assert_eq!(event.source_delivery_id.as_deref(), Some("delivery-123"));
        assert_eq!(event.provider.resource_type, "issue");
        assert_eq!(event.provider.provider_id, "issue-node-42");
        assert_eq!(event.provider_updated_at, Some(fixed_time(900)));
        assert_eq!(event.payload_schema, "github.issue.changed.v1");
        assert_eq!(
            event.idempotency_key.as_str(),
            "issue:issue-node-42:updated:1970-01-01T00:15:00.000000000Z"
        );
        assert_eq!(event.payload["issue"]["number"], 42);
        assert_eq!(event.payload["provider_snapshot"]["comment_count"], 3);
        assert_eq!(event.payload["provider_snapshot"]["body_present"], true);
        assert!(event.payload.get("headers").is_none());
    }

    #[test]
    fn issue_comment_webhook_on_pr_routes_to_pr_comment_event() {
        let pull_request = pull_request();
        let comment = comment("issue-comment-node-1", "#issuecomment-1");

        let events = normalize(
            GithubWebhookObservation::IssueComment(GithubWebhookIssueCommentObservation {
                action: GithubWebhookIssueCommentAction::Created,
                issue: issue(),
                pull_request: Some(pull_request.clone()),
                comment: comment.clone(),
                created_at: fixed_time(910),
                updated_at: fixed_time(920),
            }),
            vec![primary_pr_binding_ref(&pull_request)],
        );

        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.provider.resource_type, "issue_comment");
        assert_eq!(event.provider.provider_id, "issue-comment-node-1");
        assert_eq!(event.payload_schema, "github.review_comment.created.v1");
        assert_eq!(
            event.idempotency_key.as_str(),
            "review-comment:issue-comment-node-1"
        );
        assert_eq!(event.payload["pull_request"]["number"], 12);
        assert_eq!(event.payload["comment"]["url"], comment.url);
        assert!(event.payload.get("body").is_none());
    }

    #[test]
    fn pull_request_review_comment_webhook_routes_by_provider_binding() {
        let pull_request = pull_request();
        let comment = comment("review-comment-node-1", "#discussion_r1");

        let events = normalize(
            GithubWebhookObservation::PullRequestReviewComment(
                GithubPullRequestReviewCommentWebhookObservation {
                    action: GithubPullRequestReviewCommentWebhookAction::Created,
                    pull_request: pull_request.clone(),
                    comment: comment.clone(),
                    created_at: fixed_time(930),
                    updated_at: fixed_time(940),
                },
            ),
            vec![primary_pr_binding_ref(&pull_request)],
        );

        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.provider.resource_type, "review_comment");
        assert_eq!(event.provider.provider_id, "review-comment-node-1");
        assert_eq!(event.provider_updated_at, Some(fixed_time(940)));
        assert_eq!(event.payload_schema, "github.review_comment.created.v1");
        assert_eq!(event.payload["pull_request"]["node_id"], "pr-node-12");
        assert_eq!(event.payload["comment"]["url"], comment.url);
    }

    #[test]
    fn duplicate_webhook_delivery_reuses_source_delivery_id() {
        let first = normalize(
            GithubWebhookObservation::Issues(GithubIssueWebhookObservation {
                action: GithubIssueWebhookAction::Edited,
                issue: GithubIssueWebhookSnapshot {
                    issue: issue(),
                    title: "Bug report".to_string(),
                    state: "open".to_string(),
                    labels: vec!["bug".to_string()],
                    updated_at: Some(fixed_time(950)),
                    closed_at: None,
                    comment_count: Some(1),
                    body_present: false,
                },
            }),
            vec![issue_binding()],
        );
        let duplicate = normalize(
            GithubWebhookObservation::Issues(GithubIssueWebhookObservation {
                action: GithubIssueWebhookAction::Edited,
                issue: GithubIssueWebhookSnapshot {
                    issue: issue(),
                    title: "Bug report".to_string(),
                    state: "open".to_string(),
                    labels: vec!["bug".to_string()],
                    updated_at: Some(fixed_time(950)),
                    closed_at: None,
                    comment_count: Some(1),
                    body_present: false,
                },
            }),
            vec![issue_binding()],
        );

        assert_eq!(first[0].source_delivery_id, duplicate[0].source_delivery_id);
        assert_eq!(first[0].idempotency_key, duplicate[0].idempotency_key);
    }

    #[test]
    fn self_authored_webhook_echo_is_suppressed_when_binding_matches() {
        let comment = comment("issue-comment-node-bot", "#issuecomment-2");
        let events = normalize_github_webhook_event(NormalizeGithubWebhookEventInput {
            source_delivery_id: Some("delivery-bot-echo".to_string()),
            observed_at: fixed_time(1000),
            actor: Some(actor("ironclaw-bot", "actor-bot")),
            workflow_actor: Some(actor("ironclaw-bot", "actor-bot")),
            matched_provider_bindings: vec![issue_comment_binding(&comment)],
            observation: GithubWebhookObservation::IssueComment(
                GithubWebhookIssueCommentObservation {
                    action: GithubWebhookIssueCommentAction::Created,
                    issue: issue(),
                    pull_request: Some(pull_request()),
                    comment,
                    created_at: fixed_time(960),
                    updated_at: fixed_time(970),
                },
            ),
        })
        .unwrap();

        assert!(events.is_empty());
    }
}
