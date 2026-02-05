//! Tool management CLI commands.
//!
//! Commands for installing, listing, and removing WASM tools.

use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use clap::Subcommand;
use tokio::fs;

use crate::tools::wasm::{CapabilitiesFile, compute_binary_hash};

/// Default tools directory.
fn default_tools_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".ironclaw").join("tools"))
        .unwrap_or_else(|| PathBuf::from(".ironclaw/tools"))
}

#[derive(Subcommand, Debug, Clone)]
pub enum ToolCommand {
    /// Install a WASM tool from source directory or .wasm file
    Install {
        /// Path to tool source directory (with Cargo.toml) or .wasm file
        path: PathBuf,

        /// Tool name (defaults to directory/file name)
        #[arg(short, long)]
        name: Option<String>,

        /// Path to capabilities JSON file (auto-detected if not specified)
        #[arg(long)]
        capabilities: Option<PathBuf>,

        /// Target directory for installation (default: ~/.ironclaw/tools/)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Build in release mode (default: true)
        #[arg(long, default_value = "true")]
        release: bool,

        /// Skip compilation (use existing .wasm file)
        #[arg(long)]
        skip_build: bool,

        /// Force overwrite if tool already exists
        #[arg(short, long)]
        force: bool,
    },

    /// List installed tools
    List {
        /// Directory to list tools from (default: ~/.ironclaw/tools/)
        #[arg(short, long)]
        dir: Option<PathBuf>,

        /// Show detailed information
        #[arg(short, long)]
        verbose: bool,
    },

    /// Remove an installed tool
    Remove {
        /// Name of the tool to remove
        name: String,

        /// Directory to remove tool from (default: ~/.ironclaw/tools/)
        #[arg(short, long)]
        dir: Option<PathBuf>,
    },

    /// Show information about a tool
    Info {
        /// Name of the tool or path to .wasm file
        name_or_path: String,

        /// Directory to look for tool (default: ~/.ironclaw/tools/)
        #[arg(short, long)]
        dir: Option<PathBuf>,
    },
}

/// Run a tool command.
pub async fn run_tool_command(cmd: ToolCommand) -> anyhow::Result<()> {
    match cmd {
        ToolCommand::Install {
            path,
            name,
            capabilities,
            target,
            release,
            skip_build,
            force,
        } => install_tool(path, name, capabilities, target, release, skip_build, force).await,
        ToolCommand::List { dir, verbose } => list_tools(dir, verbose).await,
        ToolCommand::Remove { name, dir } => remove_tool(name, dir).await,
        ToolCommand::Info { name_or_path, dir } => show_tool_info(name_or_path, dir).await,
    }
}

/// Install a WASM tool.
async fn install_tool(
    path: PathBuf,
    name: Option<String>,
    capabilities: Option<PathBuf>,
    target: Option<PathBuf>,
    release: bool,
    skip_build: bool,
    force: bool,
) -> anyhow::Result<()> {
    let target_dir = target.unwrap_or_else(default_tools_dir);

    // Determine if path is a directory (source) or .wasm file
    let metadata = fs::metadata(&path).await?;

    let (wasm_path, tool_name, caps_path) = if metadata.is_dir() {
        // Source directory, need to build
        let cargo_toml = path.join("Cargo.toml");
        if !cargo_toml.exists() {
            anyhow::bail!(
                "No Cargo.toml found in {}. Expected a Rust WASM tool source directory.",
                path.display()
            );
        }

        // Extract tool name from Cargo.toml or use provided name
        let tool_name = if let Some(n) = name {
            n
        } else {
            extract_crate_name(&cargo_toml).await?
        };

        // Build the WASM component if not skipping
        let wasm_path = if skip_build {
            // Look for existing wasm file
            find_wasm_artifact(&path, &tool_name, release)?
        } else {
            build_wasm_component(&path, release)?
        };

        // Look for capabilities file
        let caps_path = capabilities.or_else(|| {
            let candidates = [
                path.join(format!("{}.capabilities.json", tool_name)),
                path.join("capabilities.json"),
            ];
            candidates.into_iter().find(|p| p.exists())
        });

        (wasm_path, tool_name, caps_path)
    } else if path.extension().map(|e| e == "wasm").unwrap_or(false) {
        // Direct .wasm file
        let tool_name = name.unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string()
        });

        // Look for capabilities file next to wasm
        let caps_path = capabilities.or_else(|| {
            let candidates = [
                path.with_extension("capabilities.json"),
                path.parent()
                    .map(|p| p.join(format!("{}.capabilities.json", tool_name)))
                    .unwrap_or_default(),
            ];
            candidates.into_iter().find(|p| p.exists())
        });

        (path, tool_name, caps_path)
    } else {
        anyhow::bail!(
            "Expected a directory with Cargo.toml or a .wasm file, got: {}",
            path.display()
        );
    };

    // Ensure target directory exists
    fs::create_dir_all(&target_dir).await?;

    // Target paths
    let target_wasm = target_dir.join(format!("{}.wasm", tool_name));
    let target_caps = target_dir.join(format!("{}.capabilities.json", tool_name));

    // Check if already exists
    if target_wasm.exists() && !force {
        anyhow::bail!(
            "Tool '{}' already exists at {}. Use --force to overwrite.",
            tool_name,
            target_wasm.display()
        );
    }

    // Validate capabilities file if provided
    if let Some(ref caps) = caps_path {
        let content = fs::read_to_string(caps).await?;
        CapabilitiesFile::from_json(&content)
            .map_err(|e| anyhow::anyhow!("Invalid capabilities file {}: {}", caps.display(), e))?;
    }

    // Copy WASM file
    println!("Installing {} to {}", tool_name, target_wasm.display());
    fs::copy(&wasm_path, &target_wasm).await?;

    // Copy capabilities file if present
    if let Some(caps) = caps_path {
        println!("  Copying capabilities from {}", caps.display());
        fs::copy(&caps, &target_caps).await?;
    } else {
        println!("  Warning: No capabilities file found. Tool will have no permissions.");
    }

    // Calculate and display hash
    let wasm_bytes = fs::read(&target_wasm).await?;
    let hash = compute_binary_hash(&wasm_bytes);
    let hash_hex: String = hash.iter().map(|b| format!("{:02x}", b)).collect();

    println!("\nInstalled successfully:");
    println!("  Name: {}", tool_name);
    println!("  WASM: {}", target_wasm.display());
    println!("  Size: {} bytes", wasm_bytes.len());
    println!("  Hash: {}", &hash_hex[..16]); // Show first 16 chars

    if target_caps.exists() {
        println!("  Caps: {}", target_caps.display());
    }

    Ok(())
}

/// Build a WASM component using cargo-component.
fn build_wasm_component(source_dir: &Path, release: bool) -> anyhow::Result<PathBuf> {
    println!("Building WASM component in {}...", source_dir.display());

    // Check if cargo-component is available
    let check = ProcessCommand::new("cargo")
        .args(["component", "--version"])
        .output();

    if check.is_err() || !check.unwrap().status.success() {
        anyhow::bail!(
            "cargo-component not found. Install with: cargo install cargo-component\n\
             Or use --skip-build with an existing .wasm file."
        );
    }

    // Build command
    let mut cmd = ProcessCommand::new("cargo");
    cmd.current_dir(source_dir).args(["component", "build"]);

    if release {
        cmd.arg("--release");
    }

    println!(
        "  Running: cargo component build{}",
        if release { " --release" } else { "" }
    );

    let output = cmd.output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Build failed:\n{}", stderr);
    }

    // Find the output wasm file
    // cargo-component may output to wasm32-wasip1 or wasm32-wasip2 depending on version
    let profile = if release { "release" } else { "debug" };
    let candidates = [
        source_dir
            .join("target")
            .join("wasm32-wasip1")
            .join(profile),
        source_dir
            .join("target")
            .join("wasm32-wasip2")
            .join(profile),
        source_dir
            .join("target")
            .join("wasm32-unknown-unknown")
            .join(profile),
    ];

    let target_dir = candidates.iter().find(|p| p.exists()).ok_or_else(|| {
        anyhow::anyhow!(
            "No WASM target directory found. Expected one of: {}",
            candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;

    // Look for .wasm files in target dir
    let entries: Vec<_> = std::fs::read_dir(target_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "wasm")
                .unwrap_or(false)
        })
        .collect();

    if entries.is_empty() {
        anyhow::bail!(
            "No .wasm file found in {}. Build may have failed.",
            target_dir.display()
        );
    }

    if entries.len() > 1 {
        println!(
            "  Warning: Multiple .wasm files found, using first: {}",
            entries[0].path().display()
        );
    }

    let wasm_path = entries[0].path();
    println!("  Built: {}", wasm_path.display());

    Ok(wasm_path)
}

/// Find an existing WASM artifact without building.
fn find_wasm_artifact(source_dir: &Path, name: &str, release: bool) -> anyhow::Result<PathBuf> {
    let profile = if release { "release" } else { "debug" };

    // cargo-component may output to wasm32-wasip1 or wasm32-wasip2 depending on version
    let target_dirs = [
        source_dir
            .join("target")
            .join("wasm32-wasip1")
            .join(profile),
        source_dir
            .join("target")
            .join("wasm32-wasip2")
            .join(profile),
        source_dir
            .join("target")
            .join("wasm32-unknown-unknown")
            .join(profile),
    ];

    let snake_name = name.replace('-', "_");

    // Try exact name match in any target dir first
    for target_dir in &target_dirs {
        let candidates = [
            target_dir.join(format!("{}.wasm", name)),
            target_dir.join(format!("{}.wasm", snake_name)),
        ];
        for candidate in &candidates {
            if candidate.exists() {
                return Ok(candidate.clone());
            }
        }
    }

    // Find a target dir that exists
    let target_dir = target_dirs.iter().find(|p| p.exists()).ok_or_else(|| {
        anyhow::anyhow!("No target directory found. Run without --skip-build to build first.")
    })?;

    // Fall back to any .wasm file
    let entries: Vec<_> = std::fs::read_dir(target_dir)
        .map_err(|_| {
            anyhow::anyhow!(
                "Target directory not found: {}. Run without --skip-build.",
                target_dir.display()
            )
        })?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "wasm")
                .unwrap_or(false)
        })
        .collect();

    if entries.is_empty() {
        anyhow::bail!(
            "No .wasm file found in {}. Build the project first or remove --skip-build.",
            target_dir.display()
        );
    }

    Ok(entries[0].path())
}

/// Extract crate name from Cargo.toml.
async fn extract_crate_name(cargo_toml: &Path) -> anyhow::Result<String> {
    let content = fs::read_to_string(cargo_toml).await?;

    // Simple TOML parsing for [package] name
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("name") {
            if let Some((_, value)) = line.split_once('=') {
                let name = value.trim().trim_matches('"').trim_matches('\'');
                return Ok(name.to_string());
            }
        }
    }

    anyhow::bail!(
        "Could not extract package name from {}",
        cargo_toml.display()
    )
}

/// List installed tools.
async fn list_tools(dir: Option<PathBuf>, verbose: bool) -> anyhow::Result<()> {
    let tools_dir = dir.unwrap_or_else(default_tools_dir);

    if !tools_dir.exists() {
        println!("No tools directory found at {}", tools_dir.display());
        println!("Install a tool with: ironclaw tool install <path>");
        return Ok(());
    }

    let mut entries = fs::read_dir(&tools_dir).await?;
    let mut tools = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().map(|e| e == "wasm").unwrap_or(false) {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            let caps_path = path.with_extension("capabilities.json");
            let has_caps = caps_path.exists();

            let size = fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);

            tools.push((name, path, has_caps, size));
        }
    }

    if tools.is_empty() {
        println!("No tools installed in {}", tools_dir.display());
        return Ok(());
    }

    tools.sort_by(|a, b| a.0.cmp(&b.0));

    println!("Installed tools in {}:", tools_dir.display());
    println!();

    for (name, path, has_caps, size) in tools {
        if verbose {
            let wasm_bytes = fs::read(&path).await?;
            let hash = compute_binary_hash(&wasm_bytes);
            let hash_hex: String = hash.iter().take(8).map(|b| format!("{:02x}", b)).collect();

            println!("  {} ({})", name, format_size(size));
            println!("    Path: {}", path.display());
            println!("    Hash: {}", hash_hex);
            println!("    Caps: {}", if has_caps { "yes" } else { "no" });

            if has_caps {
                let caps_path = path.with_extension("capabilities.json");
                if let Ok(content) = fs::read_to_string(&caps_path).await {
                    if let Ok(caps) = CapabilitiesFile::from_json(&content) {
                        print_capabilities_summary(&caps);
                    }
                }
            }
            println!();
        } else {
            let caps_indicator = if has_caps { "✓" } else { "✗" };
            println!(
                "  {} ({}, caps: {})",
                name,
                format_size(size),
                caps_indicator
            );
        }
    }

    Ok(())
}

/// Remove an installed tool.
async fn remove_tool(name: String, dir: Option<PathBuf>) -> anyhow::Result<()> {
    let tools_dir = dir.unwrap_or_else(default_tools_dir);

    let wasm_path = tools_dir.join(format!("{}.wasm", name));
    let caps_path = tools_dir.join(format!("{}.capabilities.json", name));

    if !wasm_path.exists() {
        anyhow::bail!("Tool '{}' not found in {}", name, tools_dir.display());
    }

    fs::remove_file(&wasm_path).await?;
    println!("Removed {}", wasm_path.display());

    if caps_path.exists() {
        fs::remove_file(&caps_path).await?;
        println!("Removed {}", caps_path.display());
    }

    println!("\nTool '{}' removed.", name);
    Ok(())
}

/// Show information about a tool.
async fn show_tool_info(name_or_path: String, dir: Option<PathBuf>) -> anyhow::Result<()> {
    let wasm_path = if name_or_path.ends_with(".wasm") {
        PathBuf::from(&name_or_path)
    } else {
        let tools_dir = dir.unwrap_or_else(default_tools_dir);
        tools_dir.join(format!("{}.wasm", name_or_path))
    };

    if !wasm_path.exists() {
        anyhow::bail!("Tool not found: {}", wasm_path.display());
    }

    let wasm_bytes = fs::read(&wasm_path).await?;
    let hash = compute_binary_hash(&wasm_bytes);
    let hash_hex: String = hash.iter().map(|b| format!("{:02x}", b)).collect();

    let name = wasm_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    println!("Tool: {}", name);
    println!("Path: {}", wasm_path.display());
    println!(
        "Size: {} bytes ({})",
        wasm_bytes.len(),
        format_size(wasm_bytes.len() as u64)
    );
    println!("Hash: {}", hash_hex);

    let caps_path = wasm_path.with_extension("capabilities.json");
    if caps_path.exists() {
        println!("\nCapabilities ({}):", caps_path.display());
        let content = fs::read_to_string(&caps_path).await?;
        match CapabilitiesFile::from_json(&content) {
            Ok(caps) => print_capabilities_detail(&caps),
            Err(e) => println!("  Error parsing: {}", e),
        }
    } else {
        println!("\nNo capabilities file found.");
        println!("Tool will have no permissions (default deny).");
    }

    Ok(())
}

/// Format bytes as human-readable size.
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;

    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Print a brief capabilities summary.
fn print_capabilities_summary(caps: &CapabilitiesFile) {
    let mut parts = Vec::new();

    if let Some(ref http) = caps.http {
        let hosts: Vec<_> = http.allowlist.iter().map(|e| e.host.as_str()).collect();
        if !hosts.is_empty() {
            parts.push(format!("http: {}", hosts.join(", ")));
        }
    }

    if let Some(ref secrets) = caps.secrets {
        if !secrets.allowed_names.is_empty() {
            parts.push(format!("secrets: {}", secrets.allowed_names.len()));
        }
    }

    if let Some(ref ws) = caps.workspace {
        if !ws.allowed_prefixes.is_empty() {
            parts.push("workspace: read".to_string());
        }
    }

    if !parts.is_empty() {
        println!("    Perms: {}", parts.join(", "));
    }
}

/// Print detailed capabilities.
fn print_capabilities_detail(caps: &CapabilitiesFile) {
    if let Some(ref http) = caps.http {
        println!("  HTTP:");
        for endpoint in &http.allowlist {
            let methods = if endpoint.methods.is_empty() {
                "*".to_string()
            } else {
                endpoint.methods.join(", ")
            };
            let path = endpoint.path_prefix.as_deref().unwrap_or("/*");
            println!("    {} {} {}", methods, endpoint.host, path);
        }

        if !http.credentials.is_empty() {
            println!("  Credentials:");
            for (key, cred) in &http.credentials {
                println!("    {}: {} -> {:?}", key, cred.secret_name, cred.location);
            }
        }

        if let Some(ref rate) = http.rate_limit {
            println!(
                "  Rate limit: {}/min, {}/hour",
                rate.requests_per_minute, rate.requests_per_hour
            );
        }
    }

    if let Some(ref secrets) = caps.secrets {
        if !secrets.allowed_names.is_empty() {
            println!("  Secrets (existence check only):");
            for name in &secrets.allowed_names {
                println!("    {}", name);
            }
        }
    }

    if let Some(ref tool_invoke) = caps.tool_invoke {
        if !tool_invoke.aliases.is_empty() {
            println!("  Tool aliases:");
            for (alias, real_name) in &tool_invoke.aliases {
                println!("    {} -> {}", alias, real_name);
            }
        }
    }

    if let Some(ref ws) = caps.workspace {
        if !ws.allowed_prefixes.is_empty() {
            println!("  Workspace read prefixes:");
            for prefix in &ws.allowed_prefixes {
                println!("    {}", prefix);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1048576), "1.0 MB");
        assert_eq!(format_size(2621440), "2.5 MB");
    }

    #[test]
    fn test_default_tools_dir() {
        let dir = default_tools_dir();
        assert!(dir.to_string_lossy().contains(".ironclaw"));
        assert!(dir.to_string_lossy().contains("tools"));
    }
}
