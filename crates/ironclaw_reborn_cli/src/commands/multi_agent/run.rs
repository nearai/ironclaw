use std::time::Duration;

use clap::Args;
use ironclaw_reborn_composition::{
    MultiAgentRunOptions, format_run_output, run_reborn_multi_agent_task,
};

/// Run the recursive multi-agent orchestration framework.
#[derive(Debug, Args)]
pub(crate) struct MultiAgentRunCommand {
    /// Top-level task description for the master agent.
    #[arg(long)]
    task: String,

    /// Maximum delegation depth before forcing direct execution.
    #[arg(long = "max-depth", default_value_t = 3)]
    max_depth: u32,

    /// Maximum number of orchestration iterations across the delegation tree.
    #[arg(long = "max-iterations", default_value_t = 32)]
    max_iterations: u32,

    /// Per-run timeout in seconds for the master orchestration loop.
    #[arg(long = "task-timeout-secs", default_value_t = 120)]
    task_timeout_secs: u64,

    /// Maximum retries for a failed leaf job before marking it failed.
    #[arg(long = "max-retries", default_value_t = 0)]
    max_retries: u32,

    /// Show full output for every AgentRun (task, result, event log).
    #[arg(long)]
    verbose: bool,

    /// Use the configured Reborn LLM provider to execute leaf tasks instead of
    /// the placeholder executor. Requires a provider to be configured (see
    /// `ironclaw-reborn models list`).
    #[cfg(feature = "root-llm-provider")]
    #[arg(long)]
    llm: bool,
}

impl MultiAgentRunCommand {
    pub(crate) fn execute(self) -> anyhow::Result<()> {
        let options = MultiAgentRunOptions::new(
            self.max_depth,
            self.max_iterations,
            Duration::from_secs(self.task_timeout_secs),
            self.max_retries,
            false,
        );

        #[cfg(feature = "root-llm-provider")]
        if self.llm {
            let context = crate::context::RebornCliContext::resolve_from_env()?;
            let boot = context.boot_config().clone();
            let report = crate::runtime::block_on_cli(
                ironclaw_reborn_composition::run_reborn_multi_agent_task_with_llm(
                    self.task,
                    options,
                    boot,
                ),
            )?;
            print!("{}", format_run_output(&report, self.verbose));
            return Ok(());
        }

        let report = crate::runtime::block_on_cli(run_reborn_multi_agent_task(self.task, options))?;
        print!("{}", format_run_output(&report, self.verbose));
        Ok(())
    }
}
