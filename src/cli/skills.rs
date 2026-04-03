//! Skills management CLI commands.
//!
//! Commands for listing, searching, and inspecting SKILL.md-based skills.
//! List and info operate on the filesystem only; search queries the ClawHub registry.

use std::path::Path;

use clap::Subcommand;

use crate::config::SkillsConfig;
use ironclaw_skills::catalog::SkillCatalog;
use ironclaw_skills::{SkillRegistry, SkillSource};

#[derive(Subcommand, Debug, Clone)]
pub enum SkillsCommand {
    /// List all discovered skills
    List {
        /// Show detailed information (keywords, patterns, source path)
        #[arg(short, long)]
        verbose: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Search ClawHub registry for skills
    Search {
        /// Search query
        query: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show detailed info about a specific skill
    Info {
        /// Skill name
        name: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

/// Run the skills CLI subcommand.
pub async fn run_skills_command(
    cmd: SkillsCommand,
    config_path: Option<&Path>,
) -> anyhow::Result<()> {
    let full_config = crate::config::Config::from_env_with_toml(config_path)
        .await
        .map_err(|e| anyhow::anyhow!("{e:#}"))?;
    let config = full_config.skills;

    if !config.enabled {
        anyhow::bail!("Skills system is disabled (SKILLS_ENABLED=false)");
    }

    match cmd {
        SkillsCommand::List { verbose, json } => cmd_list(&config, verbose, json).await,
        SkillsCommand::Search { query, json } => cmd_search(&query, json).await,
        SkillsCommand::Info { name, json } => cmd_info(&config, &name, json).await,
    }
}

/// Discover skills from all configured directories.
async fn discover_skills(config: &SkillsConfig) -> SkillRegistry {
    let mut registry = SkillRegistry::new(config.local_dir.clone())
        .with_installed_dir(config.installed_dir.clone())
        .with_max_scan_depth(config.max_scan_depth);
    registry.discover_all().await;
    registry
}

/// Format a skill source path for display.
fn format_source(source: &SkillSource) -> &str {
    match source {
        SkillSource::Workspace(_) => "workspace",
        SkillSource::User(_) => "user",
        SkillSource::Installed(_) => "installed",
        SkillSource::Bundled(_) => "bundled",
    }
}

/// List all discovered skills.
async fn cmd_list(config: &SkillsConfig, verbose: bool, json: bool) -> anyhow::Result<()> {
    let registry = discover_skills(config).await;
    let skills = registry.skills();

    if json {
        let entries: Vec<serde_json::Value> = skills
            .iter()
            .map(|s| {
                let mut v = serde_json::json!({
                    "name": s.manifest.name,
                    "version": s.manifest.version,
                    "description": s.manifest.description,
                    "trust": s.trust.to_string(),
                    "source": format_source(&s.source),
                });
                if verbose {
                    v["keywords"] = serde_json::json!(s.manifest.activation.keywords);
                    v["tags"] = serde_json::json!(s.manifest.activation.tags);
                    v["patterns"] = serde_json::json!(s.manifest.activation.patterns);
                }
                v
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".to_string())
        );
        return Ok(());
    }

    if skills.is_empty() {
        println!("No skills found.");
        println!();
        println!("Skills directories:");
        println!("  User:      {}", config.local_dir.display());
        println!("  Installed: {}", config.installed_dir.display());
        println!();
        println!("Use 'ironclaw skills search <query>' to find skills on ClawHub.");
        return Ok(());
    }

    println!("Discovered {} skill(s):\n", skills.len());

    for s in skills {
        if verbose {
            println!("  {} v{}", s.manifest.name, s.manifest.version);
            println!("    Trust:       {}", s.trust);
            println!("    Source:      {}", format_source(&s.source));
            if !s.manifest.description.is_empty() {
                println!("    Description: {}", s.manifest.description);
            }
            if !s.manifest.activation.keywords.is_empty() {
                println!(
                    "    Keywords:    {}",
                    s.manifest.activation.keywords.join(", ")
                );
            }
            if !s.manifest.activation.tags.is_empty() {
                println!("    Tags:        {}", s.manifest.activation.tags.join(", "));
            }
            println!();
        } else {
            let desc = truncate(&s.manifest.description, 50);
            println!(
                "  {:<24} v{:<10} [{}]  {}",
                s.manifest.name, s.manifest.version, s.trust, desc,
            );
        }
    }

    if !verbose {
        println!();
        println!(
            "Use --verbose for details, or 'ironclaw skills info <name>' for a specific skill."
        );
    }

    Ok(())
}

/// Search ClawHub registry.
async fn cmd_search(query: &str, json: bool) -> anyhow::Result<()> {
    let catalog = SkillCatalog::new();
    let outcome = catalog.search(query).await;

    let mut entries = outcome.results;
    catalog.enrich_search_results(&mut entries, 5).await;

    if json {
        let json_entries: Vec<serde_json::Value> = entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "slug": e.slug,
                    "name": e.name,
                    "description": e.description,
                    "version": e.version,
                    "stars": e.stars,
                    "downloads": e.downloads,
                    "owner": e.owner,
                })
            })
            .collect();
        let result = serde_json::json!({
            "query": query,
            "results": json_entries,
            "error": outcome.error,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string())
        );
        return Ok(());
    }

    println!("ClawHub results for \"{}\":\n", query);

    if entries.is_empty() {
        if let Some(ref err) = outcome.error {
            println!("  (registry error: {})", err);
        } else {
            println!("  No results found.");
        }
        return Ok(());
    }

    for entry in &entries {
        let owner_str = entry
            .owner
            .as_deref()
            .map(|o| format!("  by {o}"))
            .unwrap_or_default();

        let stats: Vec<String> = [
            entry.stars.map(|s| format!("{s} stars")),
            entry.downloads.map(|d| format!("{d} downloads")),
        ]
        .into_iter()
        .flatten()
        .collect();
        let stats_str = if stats.is_empty() {
            String::new()
        } else {
            format!("  ({})", stats.join(", "))
        };

        println!(
            "  {} v{}{}{}",
            entry.slug, entry.version, owner_str, stats_str
        );
        if !entry.description.is_empty() {
            println!("    {}", truncate(&entry.description, 70));
        }
    }

    if let Some(ref err) = outcome.error {
        println!("\n  (note: {})", err);
    }

    Ok(())
}

/// Show detailed info about a specific skill.
async fn cmd_info(config: &SkillsConfig, name: &str, json: bool) -> anyhow::Result<()> {
    let registry = discover_skills(config).await;
    let skill = registry.find_by_name(name).ok_or_else(|| {
        anyhow::anyhow!(
            "Skill '{}' not found. Use 'ironclaw skills list' to see available skills.",
            name
        )
    })?;

    if json {
        let v = serde_json::json!({
            "name": skill.manifest.name,
            "version": skill.manifest.version,
            "description": skill.manifest.description,
            "trust": skill.trust.to_string(),
            "source": format_source(&skill.source),
            "content_hash": skill.content_hash,
            "activation": {
                "keywords": skill.manifest.activation.keywords,
                "patterns": skill.manifest.activation.patterns,
                "tags": skill.manifest.activation.tags,
                "exclude_keywords": skill.manifest.activation.exclude_keywords,
                "max_context_tokens": skill.manifest.activation.max_context_tokens,
            },
            "prompt_length": skill.prompt_content.len(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".to_string())
        );
        return Ok(());
    }

    println!("Skill: {}", skill.manifest.name);
    println!("  Version:     {}", skill.manifest.version);
    println!("  Trust:       {}", skill.trust);
    println!("  Source:      {}", format_source(&skill.source));
    if !skill.manifest.description.is_empty() {
        println!("  Description: {}", skill.manifest.description);
    }
    println!("  Hash:        {}", skill.content_hash);
    println!(
        "  Prompt size: {} bytes (~{} tokens)",
        skill.prompt_content.len(),
        skill.prompt_content.split_whitespace().count() * 13 / 10
    );

    let act = &skill.manifest.activation;
    if !act.keywords.is_empty() {
        println!("  Keywords:    {}", act.keywords.join(", "));
    }
    if !act.exclude_keywords.is_empty() {
        println!("  Exclude:     {}", act.exclude_keywords.join(", "));
    }
    if !act.patterns.is_empty() {
        println!("  Patterns:    {}", act.patterns.join(", "));
    }
    if !act.tags.is_empty() {
        println!("  Tags:        {}", act.tags.join(", "));
    }
    println!("  Max tokens:  {}", act.max_context_tokens);

    if let Some(reqs) = skill
        .manifest
        .metadata
        .as_ref()
        .and_then(|m| m.openclaw.as_ref())
        .map(|o| &o.requires)
    {
        if !reqs.bins.is_empty() {
            println!("  Requires bins:    {}", reqs.bins.join(", "));
        }
        if !reqs.env.is_empty() {
            println!("  Requires env:     {}", reqs.env.join(", "));
        }
        if !reqs.config.is_empty() {
            println!("  Requires config:  {}", reqs.config.join(", "));
        }
        if !reqs.skills.is_empty() {
            println!("  Requires skills:  {}", reqs.skills.join(", "));
        }
    }

    Ok(())
}

/// Truncate a string to max chars, appending "..." if truncated.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_string() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string() {
        assert_eq!(truncate("hello world foo bar", 10), "hello w...");
    }

    #[test]
    fn truncate_multibyte_safe() {
        // Should not panic on multibyte characters
        let s = "日本語テスト";
        let result = truncate(s, 4);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn format_source_variants() {
        use std::path::PathBuf;
        assert_eq!(
            format_source(&SkillSource::Workspace(PathBuf::new())),
            "workspace"
        );
        assert_eq!(format_source(&SkillSource::User(PathBuf::new())), "user");
        assert_eq!(
            format_source(&SkillSource::Bundled(PathBuf::new())),
            "bundled"
        );
    }
}
