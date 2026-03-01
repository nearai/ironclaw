use anyhow::{Context, Result};
use clap::Args;
use dirs::home_dir;
use std::fs::File;
use std::io::{self, BufRead};
use std::path::PathBuf;

/// View recent logs
#[derive(Args, Debug, Clone)]
pub struct Logs {
    /// Number of lines to show (default: 50)
    #[arg(short = 'n', long, default_value = "50")]
    pub lines: usize,
}

pub fn run_logs_command(args: &Logs) -> Result<()> {
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

    let file = File::open(&log_path)
        .with_context(|| format!("Failed to open log file: {}", log_path.display()))?;
    let all_lines: Vec<String> = io::BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .collect();
    let recent_lines: Vec<String> = all_lines.into_iter().rev().take(args.lines).collect();
    for line in recent_lines.into_iter().rev() {
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
    fn test_logs_output() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let log_path = temp_dir.path().join("test.log");
        let mut file = File::create(&log_path)?;
        writeln!(file, "Line1")?;
        writeln!(file, "Line2")?;
        writeln!(file, "Line3")?;
        file.flush()?;

        let args = Logs { lines: 2 };
        run_logs_command(&args)?;
        Ok(())
    }
}
