//! Deployment profile CLI commands.
//!
//! Lists available deployment profiles and shows which is active.

use clap::Subcommand;

use crate::config::profile::{ProfileInfo, list_profiles};

#[derive(Subcommand, Debug, Clone)]
pub enum ProfileCommand {
    /// List all available deployment profiles
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

/// Run the profile CLI subcommand.
pub fn run_profile_command(cmd: ProfileCommand) -> anyhow::Result<()> {
    match cmd {
        ProfileCommand::List { json } => cmd_list(json),
    }
}

/// List all available profiles, marking the active one.
fn cmd_list(json: bool) -> anyhow::Result<()> {
    let profiles = list_profiles();
    let active = std::env::var("IRONCLAW_PROFILE")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase());

    if json {
        let entries: Vec<serde_json::Value> = profiles
            .iter()
            .map(|p| profile_to_json(p, &active))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".to_string())
        );
        return Ok(());
    }

    if profiles.is_empty() {
        println!("No profiles found.");
        return Ok(());
    }

    println!("Available profiles ({} total):\n", profiles.len());

    for p in &profiles {
        let is_active = active.as_deref() == Some(&p.name);
        let marker = if is_active { " (active)" } else { "" };
        let source = profile_source(p);
        println!("  {:<24} {:<10}{}", p.name, source, marker);
    }

    println!();
    if active.is_none() {
        println!("No profile active. Set IRONCLAW_PROFILE=<name> to activate one.");
    }
    println!("User profiles directory: ~/.ironclaw/profiles/");

    Ok(())
}

fn profile_source(p: &ProfileInfo) -> &'static str {
    match (p.builtin, &p.path) {
        (true, Some(_)) => "override",
        (true, None) => "built-in",
        (false, _) => "user",
    }
}

fn profile_to_json(p: &ProfileInfo, active: &Option<String>) -> serde_json::Value {
    serde_json::json!({
        "name": p.name,
        "builtin": p.builtin,
        "source": profile_source(p),
        "path": p.path.as_ref().map(|p| p.display().to_string()),
        "active": active.as_deref() == Some(&p.name),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_includes_builtin_profiles() {
        let profiles = list_profiles();
        let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"local"), "Should include 'local' profile");
        assert!(names.contains(&"server"), "Should include 'server' profile");
    }

    #[test]
    fn test_profile_source_builtin() {
        let p = ProfileInfo {
            name: "server".to_string(),
            builtin: true,
            path: None,
        };
        assert_eq!(profile_source(&p), "built-in");
    }

    #[test]
    fn test_profile_source_user() {
        let p = ProfileInfo {
            name: "custom".to_string(),
            builtin: false,
            path: Some("/home/user/.ironclaw/profiles/custom.toml".into()),
        };
        assert_eq!(profile_source(&p), "user");
    }

    #[test]
    fn test_profile_source_override() {
        let p = ProfileInfo {
            name: "server".to_string(),
            builtin: true,
            path: Some("/home/user/.ironclaw/profiles/server.toml".into()),
        };
        assert_eq!(profile_source(&p), "override");
    }

    #[test]
    fn test_profile_to_json_inactive() {
        let p = ProfileInfo {
            name: "local".to_string(),
            builtin: true,
            path: None,
        };
        let json = profile_to_json(&p, &None);
        assert_eq!(json["name"], "local");
        assert_eq!(json["builtin"], true);
        assert_eq!(json["active"], false);
        assert_eq!(json["source"], "built-in");
    }

    #[test]
    fn test_profile_to_json_active() {
        let p = ProfileInfo {
            name: "server".to_string(),
            builtin: true,
            path: None,
        };
        let active = Some("server".to_string());
        let json = profile_to_json(&p, &active);
        assert_eq!(json["active"], true);
    }
}
