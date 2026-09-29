//! Composition wiring for turn-start tool selection: the parsed
//! `REBORN_TOOL_PREFETCH*` settings (`ironclaw_config`, tested there) become
//! the loop host's typed configuration on `RebornRuntimeInput`, and the
//! runtime build refuses a selection it cannot serve. Selection behaviour
//! itself is tested in `ironclaw_loop_host` and at the integration tier.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_composition::{
    PollSettings, RebornRuntimeIdentity, RebornRuntimeInput, build_reborn_runtime,
};
use ironclaw_config::{
    JevSettings, ToolPrefetchMode, ToolPrefetchSettings, ToolReselectionSettings,
    ToolSelectionClassifierSettings,
};
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse, ToolDisclosureMode, ToolPrefetchConfigError, ToolPrefetchRanking,
};

fn settings(mode: ToolPrefetchMode, max_tools: usize, always: &[&str]) -> ToolPrefetchSettings {
    ToolPrefetchSettings {
        mode,
        classifier: ToolSelectionClassifierSettings::Local,
        max_tools,
        token_budget: 32_000,
        min_similarity: 0.0,
        min_relative: 0.0,
        always: always.iter().map(|name| name.to_string()).collect(),
        context_messages: 16,
        segment_bytes: 2_048,
        reselection: ToolReselectionSettings::default(),
    }
}

#[derive(Debug)]
struct ReplyGateway;

#[async_trait]
impl HostManagedModelGateway for ReplyGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Ok(HostManagedModelResponse::assistant_reply("ok".to_string()))
    }
}

fn runtime_input(root: &std::path::Path) -> RebornRuntimeInput {
    RebornRuntimeInput::from_build_input(
        ironclaw_composition::local_filesystem_build_input(
            "tool-prefetch-owner",
            root.join("standalone"),
        )
        .with_runtime_policy(EffectiveRuntimePolicy {
            deployment: DeploymentMode::LocalSingleUser,
            requested_profile: RuntimeProfile::LocalHost,
            resolved_profile: RuntimeProfile::LocalHost,
            filesystem_backend: FilesystemBackendKind::HostWorkspace,
            process_backend: ProcessBackendKind::LocalHost,
            network_mode: NetworkMode::DirectLogged,
            secret_mode: SecretMode::ScrubbedEnv,
            approval_policy: ApprovalPolicy::AskDestructive,
            audit_mode: AuditMode::LocalMinimal,
        }),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: "tool-prefetch-tenant".to_string(),
        agent_id: "tool-prefetch-agent".to_string(),
        source_binding_id: "tool-prefetch-source".to_string(),
        reply_target_binding_id: "tool-prefetch-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(10),
    })
    .with_model_gateway_override(Arc::new(ReplyGateway))
}

/// The configuration crate counts the same mandatory floor the loop host
/// advertises, so its startup check and the runtime agree on what fits.
#[test]
fn the_config_floor_is_the_loop_host_floor() {
    assert_eq!(
        ironclaw_config::TOOL_PREFETCH_MANDATORY_FLOOR,
        ironclaw_loop_host::TOOL_PREFETCH_MANDATORY_FLOOR
    );
}

/// The configuration crate's range checks on the context bounds are the
/// loop host's own, so a value that passes startup is never refused by
/// the loop host.
#[test]
fn the_config_context_bounds_are_the_loop_contract_bounds() {
    assert_eq!(
        ironclaw_config::MAX_TOOL_PREFETCH_CONTEXT_MESSAGES,
        ironclaw_loop_host::MAX_CONTEXT_MESSAGES
    );
    assert_eq!(
        ironclaw_config::MIN_TOOL_PREFETCH_SEGMENT_BYTES,
        ironclaw_loop_host::MIN_CONTEXT_SEGMENT_BYTES
    );
    assert_eq!(
        ironclaw_config::MAX_TOOL_PREFETCH_SEGMENT_BYTES,
        ironclaw_loop_host::MAX_CONTEXT_SEGMENT_BYTES
    );
}

#[test]
fn settings_become_the_loop_host_configuration() {
    let root = tempfile::tempdir().expect("tempdir");
    for (mode, ranking) in [
        (ToolPrefetchMode::Lexical, ToolPrefetchRanking::Lexical),
        (ToolPrefetchMode::Semantic, ToolPrefetchRanking::Semantic),
    ] {
        let input = runtime_input(root.path())
            .with_tool_prefetch(settings(mode, 40, &["outbound_deliver"]))
            .expect("valid settings");
        let config = input.tool_prefetch.expect("selection bound");
        assert_eq!(config.ranking(), ranking);
        assert_eq!(config.max_tools(), 40);
        let reselection = config.reselection();
        assert!(reselection.enabled(), "re-selection is on with selection");
        assert_eq!(reselection.cache_lifetime(), Duration::from_secs(3_600));
        assert_eq!(reselection.cache_margin(), Duration::from_secs(60));
        assert_eq!(config.context_messages(), 16);
        assert_eq!(config.segment_bytes(), 2_048);
        assert_eq!(config.min_relative(), 0.0);
        assert_eq!(config.token_budget(), 32_000);
    }

    let mut off = settings(ToolPrefetchMode::Lexical, 40, &[]);
    off.reselection.enabled = false;
    off.reselection.cache_lifetime_secs = 5;
    off.context_messages = 4;
    off.segment_bytes = 512;
    let config = runtime_input(root.path())
        .with_tool_prefetch(off)
        .expect("valid settings")
        .tool_prefetch
        .expect("selection bound");
    assert!(!config.reselection().enabled());
    assert_eq!(
        config.reselection().cache_lifetime(),
        Duration::from_secs(5)
    );
    assert_eq!(
        (config.context_messages(), config.segment_bytes()),
        (4, 512)
    );

    let refused = runtime_input(root.path())
        .with_tool_prefetch(settings(
            ToolPrefetchMode::Lexical,
            4,
            &["outbound_deliver"],
        ))
        .err()
        .expect("a floor of five does not fit in four");
    assert!(matches!(
        refused,
        ToolPrefetchConfigError::FloorExceedsMaxTools { floor: 5, .. }
    ));
}

/// Selection narrows the advertised tools and leans on the discovery bridges
/// for everything else, so the runtime refuses it with disclosure off rather
/// than silently serving an unreachable catalog.
#[tokio::test]
async fn the_runtime_build_refuses_selection_without_tool_disclosure() {
    Box::pin(async {
        let root = tempfile::tempdir().expect("tempdir");
        let input = runtime_input(root.path())
            .with_tool_disclosure(ToolDisclosureMode::Off)
            .with_tool_prefetch(settings(ToolPrefetchMode::Lexical, 100, &[]))
            .expect("valid settings");
        let error = match build_reborn_runtime(input).await {
            Ok(_) => panic!("selection without disclosure must refuse to build"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("needs tool disclosure"), "{error}");
    })
    .await;
}

/// Semantic selection ranks with the bound `tool_search` ranker; with only
/// the native BM25F ranker bound the runtime refuses to build.
#[tokio::test]
async fn the_runtime_build_refuses_semantic_selection_over_the_native_ranker() {
    Box::pin(async {
        let root = tempfile::tempdir().expect("tempdir");
        let input = runtime_input(root.path())
            .with_tool_disclosure(ToolDisclosureMode::Namespaces)
            .with_tool_prefetch(settings(ToolPrefetchMode::Semantic, 100, &[]))
            .expect("valid settings");
        let error = match build_reborn_runtime(input).await {
            Ok(_) => panic!("semantic selection over BM25F must refuse to build"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("dense or hybrid"), "{error}");
    })
    .await;
}

/// A classifier that is never asked: the build only has to bind it.
#[derive(Debug)]
struct UnusedClassifier;

#[async_trait]
impl ironclaw_loop_contracts::ToolSelectionClassifier for UnusedClassifier {
    fn classifier_name(&self) -> &str {
        "unused"
    }

    async fn classify(
        &self,
        _request: &ironclaw_loop_contracts::ToolSelectionRequest,
    ) -> Result<ironclaw_loop_contracts::ToolSelection, ironclaw_loop_contracts::ToolSelectionError>
    {
        Err(ironclaw_loop_contracts::ToolSelectionError::RateLimited)
    }
}

/// Settings naming a hosted classifier bind only together with it: without
/// one the local classifier must not run in its place.
#[tokio::test]
async fn a_bound_classifier_replaces_the_local_one_and_is_never_silently_dropped() {
    Box::pin(async {
        let root = tempfile::tempdir().expect("tempdir");
        let mut jev = settings(ToolPrefetchMode::Semantic, 100, &[]);
        jev.classifier = ToolSelectionClassifierSettings::Jev(JevSettings {
            endpoint: ironclaw_config::DEFAULT_JEV_ENDPOINT.to_string(),
            model: "jev-latest".to_string(),
            api_key_env: "TYPESAFE_API_KEY".to_string(),
            timeout_ms: 500,
        });
        let refused = runtime_input(root.path())
            .with_tool_prefetch(jev.clone())
            .err()
            .expect("jev settings need the jev classifier");
        assert!(matches!(
            refused,
            ToolPrefetchConfigError::ClassifierNotBound { ref name } if name == "jev"
        ));

        let input = runtime_input(root.path())
            .with_tool_disclosure(ToolDisclosureMode::Namespaces)
            .with_tool_prefetch_classifier(jev, UnusedClassifier)
            .expect("valid settings");
        assert_eq!(
            input
                .tool_prefetch
                .as_ref()
                .map(|config| config.classifier_name()),
            Some("unused")
        );
        // A bound classifier does not rank, so semantic mode builds over the
        // native ranker.
        assert!(build_reborn_runtime(input).await.is_ok());
    })
    .await;
}
