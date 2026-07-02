use std::time::Duration;

use ironclaw_reborn::multi_agent::{
    AgentDemoConfig, AgentDemoResult, MultiAgentRunConfig, format_demo_progress,
    run_multi_agent_demo, run_multi_agent_task_with_config,
};

pub use ironclaw_reborn::multi_agent::{MultiAgentError, MultiAgentRunReport, format_run_output};

/// CLI-facing options for the recursive multi-agent job runtime.
#[derive(Debug, Clone)]
pub struct MultiAgentRunOptions {
    pub max_depth: u32,
    pub max_iterations: u32,
    pub task_timeout: Duration,
    pub max_retries: u32,
    pub show_progress: bool,
}

impl MultiAgentRunOptions {
    pub fn new(
        max_depth: u32,
        max_iterations: u32,
        task_timeout: Duration,
        max_retries: u32,
        show_progress: bool,
    ) -> Self {
        Self {
            max_depth,
            max_iterations,
            task_timeout,
            max_retries,
            show_progress,
        }
    }
}

/// CLI-facing entrypoint for the recursive multi-agent orchestrator.
pub async fn run_reborn_multi_agent_task(
    task: impl Into<String>,
    options: MultiAgentRunOptions,
) -> Result<MultiAgentRunReport, MultiAgentError> {
    run_multi_agent_task_with_config(
        task,
        MultiAgentRunConfig::new(
            options.max_depth,
            options.max_iterations,
            options.task_timeout,
            options.max_retries,
        ),
    )
    .await
}

/// Resolve the currently-active Reborn LLM provider, build it, and run the
/// recursive multi-agent job runtime with a real model executing every leaf
/// task instead of the placeholder.
///
/// This is the "live" path. The planner still splits tasks heuristically
/// (semicolons, " and ", length), but each resulting atomic task is sent
/// to `provider.complete()` as a single-turn completion so the model
/// produces the actual output.
#[cfg(feature = "root-llm-provider")]
pub async fn run_reborn_multi_agent_task_with_llm(
    task: impl Into<String>,
    options: MultiAgentRunOptions,
    boot: ironclaw_reborn_config::RebornBootConfig,
) -> Result<MultiAgentRunReport, MultiAgentError> {
    use std::sync::Arc;

    use ironclaw_reborn::multi_agent::{
        HeuristicDelegationPlanner, LlmTaskExecutor, run_multi_agent_jobs_with,
    };

    // Resolve the active LLM from the boot config / config.toml / env.
    let config_path = boot.home().config_file_path();
    let config_file = ironclaw_reborn_config::RebornConfigFile::load(config_path.as_path())
        .ok()
        .flatten();
    let resolved = crate::resolve_reborn_runtime_llm(&boot, config_file.as_ref())
        .map_err(|error| MultiAgentError::OrchestrationFailed {
            reason: format!("LLM provider resolution failed: {error}"),
        })?
        .ok_or_else(|| MultiAgentError::OrchestrationFailed {
            reason: "No LLM provider configured. Run `ironclaw-reborn models list` \
                     to check configuration."
                .to_string(),
        })?;

    // Build the full static provider chain (retry / circuit breaker / cache).
    let session =
        ironclaw_llm::create_session_manager(resolved.config.session.clone()).await;
    let provider = ironclaw_llm::build_static_provider_chain(&resolved.config, session)
        .await
        .map_err(|error| MultiAgentError::OrchestrationFailed {
            reason: format!("LLM provider build failed: {error}"),
        })?;

    let planner = Arc::new(HeuristicDelegationPlanner);
    let executor = Arc::new(LlmTaskExecutor::new(provider));

    run_multi_agent_jobs_with(
        task,
        MultiAgentRunConfig::new(
            options.max_depth,
            options.max_iterations,
            options.task_timeout,
            options.max_retries,
        ),
        planner,
        executor,
    )
    .await
}

/// CLI-facing options for the fast in-memory multi-agent demo.
#[derive(Debug, Clone)]
pub struct MultiAgentDemoOptions {
    pub max_depth: u32,
    pub max_iterations: u32,
    pub show_progress: bool,
}

impl MultiAgentDemoOptions {
    pub fn new(max_depth: u32, max_iterations: u32, show_progress: bool) -> Self {
        Self {
            max_depth,
            max_iterations,
            show_progress,
        }
    }
}

/// CLI-facing entrypoint for the recursive multi-agent demo runtime.
pub async fn run_reborn_multi_agent_demo(
    task: impl Into<String>,
    options: MultiAgentDemoOptions,
) -> Result<AgentDemoResult, MultiAgentError> {
    run_multi_agent_demo(
        task,
        AgentDemoConfig::new(options.max_depth, options.max_iterations)
            .with_live_events(options.show_progress),
    )
    .await
}

pub fn format_demo_output(result: &AgentDemoResult, show_progress: bool) -> String {
    if show_progress {
        format!("\n{}\n", format_demo_progress(result))
    } else {
        format!("Final result:\n{}\n", result.final_result)
    }
}
