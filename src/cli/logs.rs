//! CLI command for viewing recent log entries.
//!
//! Provides `ironclaw logs --lines N` to tail the application log file.

use anyhow::{Context, Result};
use clap::Args;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Args, Debug)]
pub struct Logs {
    /// Number of recent lines to display
    #[arg(short, long, default_value = "100")]
    pub lines: usize,
}

/// Core implementation: stream last N lines from any log file path.
/// Uses memory-efficient VecDeque to avoid loading entire file.
pub fn run_logs_command(args: &Logs, log_path: &Path) -> Result<()> {
    if !log_path.exists() {
        eprintln!("Log file not found at: {}", log_path.display());
        eprintln!("Hint: Check if IronClaw has generated logs yet, or verify the path.");
        return Ok(());
    }

    let file = File::open(log_path)
        .with_context(|| format!("Failed to open log file: {}", log_path.display()))?;

    let reader = BufReader::new(file);
    let mut recent_lines: VecDeque<String> = VecDeque::with_capacity(args.lines);

    for line_result in reader.lines() {
        let line = line_result?;
        recent_lines.push_back(line);
        if recent_lines.len() > args.lines {
            recent_lines.pop_front();
        }
    }

    for line in recent_lines {
        println!("{}", line);
    }

    Ok(())
}

// ✅ Helper for tests: defined BEFORE mod tests
#[cfg(test)]
fn read_last_n_lines(log_path: &Path, n: usize) -> Result<Vec<String>> {
    if !log_path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(log_path)?;
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

// ✅ Tests defined AFTER all helper functions
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
        let result = read_last_n_lines(&log_path, args.lines)?;

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
        let result = run_logs_command(&args, &nonexistent_path);

        assert!(result.is_ok());
        Ok(())
    }

    #[test]
    fn test_logs_empty_file() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let log_path = temp_dir.path().join("empty.log");
        File::create(&log_path)?;

        let args = Logs { lines: 10 };
        let result = run_logs_command(&args, &log_path);

        assert!(result.is_ok());
        Ok(())
    }
}
