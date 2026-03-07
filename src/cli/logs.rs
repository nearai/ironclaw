//! CLI command for viewing recent log entries.
//!
//! Provides `ironclaw logs --lines N` to tail the application log file.
//!
use anyhow::{Context, Result};
use clap::Args;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Args, Debug)]
pub struct Logs {
    /// Number of recent lines to display
    ///
    /// Default: 100 (matches `tail -n 100` convention for debugging)
    #[arg(short, long, default_value = "100")]
    pub lines: usize,
}

/// Core logic: read last N lines from a file using memory-efficient VecDeque.
/// Returns lines in original order (oldest → newest).
///
/// This function is used by both `run_logs_command` (production) and tests,
/// ensuring tests verify the actual production code path.
fn tail_lines(log_path: &Path, n: usize) -> Result<Vec<String>> {
    if !log_path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(log_path)
        .with_context(|| format!("Failed to open log file: {}", log_path.display()))?;

    let reader = BufReader::new(file);
    let mut recent_lines: VecDeque<String> = VecDeque::with_capacity(n);

    for line_result in reader.lines() {
        let line = line_result?;
        recent_lines.push_back(line);
        if recent_lines.len() > n {
            recent_lines.pop_front();
        }
    }

    Ok(recent_lines.into_iter().collect())
}

/// CLI entry point: stream last N lines from log file to stdout.
pub fn run_logs_command(args: &Logs, log_path: &Path) -> Result<()> {
    let lines = tail_lines(log_path, args.lines)?;

    if lines.is_empty() && !log_path.exists() {
        eprintln!("Log file not found at: {}", log_path.display());
        eprintln!("Hint: Check if IronClaw has generated logs yet, or verify the path.");
        return Ok(());
    }

    for line in lines {
        println!("{}", line);
    }

    Ok(())
}

// ✅ Tests use the same tail_lines function as production code
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_logs_output_last_n_lines() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let log_path = temp_dir.path().join("test.log");

        let mut file = File::create(&log_path)?;
        for i in 1..=10 {
            writeln!(file, "Log line {}", i)?;
        }
        file.flush()?;

        let args = Logs { lines: 3 };
        let result = tail_lines(&log_path, args.lines)?;

        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "Log line 8");
        assert_eq!(result[1], "Log line 9");
        assert_eq!(result[2], "Log line 10");

        Ok(())
    }

    #[test]
    fn test_logs_file_not_found() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let nonexistent_path = temp_dir.path().join("nonexistent.log");

        let args = Logs { lines: 10 };
        let result = tail_lines(&nonexistent_path, args.lines);

        assert!(result.is_ok());
        assert!(result?.is_empty());
        Ok(())
    }

    #[test]
    fn test_logs_empty_file() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let log_path = temp_dir.path().join("empty.log");
        File::create(&log_path)?;

        let args = Logs { lines: 10 };
        let result = tail_lines(&log_path, args.lines);

        assert!(result.is_ok());
        assert!(result?.is_empty());
        Ok(())
    }

    /// Regression test: verifies log_path parameter is used (not hardcoded).
    #[test]
    fn test_logs_uses_injected_path_regression() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let custom_path = temp_dir.path().join("unique_test.log");

        let mut file = File::create(&custom_path)?;
        writeln!(file, "UNIQUE_MARKER_XYZ789")?;
        file.flush()?;

        let args = Logs { lines: 1 };
        let result = tail_lines(&custom_path, args.lines)?;

        assert_eq!(result.len(), 1);
        assert!(result[0].contains("UNIQUE_MARKER_XYZ789"));

        Ok(())
    }
}
