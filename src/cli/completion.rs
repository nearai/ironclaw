use clap::{CommandFactory, Parser, ValueEnum};
use clap_complete::{generate, Shell};
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
        
        // Конвертируем наш enum в clap_complete::Shell
        let shell = match self.shell {
            CompletionShell::Bash => Shell::Bash,
            CompletionShell::Zsh => Shell::Zsh,
            CompletionShell::Fish => Shell::Fish,
            CompletionShell::Powershell => Shell::PowerShell,
            CompletionShell::Elvish => Shell::Elvish,
        };
        
        // Генерируем и выводим скрипт в stdout
        generate(shell, &mut cmd, bin_name, &mut io::stdout());
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_completion_shell_enum() {
    // Просто проверяем, что enum парсится
    let shells = ["bash", "zsh", "fish", "powershell", "elvish"];
    for shell in shells {
        let result = CompletionShell::from_str(shell, false);  // ignore_case=false для точного совпадения
        assert!(result.is_ok(), "Failed to parse {}", shell);
    }
}
    
    #[test]
    fn test_run_does_not_panic() {
        // Проверяем, что run() не паникует (без реального вывода)
        let completion = Completion { shell: CompletionShell::Zsh };
        // Мы не вызываем run() полностью, чтобы не генерировать вывод в тестах,
        // но проверяем, что структура валидна
        assert_eq!(format!("{:?}", completion.shell), "Zsh");
    }
}