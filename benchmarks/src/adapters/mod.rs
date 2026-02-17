pub mod custom;
pub mod gaia;
pub mod spot;
pub mod swe_bench;
pub mod tau_bench;

use crate::config::BenchConfig;
use crate::error::BenchError;
use crate::suite::BenchSuite;

/// List of all known suite IDs.
pub const KNOWN_SUITES: &[(&str, &str)] = &[
    ("custom", "Custom JSONL tasks"),
    ("gaia", "GAIA benchmark (knowledge & reasoning)"),
    ("spot", "Spot checks (end-to-end user workflows)"),
    ("tau_bench", "Tau-bench (multi-turn tool use)"),
    ("swe_bench", "SWE-bench Pro (software engineering)"),
];

/// Create a suite adapter by name.
pub fn create_suite(name: &str, config: &BenchConfig) -> Result<Box<dyn BenchSuite>, BenchError> {
    let suite_map = config.suite_config_map();
    match name {
        "custom" => {
            let dataset_path = suite_map
                .get("dataset_path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    BenchError::Config(
                        "suite_config.dataset_path is required for 'custom' suite".to_string(),
                    )
                })?;
            Ok(Box::new(custom::CustomSuite::new(dataset_path)))
        }
        "gaia" => {
            let dataset_path = suite_map
                .get("dataset_path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    BenchError::Config(
                        "suite_config.dataset_path is required for 'gaia' suite".to_string(),
                    )
                })?;
            let attachments_dir = suite_map
                .get("attachments_dir")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Ok(Box::new(gaia::GaiaSuite::new(
                dataset_path,
                attachments_dir,
            )))
        }
        "tau_bench" => {
            let dataset_path = suite_map
                .get("dataset_path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    BenchError::Config(
                        "suite_config.dataset_path is required for 'tau_bench' suite".to_string(),
                    )
                })?;
            let domain = suite_map
                .get("domain")
                .and_then(|v| v.as_str())
                .unwrap_or("retail")
                .to_string();
            Ok(Box::new(tau_bench::TauBenchSuite::new(
                dataset_path,
                domain,
            )))
        }
        "spot" => {
            let dataset_path = suite_map
                .get("dataset_path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    BenchError::Config(
                        "suite_config.dataset_path is required for 'spot' suite".to_string(),
                    )
                })?;
            Ok(Box::new(spot::SpotSuite::new(dataset_path)))
        }
        "swe_bench" => {
            let dataset_path = suite_map
                .get("dataset_path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    BenchError::Config(
                        "suite_config.dataset_path is required for 'swe_bench' suite".to_string(),
                    )
                })?;
            let workspace_dir = suite_map
                .get("workspace_dir")
                .and_then(|v| v.as_str())
                .unwrap_or("/tmp/swe-bench")
                .to_string();
            let use_docker = suite_map
                .get("use_docker")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(Box::new(swe_bench::SweBenchSuite::new(
                dataset_path,
                workspace_dir,
                use_docker,
            )))
        }
        _ => {
            let available = KNOWN_SUITES
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>()
                .join(", ");
            Err(BenchError::SuiteNotFound {
                name: name.to_string(),
                available,
            })
        }
    }
}
