use clap::Args;
use ironclaw_reborn_composition::{
    MultiAgentDemoOptions, format_demo_output, run_reborn_multi_agent_demo,
};

/// Fast in-memory recursive multi-agent demo (dynamic subagent spawning).
#[derive(Debug, Args)]
pub(crate) struct MultiAgentDemoCommand {
    /// Top-level task for the master agent run.
    #[arg(long)]
    task: String,

    /// Maximum delegation depth before forcing direct execution.
    #[arg(long = "max-depth", default_value_t = 2)]
    max_depth: u32,

    /// Maximum orchestration iterations across the delegation tree.
    #[arg(long = "max-iterations", default_value_t = 32)]
    max_iterations: u32,

    /// Print the recursive agent tree and per-run progress events.
    #[arg(long = "show-progress")]
    show_progress: bool,
}

impl MultiAgentDemoCommand {
    pub(crate) fn execute(self) -> anyhow::Result<()> {
        let result = crate::runtime::block_on_cli(run_reborn_multi_agent_demo(
            self.task,
            MultiAgentDemoOptions::new(
                self.max_depth,
                self.max_iterations,
                self.show_progress,
            ),
        ))?;

        print!("{}", format_demo_output(&result, self.show_progress));
        Ok(())
    }
}
