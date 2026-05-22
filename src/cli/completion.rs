use clap::{CommandFactory, Parser};
use clap_complete::{Shell, generate};
use std::io::{self, Write};

/// Generate shell completion scripts for ironclaw
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

        if self.shell == Shell::Zsh {
            // Generate to buffer so we can patch the compdef call.
            // clap_complete emits bare `compdef _ironclaw ironclaw` which
            // errors if sourced before compinit. Guard it so the script
            // works in all sourcing contexts.
            let mut buf = Vec::new();
            generate(self.shell, &mut cmd, bin_name.clone(), &mut buf);
            let script = String::from_utf8(buf)?;

            let bare = format!("compdef _{0} {0}", bin_name);
            let guarded = format!("(( $+functions[compdef] )) && compdef _{0} {0}", bin_name);
            let patched = script.replace(&bare, &guarded);

            io::stdout().write_all(patched.as_bytes())?;
        } else {
            generate(self.shell, &mut cmd, bin_name, &mut io::stdout());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// clap_complete's `generate()` recurses through the Cli command tree and
    /// overflows the default 2 MiB test thread stack in debug builds. Run the
    /// body on a 16 MiB stack to keep these tests reliable across profiles.
    fn run_on_large_stack<F>(f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(f)
            .expect("spawn test thread")
            .join()
            .expect("test thread panicked");
    }

    #[test]
    fn test_run_generates_output() {
        run_on_large_stack(|| {
            let completion = Completion { shell: Shell::Zsh };
            let mut cmd = crate::cli::Cli::command();
            let bin_name = cmd.get_name().to_string();
            let mut buf = Vec::new();
            generate(completion.shell, &mut cmd, bin_name, &mut buf);
            assert!(!buf.is_empty(), "generate() should produce output");
        });
    }

    #[test]
    fn test_zsh_compdef_guard_applied() {
        run_on_large_stack(|| {
            let mut cmd = crate::cli::Cli::command();
            let bin_name = cmd.get_name().to_string();
            let mut buf = Vec::new();
            generate(Shell::Zsh, &mut cmd, bin_name.clone(), &mut buf);
            let raw = String::from_utf8(buf).expect("generated zsh script should be valid utf8");

            // Apply the same patching logic as run()
            let bare = format!("compdef _{0} {0}", bin_name);
            let guarded = format!("(( $+functions[compdef] )) && compdef _{0} {0}", bin_name);
            let patched = raw.replace(&bare, &guarded);

            let bare_compdef = format!("    compdef _{0} {0}\n", bin_name);
            assert!(
                !patched.contains(&bare_compdef),
                "bare compdef should not appear after patching"
            );
            assert!(
                patched.contains("$+functions[compdef]"),
                "patched output should contain compdef guard"
            );
        });
    }
}
