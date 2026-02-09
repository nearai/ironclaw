mod adapters;
mod channel;
mod config;
mod error;
mod instrumented_llm;
mod results;
mod runner;
mod scoring;
mod suite;

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

use crate::config::BenchConfig;

#[derive(Parser)]
#[command(name = "ironclaw-bench", about = "IronClaw benchmarking harness")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a benchmark suite.
    Run {
        /// Suite to run (custom, gaia, tau_bench, swe_bench).
        #[arg(long)]
        suite: String,

        /// Path to bench config TOML.
        #[arg(long)]
        config: Option<PathBuf>,

        /// Override model for all matrix entries.
        #[arg(long)]
        model: Option<String>,

        /// Max tasks to run in parallel.
        #[arg(long)]
        parallelism: Option<usize>,

        /// Sample N tasks from the suite (for quick testing).
        #[arg(long)]
        sample: Option<usize>,

        /// Only run these task IDs (comma-separated).
        #[arg(long, value_delimiter = ',')]
        task_ids: Option<Vec<String>>,

        /// Only run tasks with these tags (comma-separated).
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,

        /// Per-task timeout in seconds.
        #[arg(long)]
        timeout_secs: Option<u64>,

        /// Override results directory.
        #[arg(long)]
        results_dir: Option<PathBuf>,

        /// Resume a previous run by ID.
        #[arg(long)]
        resume: Option<Uuid>,
    },

    /// Show results for a run.
    Results {
        /// Run ID or "latest".
        #[arg(default_value = "latest")]
        run_id: String,

        /// Output format.
        #[arg(long, default_value = "table")]
        format: ResultsFormat,

        /// Override results directory.
        #[arg(long)]
        results_dir: Option<PathBuf>,
    },

    /// Compare two runs.
    Compare {
        /// Baseline run ID.
        baseline: Uuid,

        /// Comparison run ID.
        comparison: Uuid,

        /// Override results directory.
        #[arg(long)]
        results_dir: Option<PathBuf>,
    },

    /// List available benchmark suites.
    List,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum ResultsFormat {
    Table,
    Json,
    Csv,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("ironclaw_bench=info,ironclaw=warn")),
        )
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .init();

    match cli.command {
        Commands::List => {
            println!("Available benchmark suites:\n");
            for (id, desc) in adapters::KNOWN_SUITES {
                println!("  {:<15} {}", id, desc);
            }
            println!();
        }
        Commands::Run {
            suite,
            config: config_path,
            model,
            parallelism,
            sample,
            task_ids,
            tags,
            timeout_secs,
            results_dir,
            resume,
        } => {
            // Load or create config
            let mut bench_config = if let Some(ref path) = config_path {
                BenchConfig::from_file(path)?
            } else {
                BenchConfig::minimal(model.clone())
            };

            // Apply CLI overrides
            if let Some(p) = parallelism {
                bench_config.parallelism = p;
            }
            if let Some(t) = timeout_secs {
                bench_config.task_timeout = std::time::Duration::from_secs(t);
            }
            if let Some(ref dir) = results_dir {
                bench_config.results_dir = dir.clone();
            }

            // If model override specified and we have matrix entries, update them
            if let Some(ref m) = model {
                for entry in &mut bench_config.matrix {
                    entry.model = Some(m.clone());
                }
            }

            // Create suite
            let bench_suite = adapters::create_suite(&suite, &bench_config)?;

            // Initialize ironclaw LLM provider
            let ironclaw_config = ironclaw::Config::from_env().await.map_err(|e| {
                anyhow::anyhow!(
                    "Failed to load ironclaw config: {}. Make sure .env is configured.",
                    e
                )
            })?;

            let session = ironclaw::llm::create_session_manager(ironclaw::llm::SessionConfig {
                auth_base_url: ironclaw_config.llm.nearai.auth_base_url.clone(),
                session_path: ironclaw_config.llm.nearai.session_path.clone(),
                ..Default::default()
            })
            .await;
            session.ensure_authenticated().await?;

            let llm = ironclaw::llm::create_llm_provider(&ironclaw_config.llm, session)?;
            let safety = Arc::new(ironclaw::safety::SafetyLayer::new(&ironclaw_config.safety));

            let runner = runner::BenchRunner::new(bench_suite, bench_config.clone(), llm, safety);

            // Run for each matrix entry
            for matrix_entry in &bench_config.matrix {
                let run_id = runner
                    .run(
                        matrix_entry,
                        sample,
                        task_ids.as_deref(),
                        tags.as_deref(),
                        resume,
                    )
                    .await?;
                println!("Run complete: {}", run_id);
            }
        }
        Commands::Results {
            run_id,
            format,
            results_dir,
        } => {
            let base = results_dir.unwrap_or_else(|| PathBuf::from("./bench-results"));
            let uuid = if run_id == "latest" {
                results::find_latest_run(&base)?
                    .ok_or_else(|| anyhow::anyhow!("No runs found in {}", base.display()))?
            } else {
                Uuid::parse_str(&run_id)?
            };

            let json_path = results::run_json_path(&base, uuid);
            let jsonl_path = results::tasks_jsonl_path(&base, uuid);

            let run = results::read_run_result(&json_path)?;
            let tasks = results::read_task_results(&jsonl_path)?;

            match format {
                ResultsFormat::Table => {
                    results::print_results_table(&tasks, &run);
                }
                ResultsFormat::Json => {
                    let output = serde_json::json!({
                        "run": run,
                        "tasks": tasks,
                    });
                    println!("{}", serde_json::to_string_pretty(&output)?);
                }
                ResultsFormat::Csv => {
                    println!("task_id,score,label,tokens,cost,turns,time_s");
                    for task in &tasks {
                        println!(
                            "{},{:.3},{},{},{:.4},{},{:.1}",
                            task.task_id,
                            task.score.value,
                            task.score.label,
                            task.trace.input_tokens + task.trace.output_tokens,
                            task.trace.estimated_cost_usd,
                            task.trace.turns,
                            task.trace.wall_time_ms as f64 / 1000.0,
                        );
                    }
                }
            }
        }
        Commands::Compare {
            baseline,
            comparison,
            results_dir,
        } => {
            let base = results_dir.unwrap_or_else(|| PathBuf::from("./bench-results"));

            let baseline_run = results::read_run_result(&results::run_json_path(&base, baseline))?;
            let comparison_run =
                results::read_run_result(&results::run_json_path(&base, comparison))?;

            println!("\nComparison: {} vs {}\n", baseline, comparison);
            println!(
                "{:<20} {:>12} {:>12} {:>10}",
                "Metric", "Baseline", "Comparison", "Delta"
            );
            println!("{}", "-".repeat(58));

            let pass_delta = comparison_run.pass_rate - baseline_run.pass_rate;
            println!(
                "{:<20} {:>11.1}% {:>11.1}% {:>+9.1}%",
                "Pass rate",
                baseline_run.pass_rate * 100.0,
                comparison_run.pass_rate * 100.0,
                pass_delta * 100.0,
            );

            let score_delta = comparison_run.avg_score - baseline_run.avg_score;
            println!(
                "{:<20} {:>12.3} {:>12.3} {:>+10.3}",
                "Avg score", baseline_run.avg_score, comparison_run.avg_score, score_delta,
            );

            let cost_delta = comparison_run.total_cost_usd - baseline_run.total_cost_usd;
            println!(
                "{:<20} {:>11.4}$ {:>11.4}$ {:>+9.4}$",
                "Total cost",
                baseline_run.total_cost_usd,
                comparison_run.total_cost_usd,
                cost_delta,
            );

            let time_b = baseline_run.total_wall_time_ms as f64 / 1000.0;
            let time_c = comparison_run.total_wall_time_ms as f64 / 1000.0;
            println!(
                "{:<20} {:>11.1}s {:>11.1}s {:>+9.1}s",
                "Total time",
                time_b,
                time_c,
                time_c - time_b,
            );

            println!(
                "{:<20} {:>12} {:>12}",
                "Model", baseline_run.model, comparison_run.model,
            );
            println!();
        }
    }

    Ok(())
}
