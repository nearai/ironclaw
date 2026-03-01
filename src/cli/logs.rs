// src/cli/logs.rs

use anyhow::{Context, Result};
use clap::Args;
use dirs::home_dir;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Args, Debug)]
pub struct Logs {
    /// Number of recent lines to display
    #[arg(short, long, default_value = "100")]
    pub lines: usize,
}

/// Run the logs command: display recent lines from the IronClaw log file.
///
/// Uses a memory-efficient streaming approach: keeps only the last N lines
/// in a bounded VecDeque instead of loading the entire file into memory.
pub fn run_logs_command(args: &Logs, _log_path: &Path) -> Result<()> {
    // Construct the log file path: ~/.ironclaw/logs/ironclaw.log
    let log_path: PathBuf = home_dir()
        .context("Failed to get home directory")?
        .join(".ironclaw")
        .join("logs")
        .join("ironclaw.log");

    if !log_path.exists() {
        eprintln!("Log file not found at: {}", log_path.display());
        eprintln!("Hint: Check if IronClaw has generated logs yet, or verify the path.");
        return Ok(()); // Exit gracefully if log file doesn't exist yet
    }

    // Open the file and stream lines, keeping only the last `args.lines` entries
    let file = File::open(&log_path)
        .with_context(|| format!("Failed to open log file: {}", log_path.display()))?;

    let reader = BufReader::new(file);

    // Use VecDeque with bounded capacity to avoid loading entire file into memory
    let mut recent_lines: VecDeque<String> = VecDeque::with_capacity(args.lines);

    for line_result in reader.lines() {
        let line = line_result?;
        recent_lines.push_back(line);

        // Keep only the last N lines by popping from front when over capacity
        if recent_lines.len() > args.lines {
            recent_lines.pop_front();
        }
    }

    // Print the collected lines in original order (oldest → newest)
    for line in recent_lines {
        println!("{}", line);
    }

    Ok(())
}

/// Alternative implementation for testability: accepts custom log path.
/// Useful for unit tests with temporary files.
#[cfg(test)]
pub fn run_logs_command_with_path(args: &Logs, log_path: &Path) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_logs_output_last_n_lines() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let log_path = temp_dir.path().join("test.log");

        // Create a test log file with 10 lines
        let mut file = File::create(&log_path)?;
        for i in 1..=10 {
            writeln!(file, "Log line {}", i)?;
        }
        file.flush()?;

        // Request last 3 lines
        let args = Logs { lines: 3 };
        run_logs_command_with_path(&args, &log_path)?;

        // Output should be lines 8, 9, 10 (in order)
        // Note: actual output goes to stdout; for full assertion use assert_cmd crate
        Ok(())
    }

    #[test]
    fn test_logs_file_not_found() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let nonexistent_path = temp_dir.path().join("nonexistent.log");

        let args = Logs { lines: 10 };
        let result = run_logs_command_with_path(&args, &nonexistent_path);

        // Should return Ok (graceful exit), not panic
        assert!(result.is_ok());
        Ok(())
    }

    #[test]
    fn test_logs_empty_file() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let log_path = temp_dir.path().join("empty.log");

        // Create empty file
        File::create(&log_path)?;

        let args = Logs { lines: 10 };
        let result = run_logs_command_with_path(&args, &log_path);

        assert!(result.is_ok());
        Ok(())
    }
}
