use std::path::PathBuf;

use clap::Subcommand;

use crate::cli::hub_install::{hub_manifest_url_for_tag, validate_hub_name};
use crate::registry::{HubInstaller, HubManifest, HubSkillEntry, HubToolEntry};

#[derive(Subcommand, Debug, Clone)]
pub enum HubCommand {
    /// Install a tool or skill from IronHub by name
    Install {
        name: String,

        #[arg(long, conflicts_with = "tool")]
        skill: bool,

        #[arg(long, conflicts_with = "skill")]
        tool: bool,

        #[arg(long)]
        release_tag: Option<String>,

        #[arg(long)]
        target: Option<PathBuf>,

        #[arg(short, long)]
        force: bool,

        /// Acknowledge installing UNVERIFIED community content (required when the entry is not NEAR-vetted).
        #[arg(long)]
        acknowledge_unverified: bool,
    },

    /// Search the IronHub catalog by name or description
    Search {
        query: String,

        #[arg(long)]
        release_tag: Option<String>,
    },

    /// List everything available in IronHub
    List {
        #[arg(long, conflicts_with = "skills")]
        tools: bool,

        #[arg(long, conflicts_with = "tools")]
        skills: bool,

        #[arg(long)]
        release_tag: Option<String>,
    },

    /// Show detailed metadata for an entry
    Info {
        name: String,

        #[arg(long)]
        release_tag: Option<String>,
    },
}

pub async fn run_hub_command(cmd: HubCommand) -> anyhow::Result<()> {
    match cmd {
        HubCommand::Install {
            name,
            skill,
            tool,
            release_tag,
            target,
            force,
            acknowledge_unverified,
        } => {
            install(
                &name,
                skill,
                tool,
                release_tag,
                target,
                force,
                acknowledge_unverified,
            )
            .await
        }
        HubCommand::Search { query, release_tag } => search(&query, release_tag).await,
        HubCommand::List {
            tools,
            skills,
            release_tag,
        } => list(tools, skills, release_tag).await,
        HubCommand::Info { name, release_tag } => info(&name, release_tag).await,
    }
}

fn build_installer(
    release_tag: Option<String>,
    tools_dir_override: Option<PathBuf>,
    skills_dir_override: Option<PathBuf>,
) -> anyhow::Result<HubInstaller> {
    let mut installer = HubInstaller::with_defaults();
    if let Some(dir) = tools_dir_override {
        installer = HubInstaller::new(
            installer.manifest_url().to_string(),
            dir,
            installer.skills_dir().to_path_buf(),
        );
    }
    if let Some(dir) = skills_dir_override {
        installer = HubInstaller::new(
            installer.manifest_url().to_string(),
            installer.tools_dir().to_path_buf(),
            dir,
        );
    }
    if let Some(tag) = release_tag.as_deref() {
        installer = installer.with_manifest_url(hub_manifest_url_for_tag(tag)?);
    }
    Ok(installer)
}

#[derive(Debug)]
enum Kind {
    Tool,
    Skill,
}

fn classify(
    manifest: &HubManifest,
    name: &str,
    force_tool: bool,
    force_skill: bool,
) -> anyhow::Result<Kind> {
    let in_tools = manifest.find_tool(name).is_some();
    let in_skills = manifest.find_skill(name).is_some();

    if force_tool {
        if !in_tools {
            anyhow::bail!("'{}' is not a tool in this IronHub release", name);
        }
        return Ok(Kind::Tool);
    }
    if force_skill {
        if !in_skills {
            anyhow::bail!("'{}' is not a skill in this IronHub release", name);
        }
        return Ok(Kind::Skill);
    }

    match (in_tools, in_skills) {
        (true, false) => Ok(Kind::Tool),
        (false, true) => Ok(Kind::Skill),
        (true, true) => anyhow::bail!(
            "'{}' exists as both a tool and a skill in this release; pass --tool or --skill to disambiguate",
            name
        ),
        (false, false) => {
            let suggestions = nearest_matches(manifest, name);
            if suggestions.is_empty() {
                anyhow::bail!("'{}' is not in this IronHub release", name);
            }
            anyhow::bail!(
                "'{}' is not in this IronHub release. Did you mean: {}?",
                name,
                suggestions.join(", ")
            )
        }
    }
}

fn nearest_matches(manifest: &HubManifest, query: &str) -> Vec<String> {
    let q = query.to_ascii_lowercase();
    let mut out: Vec<String> = manifest
        .tools
        .iter()
        .map(|t| t.name.clone())
        .chain(manifest.skills.iter().map(|s| s.name.clone()))
        .filter(|n| {
            let nl = n.to_ascii_lowercase();
            nl.contains(&q) || q.contains(&nl)
        })
        .collect();
    out.sort();
    out.truncate(5);
    out
}

async fn install(
    name: &str,
    force_skill: bool,
    force_tool: bool,
    release_tag: Option<String>,
    target: Option<PathBuf>,
    force: bool,
    acknowledge_unverified: bool,
) -> anyhow::Result<()> {
    validate_hub_name(name)?;

    let probe = build_installer(release_tag.clone(), None, None)?;
    println!("Fetching IronHub manifest from {}", probe.manifest_url());
    let manifest = probe
        .fetch_manifest()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    install_with_manifest(
        manifest,
        name,
        force_skill,
        force_tool,
        release_tag,
        target,
        force,
        acknowledge_unverified,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn install_with_manifest(
    manifest: HubManifest,
    name: &str,
    force_skill: bool,
    force_tool: bool,
    release_tag: Option<String>,
    target: Option<PathBuf>,
    force: bool,
    acknowledge_unverified: bool,
) -> anyhow::Result<()> {
    let kind = classify(&manifest, name, force_tool, force_skill)?;

    let provenance = match kind {
        Kind::Tool => manifest.find_tool(name).map(|t| t.provenance),
        Kind::Skill => manifest.find_skill(name).map(|s| s.provenance),
    }
    .unwrap_or_default();

    if provenance.is_community_unverified() && !acknowledge_unverified {
        anyhow::bail!(
            "'{name}' is UNVERIFIED community content (trust tier: {}). \
             Not NEAR-vetted. Re-run with --acknowledge-unverified to install at your own risk.",
            provenance.as_wire()
        );
    }

    let (tools_override, skills_override) = match (&kind, &target) {
        (Kind::Tool, Some(dir)) => (Some(dir.clone()), None),
        (Kind::Skill, Some(dir)) => (None, Some(dir.clone())),
        _ => (None, None),
    };
    let installer = build_installer(release_tag, tools_override, skills_override)?;

    match kind {
        Kind::Tool => {
            println!(
                "Installing tool '{}' ({}) from IronHub...",
                name,
                provenance.as_wire()
            );
            let outcome = installer
                .install_tool_from_manifest(&manifest, name, force)
                .await
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            println!("\nInstalled successfully:");
            println!("  Kind:       tool");
            println!("  Name:       {}", outcome.name);
            println!("  Version:    {}", outcome.version);
            println!("  Release:    {}", outcome.release_tag);
            println!("  Provenance: {}", provenance.as_wire());
            println!("  WASM:       {}", outcome.primary_path.display());
            if let Some(caps) = outcome.metadata_path {
                println!("  Caps:       {}", caps.display());
            }
        }
        Kind::Skill => {
            println!(
                "Installing skill '{}' ({}) from IronHub...",
                name,
                provenance.as_wire()
            );
            let outcome = installer
                .install_skill_from_manifest(&manifest, name, force)
                .await
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            println!("\nInstalled successfully:");
            println!("  Kind:       skill");
            println!("  Name:       {}", outcome.name);
            println!("  Version:    {}", outcome.version);
            println!("  Release:    {}", outcome.release_tag);
            println!("  Provenance: {}", provenance.as_wire());
            println!("  SKILL.md:   {}", outcome.primary_path.display());
        }
    }
    Ok(())
}

async fn search(query: &str, release_tag: Option<String>) -> anyhow::Result<()> {
    let installer = build_installer(release_tag, None, None)?;
    let manifest = installer
        .fetch_manifest()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let q = query.to_ascii_lowercase();
    let tool_hits: Vec<&HubToolEntry> = manifest
        .tools
        .iter()
        .filter(|t| entry_matches(&t.name, &t.description, &q))
        .collect();
    let skill_hits: Vec<&HubSkillEntry> = manifest
        .skills
        .iter()
        .filter(|s| entry_matches(&s.name, &s.description, &q))
        .collect();

    if tool_hits.is_empty() && skill_hits.is_empty() {
        println!(
            "No matches for '{}' in release {}",
            query, manifest.release_tag
        );
        return Ok(());
    }

    println!("Release: {}", manifest.release_tag);
    if !tool_hits.is_empty() {
        println!("\nTools:");
        for t in &tool_hits {
            print_tool_row(t);
        }
    }
    if !skill_hits.is_empty() {
        println!("\nSkills:");
        for s in &skill_hits {
            print_skill_row(s);
        }
    }
    Ok(())
}

async fn list(
    tools_only: bool,
    skills_only: bool,
    release_tag: Option<String>,
) -> anyhow::Result<()> {
    let installer = build_installer(release_tag, None, None)?;
    let manifest = installer
        .fetch_manifest()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    println!("Release: {}", manifest.release_tag);
    println!("Repo:    {}", manifest.repo);

    let show_tools = !skills_only;
    let show_skills = !tools_only;

    if show_tools {
        if manifest.tools.is_empty() {
            println!("\nTools: (none in this release)");
        } else {
            println!("\nTools ({}):", manifest.tools.len());
            for t in &manifest.tools {
                print_tool_row(t);
            }
        }
    }

    if show_skills {
        if manifest.skills.is_empty() {
            println!("\nSkills: (none in this release)");
        } else {
            println!("\nSkills ({}):", manifest.skills.len());
            for s in &manifest.skills {
                print_skill_row(s);
            }
        }
    }
    Ok(())
}

async fn info(name: &str, release_tag: Option<String>) -> anyhow::Result<()> {
    validate_hub_name(name)?;
    let installer = build_installer(release_tag, None, None)?;
    let manifest = installer
        .fetch_manifest()
        .await
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    if let Some(t) = manifest.find_tool(name) {
        println!("Kind:        tool");
        println!("Name:        {}", t.name);
        println!("Crate:       {}", t.crate_name);
        println!("Version:     {}", t.version);
        println!("Description: {}", display_or_dash(&t.description));
        println!("Release:     {}", manifest.release_tag);
        println!("\nWASM artifact:");
        println!("  URL:    {}", t.wasm.url);
        println!("  Size:   {} bytes", t.wasm.size_bytes);
        println!("  SHA256: {}", t.wasm.sha256);
        println!("\nCapabilities artifact:");
        println!("  URL:    {}", t.capabilities.url);
        println!("  Size:   {} bytes", t.capabilities.size_bytes);
        println!("  SHA256: {}", t.capabilities.sha256);
        return Ok(());
    }
    if let Some(s) = manifest.find_skill(name) {
        println!("Kind:        skill");
        println!("Name:        {}", s.name);
        if !s.trunk.is_empty() {
            println!("Trunk:       {}", s.trunk);
        }
        println!("Version:     {}", display_or_dash(&s.version));
        println!("Description: {}", display_or_dash(&s.description));
        println!("Release:     {}", manifest.release_tag);
        println!("\nSKILL.md artifact:");
        println!("  URL:    {}", s.skill_md.url);
        println!("  Size:   {} bytes", s.skill_md.size_bytes);
        println!("  SHA256: {}", s.skill_md.sha256);
        return Ok(());
    }

    let suggestions = nearest_matches(&manifest, name);
    if suggestions.is_empty() {
        anyhow::bail!("'{}' is not in this IronHub release", name);
    }
    anyhow::bail!(
        "'{}' is not in this IronHub release. Did you mean: {}?",
        name,
        suggestions.join(", ")
    );
}

fn entry_matches(name: &str, description: &str, query_lower: &str) -> bool {
    name.to_ascii_lowercase().contains(query_lower)
        || description.to_ascii_lowercase().contains(query_lower)
}

fn print_tool_row(t: &HubToolEntry) {
    println!(
        "  {:<24}  {:<10}  {:<12}  {}",
        t.name,
        t.version,
        t.provenance.trust_label(),
        display_or_dash(&t.description)
    );
}

fn print_skill_row(s: &HubSkillEntry) {
    let version = if s.version.is_empty() {
        "-"
    } else {
        s.version.as_str()
    };
    println!(
        "  {:<24}  {:<10}  {:<12}  {}",
        s.name,
        version,
        s.provenance.trust_label(),
        display_or_dash(&s.description)
    );
}

fn display_or_dash(s: &str) -> &str {
    if s.is_empty() { "-" } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{HubArtifact, HubSkillEntry, HubToolEntry, Provenance};

    fn art(name: &str, ext: &str) -> HubArtifact {
        HubArtifact {
            url: format!(
                "https://github.com/nearai/ironhub/releases/download/test/{}.{}",
                name, ext
            ),
            size_bytes: 1024,
            sha256: "a".repeat(64),
        }
    }

    fn manifest_with(tools: Vec<&str>, skills: Vec<&str>) -> HubManifest {
        HubManifest {
            version: "1".into(),
            generated_at: "2026-05-14T00:00:00Z".into(),
            release_tag: "release-test".into(),
            repo: "nearai/ironhub".into(),
            tools: tools
                .into_iter()
                .map(|n| HubToolEntry {
                    name: n.into(),
                    crate_name: format!("{}-tool", n),
                    version: "0.1.0".into(),
                    description: format!("{} tool", n),
                    provenance: Provenance::Official,
                    wasm: art(n, "wasm"),
                    capabilities: art(n, "capabilities.json"),
                })
                .collect(),
            skills: skills
                .into_iter()
                .map(|n| HubSkillEntry {
                    name: n.into(),
                    trunk: String::new(),
                    version: "0.1.0".into(),
                    description: format!("{} skill", n),
                    provenance: Provenance::Official,
                    skill_md: art(n, "SKILL.md"),
                })
                .collect(),
        }
    }

    #[test]
    fn classify_picks_tool_when_only_in_tools() {
        let m = manifest_with(vec!["clickup"], vec!["chief-of-staff"]);
        assert!(matches!(
            classify(&m, "clickup", false, false).unwrap(),
            Kind::Tool
        ));
    }

    #[test]
    fn classify_picks_skill_when_only_in_skills() {
        let m = manifest_with(vec!["clickup"], vec!["chief-of-staff"]);
        assert!(matches!(
            classify(&m, "chief-of-staff", false, false).unwrap(),
            Kind::Skill
        ));
    }

    #[test]
    fn classify_requires_disambiguation_when_in_both() {
        let m = manifest_with(vec!["overlap"], vec!["overlap"]);
        let err = classify(&m, "overlap", false, false).expect_err("must error");
        assert!(err.to_string().contains("disambiguate"));
    }

    #[test]
    fn classify_honors_force_tool_flag() {
        let m = manifest_with(vec!["overlap"], vec!["overlap"]);
        assert!(matches!(
            classify(&m, "overlap", true, false).unwrap(),
            Kind::Tool
        ));
    }

    #[test]
    fn classify_honors_force_skill_flag() {
        let m = manifest_with(vec!["overlap"], vec!["overlap"]);
        assert!(matches!(
            classify(&m, "overlap", false, true).unwrap(),
            Kind::Skill
        ));
    }

    #[test]
    fn classify_force_tool_rejects_skill_only_name() {
        let m = manifest_with(vec![], vec!["chief-of-staff"]);
        let err = classify(&m, "chief-of-staff", true, false).expect_err("must error");
        assert!(err.to_string().contains("not a tool"));
    }

    #[test]
    fn classify_force_skill_rejects_tool_only_name() {
        let m = manifest_with(vec!["clickup"], vec![]);
        let err = classify(&m, "clickup", false, true).expect_err("must error");
        assert!(err.to_string().contains("not a skill"));
    }

    #[test]
    fn classify_unknown_name_suggests_nearest() {
        let m = manifest_with(vec!["clickup", "evm-rpc"], vec![]);
        let err = classify(&m, "click", false, false).expect_err("must error");
        let msg = err.to_string();
        assert!(msg.contains("Did you mean"));
        assert!(msg.contains("clickup"));
    }

    #[test]
    fn entry_matches_lowercases_name_against_already_lowercased_query() {
        assert!(entry_matches("ClickUp", "Task tracking", "clickup"));
        assert!(entry_matches("clickup", "Task tracking", "click"));
        assert!(!entry_matches("clickup", "Task tracking", "CLICK"));
    }

    #[test]
    fn entry_matches_searches_description() {
        assert!(entry_matches(
            "evm-rpc",
            "Ethereum RPC bindings",
            "ethereum"
        ));
        assert!(!entry_matches("evm-rpc", "Ethereum RPC bindings", "solana"));
    }

    #[test]
    fn nearest_matches_filters_by_substring_both_directions() {
        let m = manifest_with(vec!["clickup", "evm-rpc", "near-rpc"], vec![]);
        let hits = nearest_matches(&m, "rpc");
        assert!(hits.contains(&"evm-rpc".to_string()));
        assert!(hits.contains(&"near-rpc".to_string()));
        assert!(!hits.contains(&"clickup".to_string()));
    }

    fn manifest_with_one_new_tool(name: &str) -> HubManifest {
        let mut m = manifest_with(vec![name], vec![]);
        if let Some(tool) = m.tools.first_mut() {
            tool.provenance = Provenance::New;
        }
        m
    }

    #[tokio::test]
    async fn install_with_manifest_rejects_provenance_new_without_acknowledgement() {
        let manifest = manifest_with_one_new_tool("indie-tool");
        let err = install_with_manifest(
            manifest,
            "indie-tool",
            false,
            false,
            None,
            None,
            false,
            false,
        )
        .await
        .expect_err("community-unverified entry without --acknowledge-unverified must bail");
        let msg = err.to_string();
        assert!(
            msg.contains("UNVERIFIED") && msg.contains("--acknowledge-unverified"),
            "CLI gate error must name the flag the operator needs to set, got: {msg}"
        );
    }

    #[tokio::test]
    async fn install_with_manifest_passes_gate_with_acknowledgement() {
        let manifest = manifest_with_one_new_tool("indie-tool");
        let result = install_with_manifest(
            manifest,
            "indie-tool",
            false,
            false,
            None,
            None,
            false,
            true,
        )
        .await;
        match result {
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    !msg.contains("UNVERIFIED"),
                    "ack=true must clear the gate; any UNVERIFIED bail here means the gate \
                     fired despite acknowledgement, got: {msg}"
                );
            }
            Ok(_) => panic!(
                "test-fixture artifact URLs cannot resolve in a unit-test environment; \
                 an Ok result means the install pipeline silently bypassed network entirely"
            ),
        }
    }
}
