//! Registry CLI commands for discovering and installing extensions.

use std::path::PathBuf;

use clap::Subcommand;

use crate::registry::catalog::RegistryCatalog;
use crate::registry::installer::RegistryInstaller;
use crate::registry::manifest::ManifestKind;

#[derive(Subcommand, Debug, Clone)]
pub enum RegistryCommand {
    /// List available extensions in the registry
    List {
        /// Filter by kind: "tool" or "channel"
        #[arg(short, long)]
        kind: Option<String>,

        /// Filter by tag (e.g. "default", "google", "messaging")
        #[arg(short, long)]
        tag: Option<String>,

        /// Show detailed information
        #[arg(short, long)]
        verbose: bool,
    },

    /// Show detailed information about an extension or bundle
    Info {
        /// Extension or bundle name (e.g. "slack", "google", "tools/gmail")
        name: String,
    },

    /// Install an extension or bundle from the registry
    Install {
        /// Extension or bundle name (e.g. "slack", "google", "default")
        name: String,

        /// Force overwrite if already installed
        #[arg(short, long)]
        force: bool,

        /// Build from source instead of downloading pre-built artifact
        #[arg(long)]
        build: bool,
    },

    /// Install the default bundle of recommended extensions
    InstallDefaults {
        /// Force overwrite if already installed
        #[arg(short, long)]
        force: bool,
    },
}

/// Run a registry command.
pub async fn run_registry_command(cmd: RegistryCommand) -> anyhow::Result<()> {
    let registry_dir = find_registry_dir()?;
    let catalog = RegistryCatalog::load(&registry_dir)?;

    match cmd {
        RegistryCommand::List { kind, tag, verbose } => {
            cmd_list(&catalog, kind.as_deref(), tag.as_deref(), verbose)
        }
        RegistryCommand::Info { name } => cmd_info(&catalog, &name),
        RegistryCommand::Install { name, force, build } => {
            cmd_install(&catalog, &registry_dir, &name, force, build).await
        }
        RegistryCommand::InstallDefaults { force } => {
            cmd_install(&catalog, &registry_dir, "default", force, true).await
        }
    }
}

/// Find the registry directory by looking relative to the current executable or cwd.
fn find_registry_dir() -> anyhow::Result<PathBuf> {
    // Try relative to current directory (for dev usage)
    let cwd = std::env::current_dir()?;
    let candidate = cwd.join("registry");
    if candidate.is_dir() {
        return Ok(candidate);
    }

    // Try relative to executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let candidate = parent.join("registry");
            if candidate.is_dir() {
                return Ok(candidate);
            }
            // Also try one level up (for target/release/ironclaw)
            if let Some(grandparent) = parent.parent() {
                let candidate = grandparent.join("registry");
                if candidate.is_dir() {
                    return Ok(candidate);
                }
            }
        }
    }

    anyhow::bail!(
        "Could not find registry/ directory. Run from the ironclaw repo root, \
         or ensure registry/ is next to the ironclaw binary."
    )
}

fn cmd_list(
    catalog: &RegistryCatalog,
    kind: Option<&str>,
    tag: Option<&str>,
    verbose: bool,
) -> anyhow::Result<()> {
    let kind_filter = match kind {
        Some("tool" | "tools") => Some(ManifestKind::Tool),
        Some("channel" | "channels") => Some(ManifestKind::Channel),
        Some(other) => anyhow::bail!("Unknown kind '{}'. Use 'tool' or 'channel'.", other),
        None => None,
    };

    let manifests = catalog.list(kind_filter, tag);

    if manifests.is_empty() {
        println!("No extensions found matching the criteria.");
        return Ok(());
    }

    // Print header
    if verbose {
        println!(
            "{:<20} {:<8} {:<8} {:<10} {}",
            "NAME", "KIND", "VERSION", "AUTH", "DESCRIPTION"
        );
        println!("{}", "-".repeat(80));
    } else {
        println!("{:<20} {:<8} {}", "NAME", "KIND", "DESCRIPTION");
        println!("{}", "-".repeat(60));
    }

    for m in &manifests {
        if verbose {
            let auth = m
                .auth_summary
                .as_ref()
                .and_then(|a| a.method.as_deref())
                .unwrap_or("none");
            println!(
                "{:<20} {:<8} {:<8} {:<10} {}",
                m.name, m.kind, m.version, auth, m.description
            );
        } else {
            println!("{:<20} {:<8} {}", m.name, m.kind, m.description);
        }
    }

    println!("\n{} extension(s) found.", manifests.len());

    // Show bundles hint
    let bundle_names = catalog.bundle_names();
    if !bundle_names.is_empty() {
        println!("\nBundles available: {}", bundle_names.join(", "));
        println!("Use `ironclaw registry info <bundle>` for details.");
    }

    Ok(())
}

fn cmd_info(catalog: &RegistryCatalog, name: &str) -> anyhow::Result<()> {
    // Check if it's a bundle
    if let Some(bundle) = catalog.get_bundle(name) {
        println!("Bundle: {}", bundle.display_name);
        if let Some(desc) = &bundle.description {
            println!("  {}", desc);
        }
        println!("\nExtensions:");
        for ext_key in &bundle.extensions {
            if let Some(m) = catalog.get(ext_key) {
                println!("  {} - {} ({})", ext_key, m.description, m.kind);
            } else {
                println!("  {} (not found in registry)", ext_key);
            }
        }
        if let Some(shared) = &bundle.shared_auth {
            println!("\nShared auth: {}", shared);
        }
        return Ok(());
    }

    // Single extension (use get_strict to surface ambiguous bare names)
    let manifest = catalog
        .get_strict(name)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    println!("{} ({})", manifest.display_name, manifest.kind);
    println!("  Version: {}", manifest.version);
    println!("  {}", manifest.description);

    if !manifest.keywords.is_empty() {
        println!("  Keywords: {}", manifest.keywords.join(", "));
    }

    println!("\nSource:");
    println!("  Directory: {}", manifest.source.dir);
    println!("  Crate: {}", manifest.source.crate_name);
    println!("  Capabilities: {}", manifest.source.capabilities);

    if let Some(artifact) = manifest.artifacts.get("wasm32-wasip2") {
        println!("\nArtifact (wasm32-wasip2):");
        match &artifact.url {
            Some(url) => println!("  URL: {}", url),
            None => println!("  URL: (not yet published)"),
        }
        match &artifact.sha256 {
            Some(sha) => println!("  SHA256: {}", sha),
            None => println!("  SHA256: (not yet computed)"),
        }
    }

    if let Some(auth) = &manifest.auth_summary {
        println!("\nAuthentication:");
        if let Some(method) = &auth.method {
            println!("  Method: {}", method);
        }
        if let Some(provider) = &auth.provider {
            println!("  Provider: {}", provider);
        }
        if !auth.secrets.is_empty() {
            println!("  Secrets: {}", auth.secrets.join(", "));
        }
        if let Some(shared) = &auth.shared_auth {
            println!("  Shared with: {}", shared);
        }
        if let Some(url) = &auth.setup_url {
            println!("  Setup: {}", url);
        }
    }

    if !manifest.tags.is_empty() {
        println!("\nTags: {}", manifest.tags.join(", "));
    }

    Ok(())
}

async fn cmd_install(
    catalog: &RegistryCatalog,
    registry_dir: &std::path::Path,
    name: &str,
    force: bool,
    prefer_build: bool,
) -> anyhow::Result<()> {
    // Registry dir parent is the repo root
    let repo_root = registry_dir
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine repo root from registry dir"))?;

    let installer = RegistryInstaller::with_defaults(repo_root.to_path_buf());

    let (manifests, bundle) = catalog.resolve(name)?;

    if manifests.is_empty() {
        anyhow::bail!("No extensions found for '{}'.", name);
    }

    if let Some(bundle_def) = bundle {
        // Bundle install
        println!(
            "Installing bundle '{}' ({} extensions)...\n",
            bundle_def.display_name,
            manifests.len()
        );

        let (outcomes, hints) = installer
            .install_bundle(&manifests, bundle_def, force, prefer_build)
            .await;

        println!("\n--- Results ---");
        for outcome in &outcomes {
            let caps_status = if outcome.has_capabilities { "+" } else { "-" };
            println!(
                "  [{}] {} ({}) -> {}",
                caps_status,
                outcome.name,
                outcome.kind,
                outcome.wasm_path.display()
            );
            for w in &outcome.warnings {
                println!("      Warning: {}", w);
            }
        }

        if !hints.is_empty() {
            println!("\nAuth setup:");
            for hint in &hints {
                println!("{}", hint);
            }
        }

        println!(
            "\nInstalled {}/{} extensions.",
            outcomes.len(),
            manifests.len()
        );
    } else {
        // Single extension
        let manifest = manifests[0];
        let outcome = installer.install(manifest, force, prefer_build).await?;

        println!("\nInstalled successfully:");
        println!("  Name: {}", outcome.name);
        println!("  Kind: {}", outcome.kind);
        println!("  WASM: {}", outcome.wasm_path.display());
        println!("  Capabilities: {}", outcome.has_capabilities);

        if let Some(auth) = &manifest.auth_summary {
            if auth.method.as_deref() != Some("none") {
                println!(
                    "\nNext step: authenticate with `ironclaw tool auth {}`",
                    manifest.name
                );
                if let Some(url) = &auth.setup_url {
                    println!("  Setup credentials at: {}", url);
                }
            }
        }
    }

    Ok(())
}
