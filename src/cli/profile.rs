//! CLI subcommand definitions for `ironclaw profile`.

use clap::Subcommand;

use crate::cli::fmt;
use crate::config::profile::{ProfileInfo, list_profiles};

#[derive(Subcommand, Debug, Clone)]
pub enum ProfileCommand {
    /// List all available deployment profiles.
    List,
}

/// Run the profile command.
pub fn run_profile_command(cmd: &ProfileCommand) -> anyhow::Result<()> {
    match cmd {
        ProfileCommand::List => run_list(),
    }
}

/// Brief description for each built-in profile.
fn builtin_description(name: &str) -> &'static str {
    match name {
        "local" => "Solo developer, TUI mode, no server features",
        "local-sandbox" => "Solo developer with Docker sandbox enabled",
        "server" => "Single-user server with PostgreSQL and sandbox",
        "server-multitenant" => "Multi-user SaaS with higher concurrency",
        _ => "Built-in profile",
    }
}

fn run_list() -> anyhow::Result<()> {
    let profiles = list_profiles();
    let active = std::env::var("IRONCLAW_PROFILE")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase());

    println!();
    println!("  {}Deployment Profiles{}", fmt::bold(), fmt::reset());
    println!();

    if profiles.is_empty() {
        println!("  No profiles found.");
        println!();
        return Ok(());
    }

    for profile in &profiles {
        print_profile(profile, active.as_deref());
    }

    println!();
    println!(
        "  {}Activate a profile with: {}IRONCLAW_PROFILE=<name> ironclaw{}",
        fmt::dim(),
        fmt::reset(),
        fmt::reset(),
    );
    println!();

    Ok(())
}

fn print_profile(profile: &ProfileInfo, active: Option<&str>) {
    let is_active = active.is_some_and(|a| a == profile.name);
    let marker = if is_active { " (active)" } else { "" };

    let description = if profile.builtin {
        builtin_description(&profile.name)
    } else {
        "Custom profile"
    };

    let source = if profile.builtin {
        if profile.path.is_some() {
            "built-in + user override"
        } else {
            "built-in"
        }
    } else {
        "user-defined"
    };

    println!(
        "  {}{}{}{}",
        fmt::accent(),
        profile.name,
        marker,
        fmt::reset(),
    );
    println!("    {}", description);
    println!("    {}[{}]{}", fmt::dim(), source, fmt::reset(),);
    if let Some(path) = &profile.path {
        println!("    {}{}{}", fmt::dim(), path.display(), fmt::reset(),);
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_profile_list_succeeds() {
        // Ensure the List subcommand runs without error.
        // Clear the env var so no profile is marked active.
        unsafe { std::env::remove_var("IRONCLAW_PROFILE") };
        let result = run_profile_command(&ProfileCommand::List);
        assert!(result.is_ok());
    }

    #[test]
    fn run_profile_list_with_active_profile() {
        // When IRONCLAW_PROFILE is set, the command should still succeed
        // and the matching profile gets an "(active)" marker.
        unsafe { std::env::set_var("IRONCLAW_PROFILE", "local") };
        let result = run_profile_command(&ProfileCommand::List);
        unsafe { std::env::remove_var("IRONCLAW_PROFILE") };
        assert!(result.is_ok());
    }

    #[test]
    fn builtin_description_known_profiles() {
        assert_eq!(
            builtin_description("local"),
            "Solo developer, TUI mode, no server features"
        );
        assert_eq!(
            builtin_description("server"),
            "Single-user server with PostgreSQL and sandbox"
        );
        assert_eq!(builtin_description("unknown-name"), "Built-in profile");
    }

    #[test]
    fn print_profile_builtin_no_active() {
        // Should not panic.
        let info = ProfileInfo {
            name: "local".to_string(),
            builtin: true,
            path: None,
        };
        print_profile(&info, None);
    }

    #[test]
    fn print_profile_user_defined_with_path() {
        let info = ProfileInfo {
            name: "my-custom".to_string(),
            builtin: false,
            path: Some(std::path::PathBuf::from("/tmp/my-custom.toml")),
        };
        print_profile(&info, Some("my-custom"));
    }

    #[test]
    fn print_profile_builtin_with_override() {
        let info = ProfileInfo {
            name: "server".to_string(),
            builtin: true,
            path: Some(std::path::PathBuf::from(
                "/home/user/.ironclaw/profiles/server.toml",
            )),
        };
        print_profile(&info, None);
    }
}
