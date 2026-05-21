//! Skills management CLI commands.
//!
//! Commands for listing, searching, inspecting, and installing SKILL.md-based skills.
//! List and info operate on the filesystem only; search queries the ClawHub registry;
//! install pulls from the IronHub release manifest or copies from a local path.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use tokio::fs;

use crate::bootstrap::ironclaw_base_dir;
use crate::cli::hub_install::looks_like_hub_name;
use crate::config::SkillsConfig;
use ironclaw_skills::catalog::SkillCatalog;
use ironclaw_skills::{SkillRegistry, SkillSource, parse_skill_md};

fn default_skills_dir() -> PathBuf {
    ironclaw_base_dir().join("skills")
}

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

    /// Install a skill from a local SKILL.md or directory. For IronHub installs use `ironclaw hub install <name>`.
    Install {
        path: PathBuf,

        #[arg(short, long)]
        target: Option<PathBuf>,

        #[arg(short, long)]
        force: bool,
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
        SkillsCommand::Install {
            path,
            target,
            force,
        } => install_skill_local(&path, target, force).await,
    }
}

async fn install_skill_local(
    source_path: &Path,
    target: Option<PathBuf>,
    force: bool,
) -> anyhow::Result<()> {
    let skills_dir = target.unwrap_or_else(default_skills_dir);

    let metadata = fs::metadata(source_path)
        .await
        .map_err(|e| anyhow::anyhow!("Cannot read {}: {}", source_path.display(), e))?;

    let (skill_md_src, skill_name) = if metadata.is_dir() {
        let candidate = source_path.join("SKILL.md");
        if !candidate.exists() {
            anyhow::bail!("No SKILL.md found in directory {}.", source_path.display());
        }
        let name = source_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| {
                anyhow::anyhow!("Cannot derive skill name from {}", source_path.display())
            })?
            .to_string();
        (candidate, name)
    } else if source_path.file_name().and_then(|n| n.to_str()) == Some("SKILL.md") {
        let parent = source_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("SKILL.md has no parent directory"))?;
        let name = parent
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow::anyhow!("Cannot derive skill name from {}", parent.display()))?
            .to_string();
        (source_path.to_path_buf(), name)
    } else {
        anyhow::bail!(
            "Expected a SKILL.md file or a directory containing one, got: {}",
            source_path.display()
        );
    };

    if !looks_like_hub_name(&skill_name) {
        anyhow::bail!(
            "Skill directory name '{}' is not a valid identifier (lowercase letters, digits, hyphens; must start with a letter or digit).",
            skill_name
        );
    }

    let content = fs::read_to_string(&skill_md_src)
        .await
        .map_err(|e| anyhow::anyhow!("Cannot read {}: {}", skill_md_src.display(), e))?;
    let parsed = parse_skill_md(&content)
        .map_err(|e| anyhow::anyhow!("Invalid SKILL.md at {}: {}", skill_md_src.display(), e))?;
    if parsed.manifest.name != skill_name {
        anyhow::bail!(
            "SKILL.md manifest name '{}' does not match directory '{}'.",
            parsed.manifest.name,
            skill_name
        );
    }

    let target_dir = skills_dir.join(&skill_name);
    let target_md = target_dir.join("SKILL.md");

    if target_md.exists() && !force {
        anyhow::bail!(
            "Skill '{}' already exists at {}. Use --force to overwrite.",
            skill_name,
            target_md.display()
        );
    }

    fs::create_dir_all(&target_dir).await?;
    fs::write(&target_md, content.as_bytes()).await?;

    let on_disk = fs::metadata(&target_md).await?;
    if on_disk.len() as usize != content.len() {
        anyhow::bail!(
            "On-disk size mismatch after write at {} (expected {} bytes, got {}).",
            target_md.display(),
            content.len(),
            on_disk.len()
        );
    }

    println!("\nInstalled successfully:");
    println!("  Name:    {}", skill_name);
    println!("  Version: {}", parsed.manifest.version);
    println!("  Path:    {}", target_md.display());
    println!("  Size:    {} bytes", content.len());

    Ok(())
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

    let reqs = &skill.manifest.requires;
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

    fn skill_md_fixture(name: &str, version: &str) -> String {
        format!(
            "---\nname: {}\nversion: {}\ndescription: test fixture\n---\n\nBody content for {}.\n",
            name, version, name
        )
    }

    #[tokio::test]
    async fn install_skill_local_copies_skill_md_from_directory() {
        let src = tempfile::tempdir().expect("src tempdir");
        let skill_dir = src.path().join("test-skill");
        fs::create_dir(&skill_dir).await.expect("mkdir");
        let body = skill_md_fixture("test-skill", "0.1.0");
        fs::write(skill_dir.join("SKILL.md"), &body)
            .await
            .expect("write");

        let dest = tempfile::tempdir().expect("dest tempdir");
        install_skill_local(&skill_dir, Some(dest.path().to_path_buf()), false)
            .await
            .expect("install ok");

        let installed = dest.path().join("test-skill/SKILL.md");
        assert!(installed.exists());
        let on_disk = fs::read_to_string(&installed).await.expect("read");
        assert_eq!(on_disk, body);
    }

    #[tokio::test]
    async fn install_skill_local_accepts_direct_skill_md_path() {
        let src = tempfile::tempdir().expect("src tempdir");
        let skill_dir = src.path().join("test-skill");
        fs::create_dir(&skill_dir).await.expect("mkdir");
        let md_path = skill_dir.join("SKILL.md");
        let body = skill_md_fixture("test-skill", "0.1.0");
        fs::write(&md_path, &body).await.expect("write");

        let dest = tempfile::tempdir().expect("dest tempdir");
        install_skill_local(&md_path, Some(dest.path().to_path_buf()), false)
            .await
            .expect("install ok");

        assert!(dest.path().join("test-skill/SKILL.md").exists());
    }

    #[tokio::test]
    async fn install_skill_local_refuses_overwrite_without_force() {
        let src = tempfile::tempdir().expect("src tempdir");
        let skill_dir = src.path().join("test-skill");
        fs::create_dir(&skill_dir).await.expect("mkdir");
        fs::write(
            skill_dir.join("SKILL.md"),
            skill_md_fixture("test-skill", "0.1.0"),
        )
        .await
        .expect("write");

        let dest = tempfile::tempdir().expect("dest tempdir");
        install_skill_local(&skill_dir, Some(dest.path().to_path_buf()), false)
            .await
            .expect("first install");

        let err = install_skill_local(&skill_dir, Some(dest.path().to_path_buf()), false)
            .await
            .expect_err("second install must fail without force");
        assert!(err.to_string().contains("already exists"));
    }

    #[tokio::test]
    async fn install_skill_local_overwrites_with_force() {
        let src = tempfile::tempdir().expect("src tempdir");
        let skill_dir = src.path().join("test-skill");
        fs::create_dir(&skill_dir).await.expect("mkdir");
        fs::write(
            skill_dir.join("SKILL.md"),
            skill_md_fixture("test-skill", "0.1.0"),
        )
        .await
        .expect("write");

        let dest = tempfile::tempdir().expect("dest tempdir");
        install_skill_local(&skill_dir, Some(dest.path().to_path_buf()), false)
            .await
            .expect("first install");

        let updated = skill_md_fixture("test-skill", "0.2.0");
        fs::write(skill_dir.join("SKILL.md"), &updated)
            .await
            .expect("update src");
        install_skill_local(&skill_dir, Some(dest.path().to_path_buf()), true)
            .await
            .expect("forced reinstall");

        let on_disk = fs::read_to_string(dest.path().join("test-skill/SKILL.md"))
            .await
            .expect("read");
        assert_eq!(on_disk, updated);
    }

    #[tokio::test]
    async fn install_skill_local_rejects_directory_without_skill_md() {
        let src = tempfile::tempdir().expect("src tempdir");
        let bare = src.path().join("empty-skill");
        fs::create_dir(&bare).await.expect("mkdir");

        let dest = tempfile::tempdir().expect("dest tempdir");
        let err = install_skill_local(&bare, Some(dest.path().to_path_buf()), false)
            .await
            .expect_err("missing SKILL.md must fail");
        assert!(err.to_string().contains("No SKILL.md"));
    }

    #[tokio::test]
    async fn install_skill_local_rejects_arbitrary_file() {
        let src = tempfile::tempdir().expect("src tempdir");
        let stray = src.path().join("README.md");
        fs::write(&stray, b"# not a skill").await.expect("write");

        let dest = tempfile::tempdir().expect("dest tempdir");
        let err = install_skill_local(&stray, Some(dest.path().to_path_buf()), false)
            .await
            .expect_err("arbitrary file must fail");
        assert!(err.to_string().contains("SKILL.md"));
    }

    #[tokio::test]
    async fn install_skill_local_rejects_bad_dir_name() {
        let src = tempfile::tempdir().expect("src tempdir");
        let bad_dir = src.path().join("Bad Name!");
        fs::create_dir(&bad_dir).await.expect("mkdir");
        fs::write(
            bad_dir.join("SKILL.md"),
            skill_md_fixture("bad-name", "0.1.0"),
        )
        .await
        .expect("write");

        let dest = tempfile::tempdir().expect("dest tempdir");
        let err = install_skill_local(&bad_dir, Some(dest.path().to_path_buf()), false)
            .await
            .expect_err("bad dir name must fail");
        assert!(err.to_string().contains("not a valid identifier"));
    }

    #[tokio::test]
    async fn install_skill_local_rejects_malformed_skill_md() {
        let src = tempfile::tempdir().expect("src tempdir");
        let skill_dir = src.path().join("test-skill");
        fs::create_dir(&skill_dir).await.expect("mkdir");
        fs::write(skill_dir.join("SKILL.md"), b"no frontmatter here at all")
            .await
            .expect("write");

        let dest = tempfile::tempdir().expect("dest tempdir");
        let err = install_skill_local(&skill_dir, Some(dest.path().to_path_buf()), false)
            .await
            .expect_err("malformed SKILL.md must fail");
        assert!(err.to_string().contains("Invalid SKILL.md"));
    }

    #[tokio::test]
    async fn install_skill_local_rejects_manifest_name_mismatch() {
        let src = tempfile::tempdir().expect("src tempdir");
        let skill_dir = src.path().join("test-skill");
        fs::create_dir(&skill_dir).await.expect("mkdir");
        fs::write(
            skill_dir.join("SKILL.md"),
            skill_md_fixture("different-name", "0.1.0"),
        )
        .await
        .expect("write");

        let dest = tempfile::tempdir().expect("dest tempdir");
        let err = install_skill_local(&skill_dir, Some(dest.path().to_path_buf()), false)
            .await
            .expect_err("name mismatch must fail");
        assert!(err.to_string().contains("does not match"));
    }

    #[test]
    fn default_skills_dir_under_ironclaw_base() {
        let dir = default_skills_dir();
        assert!(dir.to_string_lossy().contains(".ironclaw"));
        assert!(dir.to_string_lossy().contains("skills"));
    }
}
