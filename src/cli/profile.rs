//! Profile management CLI commands.
//!
//! Lists available deployment profiles and shows which one is active.

use clap::Subcommand;

use crate::config::profile::{BUILTIN_PROFILES, ProfileInfo, list_profiles};

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
pub async fn run_profile_command(cmd: ProfileCommand) -> anyhow::Result<()> {
    match cmd {
        ProfileCommand::List { json } => cmd_list(json),
    }
}

/// Extract a one-line description from a profile's TOML comment header.
///
/// Built-in profiles have a header like:
/// ```text
/// # IronClaw Profile: local
/// #
/// # Stripped-down configuration for solo developers working locally.
/// ```
///
/// We skip the title line and blank comment lines, then take the first
/// non-empty comment line as the description.
fn extract_description(toml_content: &str) -> Option<String> {
    let mut lines = toml_content.lines();
    // Skip the `# IronClaw Profile: <name>` title line.
    lines.next()?;

    for line in lines {
        let trimmed = line.trim();
        if !trimmed.starts_with('#') {
            break;
        }
        let comment = trimmed.trim_start_matches('#').trim();
        if !comment.is_empty() {
            return Some(comment.to_string());
        }
    }
    None
}

/// Get the description for a built-in profile by name.
fn builtin_description(name: &str) -> Option<String> {
    BUILTIN_PROFILES
        .iter()
        .find(|(n, _)| *n == name)
        .and_then(|(_, toml)| extract_description(toml))
}

/// Read the description from a user-defined profile TOML file.
fn user_profile_description(path: &std::path::Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(content) => extract_description(&content),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "Failed to read user profile for description");
            None
        }
    }
}

/// Get the description for a profile, checking built-in first then file.
fn profile_description(info: &ProfileInfo) -> Option<String> {
    if info.builtin {
        builtin_description(&info.name)
    } else {
        info.path.as_deref().and_then(user_profile_description)
    }
}

/// List all available profiles.
fn cmd_list(json: bool) -> anyhow::Result<()> {
    let profiles = list_profiles();
    let active_name = std::env::var("IRONCLAW_PROFILE")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase());

    if json {
        #[derive(serde::Serialize)]
        struct ProfileEntry<'a> {
            name: &'a str,
            builtin: bool,
            active: bool,
            #[serde(skip_serializing_if = "Option::is_none")]
            path: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            description: Option<String>,
        }

        let entries: Vec<ProfileEntry> = profiles
            .iter()
            .map(|p| ProfileEntry {
                name: &p.name,
                builtin: p.builtin,
                active: active_name.as_deref() == Some(p.name.as_str()),
                path: p.path.as_ref().map(|path| path.display().to_string()),
                description: profile_description(p),
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".to_string())
        );
        return Ok(());
    }

    // Human-readable output.
    println!("Available deployment profiles:\n");

    for p in &profiles {
        let active = if active_name.as_deref() == Some(p.name.as_str()) {
            " (active)"
        } else {
            ""
        };
        let kind = if p.builtin { "built-in" } else { "custom" };
        let desc = profile_description(p).unwrap_or_default();

        println!("  {:<24} {:<10}{}", p.name, kind, active);
        if !desc.is_empty() {
            println!("    {desc}");
        }
        if let Some(path) = &p.path {
            println!("    path: {}", path.display());
        }
    }

    println!();
    if active_name.is_none() {
        println!("No profile is currently active.");
        println!("Set IRONCLAW_PROFILE=<name> to activate one.");
    }
    println!("\nCustom profiles can be added to ~/.ironclaw/profiles/");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_description_from_builtin() {
        let toml = "# IronClaw Profile: local\n#\n# A short description.\n#\ndatabase_backend = \"libsql\"\n";
        assert_eq!(
            extract_description(toml),
            Some("A short description.".to_string())
        );
    }

    #[test]
    fn extract_description_skips_blank_comments() {
        let toml = "# IronClaw Profile: foo\n#\n#\n# Real description.\n";
        assert_eq!(
            extract_description(toml),
            Some("Real description.".to_string())
        );
    }

    #[test]
    fn extract_description_none_when_no_comments() {
        let toml = "database_backend = \"libsql\"\n";
        assert_eq!(extract_description(toml), None);
    }

    #[test]
    fn builtin_descriptions_exist() {
        for &(name, _) in BUILTIN_PROFILES {
            assert!(
                builtin_description(name).is_some(),
                "built-in profile '{name}' should have a description"
            );
        }
    }

    #[test]
    fn user_profile_description_from_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("custom.toml");
        std::fs::write(
            &path,
            "# IronClaw Profile: custom\n#\n# My custom profile.\n\ndatabase_backend = \"postgres\"\n",
        )
        .unwrap();
        assert_eq!(
            user_profile_description(&path),
            Some("My custom profile.".to_string())
        );
    }

    #[test]
    fn user_profile_description_missing_file() {
        assert_eq!(
            user_profile_description(std::path::Path::new("/nonexistent/profile.toml")),
            None
        );
    }
}
