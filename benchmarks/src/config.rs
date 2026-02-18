use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::error::BenchError;

/// Top-level bench configuration, loaded from TOML.
#[derive(Debug, Clone, Deserialize)]
pub struct BenchConfig {
    /// Where to write results. Default: "./bench-results".
    #[serde(default = "default_results_dir")]
    pub results_dir: PathBuf,

    /// Per-task timeout. Default: "300s".
    #[serde(
        default = "default_task_timeout",
        deserialize_with = "deserialize_duration"
    )]
    pub task_timeout: Duration,

    /// How many tasks to run in parallel. Default: 1.
    #[serde(default = "default_parallelism")]
    pub parallelism: usize,

    /// Model/config matrix entries. At least one required.
    #[serde(default)]
    pub matrix: Vec<MatrixEntry>,

    /// Max agentic loop iterations per task. Default: 30.
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,

    /// Suite-specific configuration (passed through to adapter).
    #[serde(default = "default_suite_config")]
    pub suite_config: toml::Value,
}

/// A single model/config combination to benchmark.
#[derive(Debug, Clone, Deserialize)]
pub struct MatrixEntry {
    /// Label for this configuration (used in results).
    pub label: String,

    /// Model identifier.
    #[serde(default)]
    pub model: Option<String>,
}

impl BenchConfig {
    /// Load from a TOML file.
    pub fn from_file(path: &Path) -> Result<Self, BenchError> {
        if !path.exists() {
            return Err(BenchError::ConfigNotFound {
                path: path.to_path_buf(),
            });
        }
        let content = std::fs::read_to_string(path)?;
        let config: BenchConfig = toml::from_str(&content)?;
        if config.matrix.is_empty() {
            return Err(BenchError::Config(
                "config must have at least one [[matrix]] entry".to_string(),
            ));
        }
        Ok(config)
    }

    /// Create a minimal config for when no config file is provided.
    /// Uses defaults and optional CLI overrides.
    pub fn minimal(model: Option<String>) -> Self {
        let label = model.as_deref().unwrap_or("default").to_string();
        Self {
            results_dir: default_results_dir(),
            task_timeout: default_task_timeout(),
            parallelism: default_parallelism(),
            max_iterations: default_max_iterations(),
            matrix: vec![MatrixEntry { label, model }],
            suite_config: toml::Value::Table(toml::map::Map::new()),
        }
    }

    /// Get the suite_config as a generic map for adapter use.
    pub fn suite_config_map(&self) -> toml::map::Map<String, toml::Value> {
        match &self.suite_config {
            toml::Value::Table(map) => map.clone(),
            _ => toml::map::Map::new(),
        }
    }

    /// Get a string value from suite_config.
    pub fn suite_config_str(&self, key: &str) -> Option<String> {
        self.suite_config_map()
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
}

fn default_suite_config() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

fn default_results_dir() -> PathBuf {
    PathBuf::from("./bench-results")
}

fn default_task_timeout() -> Duration {
    Duration::from_secs(300)
}

fn default_parallelism() -> usize {
    1
}

fn default_max_iterations() -> usize {
    30
}

/// Deserialize a duration from a string like "300s", "5m", etc.
fn deserialize_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    parse_duration(&s).map_err(serde::de::Error::custom)
}

fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    if let Some(secs) = s.strip_suffix('s') {
        secs.trim()
            .parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|e| format!("invalid seconds: {e}"))
    } else if let Some(mins) = s.strip_suffix('m') {
        mins.trim()
            .parse::<u64>()
            .map(|m| Duration::from_secs(m * 60))
            .map_err(|e| format!("invalid minutes: {e}"))
    } else {
        // Assume seconds if no suffix
        s.parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|e| format!("invalid duration '{s}': {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("300s").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("60").unwrap(), Duration::from_secs(60));
    }

    #[test]
    fn test_minimal_config() {
        let config = BenchConfig::minimal(Some("test-model".to_string()));
        assert_eq!(config.matrix.len(), 1);
        assert_eq!(config.matrix[0].label, "test-model");
        assert_eq!(config.parallelism, 1);
    }

    #[test]
    fn test_config_rejects_empty_matrix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.toml");
        std::fs::write(
            &path,
            r#"
results_dir = "./results"
task_timeout = "60s"
"#,
        )
        .unwrap();
        let err = BenchConfig::from_file(&path).unwrap_err();
        assert!(
            err.to_string().contains("at least one [[matrix]]"),
            "got: {err}"
        );
    }

    #[test]
    fn test_config_from_toml() {
        let toml_str = r#"
results_dir = "./my-results"
task_timeout = "60s"
parallelism = 2

[[matrix]]
label = "fast"
model = "gpt-4o-mini"

[[matrix]]
label = "full"
model = "claude-3-5-sonnet"

[suite_config]
dataset_path = "./data/test.jsonl"
"#;
        let config: BenchConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.results_dir, PathBuf::from("./my-results"));
        assert_eq!(config.task_timeout, Duration::from_secs(60));
        assert_eq!(config.parallelism, 2);
        assert_eq!(config.matrix.len(), 2);
        assert_eq!(
            config.suite_config_str("dataset_path").unwrap(),
            "./data/test.jsonl"
        );
    }
}
