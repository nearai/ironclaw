use std::ffi::OsString;

use clap::{CommandFactory, Parser};

use crate::commands::Command;

#[derive(Debug, Parser)]
#[command(name = "ironclaw", about = "IronClaw agent runtime", version)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

pub(crate) fn command() -> clap::Command {
    Cli::command()
}

pub(crate) fn run() -> anyhow::Result<()> {
    let cli = Cli::parse_from(args_with_default_serve(std::env::args_os()));
    cli.command
        .unwrap_or(Command::Serve(
            crate::commands::serve::ServeCommand::default(),
        ))
        .execute()
}

fn args_with_default_serve(mut args: impl Iterator<Item = OsString>) -> Vec<OsString> {
    let program = args.next().unwrap_or_else(|| OsString::from("ironclaw"));
    let mut rest: Vec<_> = args.collect();

    let has_explicit_command = rest
        .first()
        .and_then(|arg| arg.to_str())
        .is_some_and(|arg| Cli::command().find_subcommand(arg).is_some());
    let is_help_or_version = rest
        .iter()
        .any(|arg| matches!(arg.to_str(), Some("--help" | "-h" | "--version" | "-V")));

    if !has_explicit_command && !is_help_or_version {
        rest.insert(0, OsString::from("serve"));
    }

    std::iter::once(program).chain(rest).collect()
}

#[cfg(test)]
mod tests {
    use super::{Cli, args_with_default_serve};
    use crate::commands::Command;
    use clap::Parser;
    use std::ffi::OsString;

    #[test]
    fn missing_subcommand_inserts_serve() {
        let args = [OsString::from("ironclaw")];
        let cli = Cli::try_parse_from(args_with_default_serve(args.into_iter()))
            .expect("default serve command should parse");

        assert!(matches!(cli.command, Some(Command::Serve(_))));
    }

    #[test]
    fn serve_options_before_another_subcommand_are_rejected() {
        let args = [
            OsString::from("ironclaw"),
            OsString::from("--host"),
            OsString::from("127.0.0.1"),
            OsString::from("status"),
        ];
        let error = Cli::try_parse_from(args_with_default_serve(args.into_iter()))
            .expect_err("serve options must not be silently ignored");

        assert!(error.to_string().contains("unexpected argument 'status'"));
    }
}
