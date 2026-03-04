//! CLI for the benchmark runner.

use std::path::PathBuf;

use clap::Args;

use crate::benchmark::baseline::{load_baseline, promote_to_baseline, save_scenario_results};
use crate::benchmark::report::format_report;
use crate::benchmark::runner::{BenchmarkConfig, run_all_bench};
use crate::config::Config;
use crate::llm::{SessionConfig, build_provider_chain, create_session_manager};

#[derive(Args, Debug)]
pub struct BenchmarkCommand {
    /// Directory containing scenario JSON files
    #[arg(long, default_value = "benchmarks/trajectories")]
    pub scenarios_dir: PathBuf,

    /// Filter scenarios by tag (comma-separated, scenario must match at least one)
    #[arg(long, value_delimiter = ',')]
    pub tags: Option<Vec<String>>,

    /// Filter to a single scenario by name (substring match)
    #[arg(long)]
    pub scenario: Option<String>,

    /// Skip LLM-as-judge scoring (assertions only)
    #[arg(long)]
    pub no_judge: bool,

    /// Global timeout per scenario in seconds
    #[arg(long, default_value = "120")]
    pub timeout: u64,

    /// Save results as the new baseline
    #[arg(long)]
    pub update_baseline: bool,

    /// Number of scenarios to run in parallel (default: 1 = sequential)
    #[arg(long, default_value = "1")]
    pub parallel: usize,

    /// Maximum total cost in USD across all scenarios; abort remaining if exceeded
    #[arg(long)]
    pub max_cost: Option<f64>,

    /// Output results as JSON (machine-readable) instead of human-readable report
    #[arg(long)]
    pub json: bool,
}

pub async fn run_benchmark_command(cmd: &BenchmarkCommand) -> anyhow::Result<()> {
    let config = Config::from_env()
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let session = create_session_manager(SessionConfig {
        auth_base_url: config.llm.nearai.auth_base_url.clone(),
        session_path: config.llm.nearai.session_path.clone(),
    })
    .await;

    let (llm, _cheap_llm) = build_provider_chain(&config.llm, session)
        .map_err(|e| anyhow::anyhow!("Failed to create LLM provider: {}", e))?;

    let bench_config = BenchmarkConfig {
        scenarios_dir: cmd.scenarios_dir.clone(),
        global_timeout_secs: cmd.timeout,
        filter: cmd.scenario.clone(),
        category_filter: None,
        tags_filter: cmd.tags.clone(),
        parallel: cmd.parallel,
        max_total_cost_usd: cmd.max_cost,
    };

    let run_result = run_all_bench(&bench_config, llm)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    // Save results to disk (per-scenario files + summary).
    let result_dir = save_scenario_results(&run_result).map_err(|e| anyhow::anyhow!("{}", e))?;
    eprintln!("Results saved to: {}/", result_dir);

    if cmd.json {
        // Machine-readable JSON output to stdout.
        let json = serde_json::to_string_pretty(&run_result)
            .map_err(|e| anyhow::anyhow!("Failed to serialize results: {}", e))?;
        println!("{json}");
    } else {
        // Human-readable report with baseline comparison.
        let baseline = load_baseline().map_err(|e| anyhow::anyhow!("{}", e))?;
        let report = format_report(&run_result, baseline.as_ref());
        println!("{report}");
    }

    // Promote to baseline if requested.
    if cmd.update_baseline {
        let summary_path = format!("{result_dir}/summary.json");
        promote_to_baseline(&summary_path).map_err(|e| anyhow::anyhow!("{}", e))?;
        eprintln!("Baseline updated.");
    }

    // Exit with code 1 if any scenario failed.
    let any_failed = run_result.scenarios.iter().any(|s| !s.passed);
    if any_failed {
        std::process::exit(1);
    }

    Ok(())
}
