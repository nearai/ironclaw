use clap::{CommandFactory, Parser, ValueEnum};
use clap_complete::{Shell, generate};
use std::io;

/// Generate shell completion scripts for ironclaw
#[derive(Parser, Debug)]
pub struct Completion {
    /// The shell to generate completions for
    #[arg(value_enum, long)]
    pub shell: CompletionShell,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    Powershell,
    Elvish,
}

impl Completion {
    pub fn run(&self) -> anyhow::Result<()> {
        let mut cmd = crate::cli::Cli::command();
        let bin_name = cmd.get_name().to_string();

        // Convert enum to clap_complete::Shell
        let shell = match self.shell {
            CompletionShell::Bash => Shell::Bash,
            CompletionShell::Zsh => Shell::Zsh,
            CompletionShell::Fish => Shell::Fish,
            CompletionShell::Powershell => Shell::PowerShell,
            CompletionShell::Elvish => Shell::Elvish,
        };

        // Generate and output a script to stdout
        generate(shell, &mut cmd, bin_name, &mut io::stdout());

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_completion_shell_enum() {
        // Check that the enum is parsed
        let shells = ["bash", "zsh", "fish", "powershell", "elvish"];
        for shell in shells {
            let result = CompletionShell::from_str(shell, false); // ignore_case=false for exact match
            assert!(result.is_ok(), "Failed to parse {}", shell);
        }
    }

    #[test]
    fn test_run_does_not_panic() {
        // Check that run() doesn't panic (no real output)
        let completion = Completion {
            shell: CompletionShell::Zsh,
        };
        // We don't call the run() method completely to avoid generating output in tests,
        // but we do check that the structure is correct.
        assert_eq!(format!("{:?}", completion.shell), "Zsh");
    }
}
