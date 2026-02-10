use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum BenchError {
    #[error("Config error: {0}")]
    Config(String),

    #[error("Config file not found: {path}")]
    ConfigNotFound { path: PathBuf },

    #[error("Suite {name} not found. Available: {available}")]
    SuiteNotFound { name: String, available: String },

    #[error("Task {task_id} failed: {reason}")]
    TaskFailed { task_id: String, reason: String },

    #[error("Timeout after {seconds}s for task {task_id}")]
    Timeout { task_id: String, seconds: u64 },

    #[error("Scoring error for task {task_id}: {reason}")]
    Scoring { task_id: String, reason: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("Agent error: {0}")]
    Agent(#[from] ironclaw::Error),

    #[error("Results directory error: {0}")]
    ResultsDir(String),

    #[error("Resume failed: no completed tasks found in {path}")]
    ResumeEmpty { path: PathBuf },
}
