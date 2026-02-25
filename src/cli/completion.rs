use clap::{CommandFactory, Parser};
use clap_complete::{Shell, generate};
use std::io;

/// Generate shell completion scripts for clawyer
#[derive(Parser, Debug)]
pub struct Completion {
    /// The shell to generate completions for
    #[arg(value_enum, long)]
    pub shell: Shell,
}

impl Completion {
    pub fn run(&self) -> anyhow::Result<()> {
        let mut cmd = crate::cli::Cli::command();
        let bin_name = cmd.get_name().to_string();

        // Generated and output script to stdout
        generate(self.shell, &mut cmd, bin_name, &mut io::stdout());

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_run_generates_output() {
        let completion = Completion { shell: Shell::Zsh };
        let mut cmd = crate::cli::Cli::command();
        let bin_name = cmd.get_name().to_string();
        let mut buf = Vec::new();
        generate(completion.shell, &mut cmd, bin_name, &mut buf);
        assert!(!buf.is_empty(), "generate() should produce output");
    }
}
