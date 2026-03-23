//! Tool management CLI commands.
//!
//! Commands for installing, listing, removing, and authenticating WASM tools.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Subcommand;
use tokio::fs;

use crate::bootstrap::ironclaw_base_dir;
use crate::secrets::{CreateSecretParams, SecretsStore};
use crate::tools::wasm::{CapabilitiesFile, compute_binary_hash};

/// Default tools directory.
fn default_tools_dir() -> PathBuf {
    ironclaw_base_dir().join("tools")
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

        /// User ID for checking credential status (default: "default")
        #[arg(short, long, default_value = "default")]
        user: String,
    },

    /// Configure authentication for a tool
    Auth {
        /// Name of the tool
        name: String,

        /// Directory to look for tool (default: ~/.ironclaw/tools/)
        #[arg(short, long)]
        dir: Option<PathBuf>,

        /// User ID for storing the secret (default: "default")
        #[arg(short, long, default_value = "default")]
        user: String,
    },

    /// Configure required secrets for a tool (from setup.required_secrets)
    Setup {
        /// Name of the tool
        name: String,

        /// Directory to look for tool (default: ~/.ironclaw/tools/)
        #[arg(short, long)]
        dir: Option<PathBuf>,

        /// User ID for storing the secret (default: "default")
        #[arg(short, long, default_value = "default")]
        user: String,
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
        ToolCommand::Info {
            name_or_path,
            dir,
            user,
        } => show_tool_info(name_or_path, dir, user).await,
        ToolCommand::Auth { name, dir, user } => auth_tool(name, dir, user).await,
        ToolCommand::Setup { name, dir, user } => setup_tool(name, dir, user).await,
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
        let profile = if release { "release" } else { "debug" };
        let wasm_path = if skip_build {
            // Look for existing wasm file
            crate::registry::artifacts::find_wasm_artifact(&path, &tool_name, profile)
                .or_else(|| crate::registry::artifacts::find_any_wasm_artifact(&path, profile))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "No .wasm artifact found. Run without --skip-build to build first."
                    )
                })?
        } else {
            crate::registry::artifacts::build_wasm_component_sync(&path, release)?
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

/// Extract crate name from Cargo.toml.
async fn extract_crate_name(cargo_toml: &Path) -> anyhow::Result<String> {
    let content = fs::read_to_string(cargo_toml).await?;

    // Simple TOML parsing for [package] name
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("name")
            && let Some((_, value)) = line.split_once('=')
        {
            let name = value.trim().trim_matches('"').trim_matches('\'');
            return Ok(name.to_string());
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
                if let Ok(content) = fs::read_to_string(&caps_path).await
                    && let Ok(caps) = CapabilitiesFile::from_json(&content)
                {
                    print_capabilities_summary(&caps);
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
async fn show_tool_info(
    name_or_path: String,
    dir: Option<PathBuf>,
    user_id: String,
) -> anyhow::Result<()> {
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
            Ok(caps) => {
                // Lazily init secrets store only when auth secrets need checking.
                let has_auth = caps.auth.is_some()
                    || caps
                        .setup
                        .as_ref()
                        .is_some_and(|s| !s.required_secrets.is_empty())
                    || caps
                        .http
                        .as_ref()
                        .is_some_and(|h| !h.credentials.is_empty());
                let secrets_store = if has_auth {
                    match init_secrets_store().await {
                        Ok(store) => Some(store),
                        Err(e) => {
                            eprintln!("  Warning: could not init secrets store: {}", e);
                            None
                        }
                    }
                } else {
                    None
                };
                print_capabilities_detail(
                    &caps,
                    secrets_store
                        .as_ref()
                        .map(|s| s.as_ref() as &(dyn SecretsStore + Send + Sync)),
                    &user_id,
                )
                .await;
            }
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

    if let Some(ref secrets) = caps.secrets
        && !secrets.allowed_names.is_empty()
    {
        parts.push(format!("secrets: {}", secrets.allowed_names.len()));
    }

    if let Some(ref ws) = caps.workspace
        && !ws.allowed_prefixes.is_empty()
    {
        parts.push("workspace: read".to_string());
    }

    if !parts.is_empty() {
        println!("    Perms: {}", parts.join(", "));
    }
}

/// Per-secret info collected from all auth-related capability sections.
struct AuthSecretInfo {
    secret_name: String,
    /// Human-readable label (from auth.display_name or setup prompt).
    description: Option<String>,
    /// Injection location (from http.credentials).
    location: Option<String>,
}

/// Collected auth secrets and the set of secret names they cover.
struct CollectedAuthSecrets {
    secrets: Vec<AuthSecretInfo>,
    /// Secret names present in `secrets`, for filtering the Secrets capability section.
    seen_names: std::collections::HashSet<String>,
}

/// Collect and deduplicate auth secrets from all auth-related capability sections.
///
/// Priority for the description label: auth.display_name > setup.required_secrets.prompt.
/// Injection location is merged from http.credentials.
fn collect_auth_secrets(caps: &CapabilitiesFile) -> CollectedAuthSecrets {
    let mut secrets: Vec<AuthSecretInfo> = Vec::new();
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    // auth.display_name is the best label — seed first.
    if let Some(ref auth) = caps.auth {
        let index = secrets.len();
        seen.insert(auth.secret_name.clone(), index);
        secrets.push(AuthSecretInfo {
            secret_name: auth.secret_name.clone(),
            description: auth.display_name.clone(),
            location: None,
        });
    }

    // setup.required_secrets.prompt is second-best label.
    if let Some(ref setup) = caps.setup {
        for secret in &setup.required_secrets {
            if !seen.contains_key(&secret.name) {
                let index = secrets.len();
                seen.insert(secret.name.clone(), index);
                secrets.push(AuthSecretInfo {
                    secret_name: secret.name.clone(),
                    description: Some(secret.prompt.clone()),
                    location: None,
                });
            }
        }
    }

    // Merge injection location from http.credentials.
    if let Some(ref http) = caps.http {
        for cred in http.credentials.values() {
            let loc = format!("{:?}", cred.location);
            if let Some(&index) = seen.get(&cred.secret_name) {
                secrets[index].location = Some(loc);
            } else {
                let index = secrets.len();
                seen.insert(cred.secret_name.clone(), index);
                secrets.push(AuthSecretInfo {
                    secret_name: cred.secret_name.clone(),
                    description: None,
                    location: Some(loc),
                });
            }
        }
    }

    let seen_names = seen.into_keys().collect();
    CollectedAuthSecrets {
        secrets,
        seen_names,
    }
}

/// Print detailed capabilities.
async fn print_capabilities_detail(
    caps: &CapabilitiesFile,
    secrets_store: Option<&(dyn SecretsStore + Send + Sync)>,
    user_id: &str,
) {
    let mut collected = collect_auth_secrets(caps);

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

        if let Some(ref rate) = http.rate_limit {
            println!(
                "  Rate limit: {}/min, {}/hour",
                rate.requests_per_minute, rate.requests_per_hour
            );
        }
    }

    // Filter secrets already covered by the auth section (always rendered when non-empty).
    if let Some(ref secrets) = caps.secrets
        && !secrets.allowed_names.is_empty()
    {
        let extra: Vec<_> = if collected.secrets.is_empty() {
            secrets.allowed_names.iter().collect()
        } else {
            secrets
                .allowed_names
                .iter()
                .filter(|name| !collected.seen_names.contains(name.as_str()))
                .collect()
        };
        if !extra.is_empty() {
            println!("  Secrets (existence check only):");
            for name in extra {
                println!("    {}", name);
            }
        }
    }

    if let Some(ref tool_invoke) = caps.tool_invoke
        && !tool_invoke.aliases.is_empty()
    {
        println!("  Tool aliases:");
        for (alias, real_name) in &tool_invoke.aliases {
            println!("    {} -> {}", alias, real_name);
        }
    }

    if let Some(ref ws) = caps.workspace
        && !ws.allowed_prefixes.is_empty()
    {
        println!("  Workspace read prefixes:");
        for prefix in &ws.allowed_prefixes {
            println!("    {}", prefix);
        }
    }

    // Consolidated auth status — sorted by secret name for deterministic output.
    if !collected.secrets.is_empty() {
        collected
            .secrets
            .sort_by(|a, b| a.secret_name.cmp(&b.secret_name));
        println!("  Auth:");
        for info in &collected.secrets {
            let (icon, label) = match secrets_store {
                Some(store) => match store.exists(user_id, &info.secret_name).await {
                    Ok(true) => ("\u{2713}", "configured"),
                    Ok(false) => ("\u{2717}", "missing"),
                    Err(e) => {
                        eprintln!(
                            "  Warning: failed to check secret `{}`: {}",
                            info.secret_name, e
                        );
                        ("?", "unknown")
                    }
                },
                None => ("?", "unknown"),
            };
            let mut parts = info.secret_name.clone();
            if let Some(ref desc) = info.description {
                parts = format!("{} ({})", parts, desc);
            }
            if let Some(ref loc) = info.location {
                parts = format!("{} -> {}", parts, loc);
            }
            println!("    {}  {} {}", parts, icon, label);
        }
    }
}

/// Validate a tool name to prevent path traversal.
fn validate_tool_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name.contains('\0')
    {
        anyhow::bail!(
            "Invalid tool name '{}': must not contain path separators or '..'",
            name
        );
    }
    Ok(())
}

/// Initialize the secrets store from environment config.
async fn init_secrets_store() -> anyhow::Result<Arc<dyn SecretsStore + Send + Sync>> {
    crate::cli::init_secrets_store().await
}

/// Configure authentication for a tool.
async fn auth_tool(name: String, dir: Option<PathBuf>, user_id: String) -> anyhow::Result<()> {
    validate_tool_name(&name)?;
    let tools_dir = dir.unwrap_or_else(default_tools_dir);
    let caps_path = tools_dir.join(format!("{}.capabilities.json", name));

    if !caps_path.exists() {
        anyhow::bail!(
            "Tool '{}' not found or has no capabilities file at {}",
            name,
            caps_path.display()
        );
    }

    // Parse capabilities
    let content = fs::read_to_string(&caps_path).await?;
    let caps = CapabilitiesFile::from_json(&content)
        .map_err(|e| anyhow::anyhow!("Invalid capabilities file: {}", e))?;

    // Check for auth section
    let auth = caps.auth.ok_or_else(|| {
        anyhow::anyhow!(
            "Tool '{}' has no auth configuration.\n\
             The tool may not require authentication, or auth setup is not defined.",
            name
        )
    })?;

    let display_name = auth.display_name.as_deref().unwrap_or(&name);

    let header = format!("{} Authentication", display_name);
    println!();
    println!("╔════════════════════════════════════════════════════════════════╗");
    println!("║  {:^62}║", header);
    println!("╚════════════════════════════════════════════════════════════════╝");
    println!();

    let secrets_store = init_secrets_store().await?;

    // Check if already configured
    let already_configured = secrets_store
        .exists(&user_id, &auth.secret_name)
        .await
        .unwrap_or(false);

    if already_configured {
        println!("  {} is already configured.", display_name);
        println!();
        print!("  Replace existing credentials? [y/N]: ");
        std::io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!();
            println!("  Keeping existing credentials.");
            return Ok(());
        }
        println!();
    }

    // Check for environment variable
    if let Some(ref env_var) = auth.env_var
        && let Ok(token) = std::env::var(env_var)
        && !token.is_empty()
    {
        println!("  Found {} in environment.", env_var);
        println!();

        // Validate if endpoint is provided
        if let Some(ref validation) = auth.validation_endpoint {
            print!("  Validating token...");
            std::io::stdout().flush()?;

            match validate_token(&token, validation, &auth.secret_name).await {
                Ok(()) => {
                    println!(" ✓");
                }
                Err(e) => {
                    println!(" ✗");
                    println!("  Validation failed: {}", e);
                    println!();
                    println!("  Falling back to manual entry...");
                    return auth_tool_manual(secrets_store.as_ref(), &user_id, &auth).await;
                }
            }
        }

        // Save the token
        save_token(secrets_store.as_ref(), &user_id, &auth, &token, None, None).await?;
        print_success(display_name);
        return Ok(());
    }

    // Check for OAuth configuration
    if let Some(ref oauth) = auth.oauth {
        // For providers with shared tokens, combine scopes from all installed
        // tools so one auth covers everything.
        let combined = combine_provider_scopes(&tools_dir, &auth.secret_name, oauth).await;
        if combined.scopes.len() > oauth.scopes.len() {
            let extra = combined.scopes.len() - oauth.scopes.len();
            println!(
                "  Including scopes from {} other installed tool(s) sharing this credential.",
                extra
            );
            println!();
        }
        return auth_tool_oauth(secrets_store.as_ref(), &user_id, &auth, &combined).await;
    }

    // Fall back to manual entry
    auth_tool_manual(secrets_store.as_ref(), &user_id, &auth).await
}

/// Scan the tools directory for all capabilities files sharing the same secret_name
/// and combine their OAuth scopes so one authorization covers the full shared
/// credential set.
async fn combine_provider_scopes(
    tools_dir: &Path,
    secret_name: &str,
    base_oauth: &crate::tools::wasm::OAuthConfigSchema,
) -> crate::tools::wasm::OAuthConfigSchema {
    let mut all_scopes: std::collections::HashSet<String> =
        base_oauth.scopes.iter().cloned().collect();

    if let Ok(mut entries) = tokio::fs::read_dir(tools_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if !name.ends_with(".capabilities.json") {
                continue;
            }

            if let Ok(content) = tokio::fs::read_to_string(&path).await
                && let Ok(caps) = CapabilitiesFile::from_json(&content)
                && let Some(auth) = &caps.auth
                && auth.secret_name == secret_name
                && let Some(oauth) = &auth.oauth
            {
                all_scopes.extend(oauth.scopes.iter().cloned());
            }
        }
    }

    let mut combined = base_oauth.clone();
    combined.scopes = all_scopes.into_iter().collect();
    combined.scopes.sort(); // deterministic ordering
    combined
}

/// OAuth browser-based login flow.
async fn auth_tool_oauth(
    store: &(dyn SecretsStore + Send + Sync),
    user_id: &str,
    auth: &crate::tools::wasm::AuthCapabilitySchema,
    oauth: &crate::tools::wasm::OAuthConfigSchema,
) -> anyhow::Result<()> {
    use crate::cli::oauth_defaults;

    let display_name = auth.display_name.as_deref().unwrap_or(&auth.secret_name);

    // Get client_id: capabilities file > runtime env var > built-in defaults
    let builtin = oauth_defaults::builtin_credentials(&auth.secret_name);

    let client_id = oauth
        .client_id
        .clone()
        .or_else(|| {
            oauth
                .client_id_env
                .as_ref()
                .and_then(|env| std::env::var(env).ok())
        })
        .or_else(|| builtin.as_ref().map(|c| c.client_id.to_string()))
        .ok_or_else(|| {
            let mut message = format!(
                "OAuth client_id not configured.\n\
                 Set {} env var",
                oauth.client_id_env.as_deref().unwrap_or("the client_id")
            );
            if let Some(override_env) =
                oauth_defaults::builtin_client_id_override_env(&auth.secret_name)
            {
                message.push_str(&format!(", or build with {override_env}"));
            }
            message.push('.');
            anyhow::anyhow!(message)
        })?;

    // Get client_secret: capabilities file > runtime env var > built-in defaults
    let client_secret = oauth
        .client_secret
        .clone()
        .or_else(|| {
            oauth
                .client_secret_env
                .as_ref()
                .and_then(|env| std::env::var(env).ok())
        })
        .or_else(|| builtin.as_ref().map(|c| c.client_secret.to_string()));

    println!("  Starting OAuth authentication...");
    println!();

    let listener = oauth_defaults::bind_callback_listener().await?;
    let redirect_uri = format!("{}/callback", oauth_defaults::callback_url());

    // Build authorization URL with PKCE and CSRF state
    let oauth_result = oauth_defaults::build_oauth_url(
        &oauth.authorization_url,
        &client_id,
        &redirect_uri,
        &oauth.scopes,
        oauth.use_pkce,
        &oauth.extra_params,
    );
    let code_verifier = oauth_result.code_verifier;

    println!("  Opening browser for {} login...", display_name);
    println!();

    if let Err(e) = open::that(&oauth_result.url) {
        println!("  Could not open browser: {}", e);
        println!("  Please open this URL manually:");
        println!("  {}", oauth_result.url);
    }

    println!("  Waiting for authorization...");

    let code = oauth_defaults::wait_for_callback(
        listener,
        "/callback",
        "code",
        display_name,
        Some(&oauth_result.state),
    )
    .await?;

    println!();
    println!("  Exchanging code for token...");

    // Exchange code for token
    let token_response = oauth_defaults::exchange_oauth_code(
        &oauth.token_url,
        &client_id,
        client_secret.as_deref(),
        &code,
        &redirect_uri,
        code_verifier.as_deref(),
        &oauth.access_token_field,
    )
    .await?;

    // Save tokens (access + refresh + scopes)
    oauth_defaults::store_oauth_tokens(
        store,
        user_id,
        &auth.secret_name,
        auth.provider.as_deref(),
        &token_response.access_token,
        token_response.refresh_token.as_deref(),
        token_response.expires_in,
        &oauth.scopes,
    )
    .await?;

    println!();
    println!("  ✓ {} connected!", display_name);
    println!();
    println!("  The tool can now access the API.");
    println!();

    Ok(())
}

/// Manual token entry flow.
async fn auth_tool_manual(
    store: &(dyn SecretsStore + Send + Sync),
    user_id: &str,
    auth: &crate::tools::wasm::AuthCapabilitySchema,
) -> anyhow::Result<()> {
    let display_name = auth.display_name.as_deref().unwrap_or(&auth.secret_name);

    // Show instructions
    if let Some(ref instructions) = auth.instructions {
        println!("  Setup instructions:");
        println!();
        for line in instructions.lines() {
            println!("    {}", line);
        }
        println!();
    }

    // Offer to open setup URL
    if let Some(ref url) = auth.setup_url {
        print!("  Press Enter to open setup page (or 's' to skip): ");
        std::io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("s") {
            if let Err(e) = open::that(url) {
                println!("  Could not open browser: {}", e);
                println!("  Please open manually: {}", url);
            } else {
                println!("  Opening browser...");
            }
        }
        println!();
    }

    // Show token hint
    if let Some(ref hint) = auth.token_hint {
        println!("  Token format: {}", hint);
        println!();
    }

    // Prompt for token
    print!("  Paste your token: ");
    std::io::stdout().flush()?;

    let token = read_hidden_input()?;
    println!();

    if token.is_empty() {
        println!("  No token provided. Aborting.");
        return Ok(());
    }

    // Validate if endpoint is provided
    if let Some(ref validation) = auth.validation_endpoint {
        print!("  Validating token...");
        std::io::stdout().flush()?;

        match validate_token(&token, validation, &auth.secret_name).await {
            Ok(()) => {
                println!(" ✓");
            }
            Err(e) => {
                println!(" ✗");
                println!("  Validation failed: {}", e);
                println!();
                print!("  Save anyway? [y/N]: ");
                std::io::stdout().flush()?;

                let mut confirm = String::new();
                std::io::stdin().read_line(&mut confirm)?;

                if !confirm.trim().eq_ignore_ascii_case("y") {
                    println!("  Aborting.");
                    return Ok(());
                }
            }
        }
    }

    // Save the token (manual path: no refresh token or expiry)
    save_token(store, user_id, auth, &token, None, None).await?;
    print_success(display_name);
    Ok(())
}

/// Read input with hidden characters.
fn read_hidden_input() -> anyhow::Result<String> {
    use crossterm::{
        event::{self, Event, KeyCode, KeyModifiers},
        terminal,
    };

    let mut input = String::new();

    terminal::enable_raw_mode()?;

    loop {
        if let Event::Key(key_event) = event::read()? {
            match key_event.code {
                KeyCode::Enter => {
                    break;
                }
                KeyCode::Backspace => {
                    if !input.is_empty() {
                        input.pop();
                        print!("\x08 \x08");
                        std::io::stdout().flush()?;
                    }
                }
                KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                    terminal::disable_raw_mode()?;
                    return Err(anyhow::anyhow!("Interrupted"));
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    print!("*");
                    std::io::stdout().flush()?;
                }
                _ => {}
            }
        }
    }

    terminal::disable_raw_mode()?;

    Ok(input)
}

/// Validate a token against the validation endpoint.
async fn validate_token(
    token: &str,
    validation: &crate::tools::wasm::ValidationEndpointSchema,
    _secret_name: &str,
) -> anyhow::Result<()> {
    crate::cli::oauth_defaults::validate_oauth_token(token, validation)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))
}

/// Save token to secrets store.
///
/// Delegates to the shared `store_oauth_tokens` for OAuth tokens, or stores
/// directly for manual/env-var tokens (no scopes or refresh token).
async fn save_token(
    store: &(dyn SecretsStore + Send + Sync),
    user_id: &str,
    auth: &crate::tools::wasm::AuthCapabilitySchema,
    token: &str,
    refresh_token: Option<&str>,
    expires_in: Option<u64>,
) -> anyhow::Result<()> {
    crate::cli::oauth_defaults::store_oauth_tokens(
        store,
        user_id,
        &auth.secret_name,
        auth.provider.as_deref(),
        token,
        refresh_token,
        expires_in,
        &[], // No scopes for manual/env-var tokens
    )
    .await
    .map_err(|e| anyhow::anyhow!("{}", e))
}

/// Print success message.
fn print_success(display_name: &str) {
    println!();
    println!("  ✓ {} connected!", display_name);
    println!();
    println!("  The tool can now access the API.");
    println!();
}

/// Configure required secrets for a tool via its `setup.required_secrets` schema.
async fn setup_tool(name: String, dir: Option<PathBuf>, user_id: String) -> anyhow::Result<()> {
    validate_tool_name(&name)?;
    let tools_dir = dir.unwrap_or_else(default_tools_dir);
    let caps_path = tools_dir.join(format!("{}.capabilities.json", name));

    if !caps_path.exists() {
        anyhow::bail!(
            "Tool '{}' not found or has no capabilities file at {}",
            name,
            caps_path.display()
        );
    }

    let content = fs::read_to_string(&caps_path).await?;
    let caps = CapabilitiesFile::from_json(&content)
        .map_err(|e| anyhow::anyhow!("Invalid capabilities file: {}", e))?;

    let setup = caps.setup.ok_or_else(|| {
        anyhow::anyhow!(
            "Tool '{}' has no setup configuration.\n\
             The tool may not require setup, or setup is not defined.\n\
             Try 'ironclaw tool auth {}' for OAuth-based authentication.",
            name,
            name
        )
    })?;

    if setup.required_secrets.is_empty() {
        println!("Tool '{}' has no required secrets.", name);
        return Ok(());
    }

    let display_name = caps
        .auth
        .as_ref()
        .and_then(|a| a.display_name.as_deref())
        .unwrap_or(&name);

    println!();
    println!("╔════════════════════════════════════════════════════════════════╗");
    println!("║  {:^62}║", format!("{} Setup", display_name));
    println!("╚════════════════════════════════════════════════════════════════╝");
    println!();

    let secrets_store = init_secrets_store().await?;

    let mut any_saved = false;

    for secret in &setup.required_secrets {
        let already_exists = secrets_store
            .exists(&user_id, &secret.name)
            .await
            .unwrap_or(false);

        if already_exists {
            println!("  ✓ {} (already configured)", secret.prompt);

            print!("    Replace? [y/N]: ");
            std::io::stdout().flush()?;

            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;

            if !input.trim().eq_ignore_ascii_case("y") {
                continue;
            }
            print!("  {}: ", secret.prompt);
        } else if secret.optional {
            print!("  {} (optional, Enter to skip): ", secret.prompt);
        } else {
            print!("  {}: ", secret.prompt);
        }

        std::io::stdout().flush()?;
        let value = read_hidden_input()?;
        println!();

        if value.is_empty() {
            if secret.optional {
                println!("    Skipped.");
            } else {
                println!(
                    "    Warning: empty value for required secret '{}'.",
                    secret.name
                );
            }
            continue;
        }

        let params = CreateSecretParams::new(&secret.name, &value).with_provider(name.to_string());
        secrets_store
            .create(&user_id, params)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to save secret: {}", e))?;

        println!("    ✓ Saved.");
        any_saved = true;
    }

    println!();
    if any_saved {
        println!("  ✓ {} setup complete!", display_name);
    } else {
        println!("  No changes made.");
    }
    println!();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{CreateSecretParams, SecretsStore};
    use crate::testing::credentials::test_secrets_store;

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

    /// Verify that auth secrets are deduplicated across auth, setup, and http.credentials,
    /// and that credential status is checked against the secrets store.
    #[tokio::test]
    async fn test_auth_secret_dedup_and_status() {
        let caps = CapabilitiesFile::from_json(
            r#"{
                "auth": {
                    "secret_name": "gh_token",
                    "display_name": "GitHub"
                },
                "setup": {
                    "required_secrets": [
                        { "name": "gh_token", "prompt": "GitHub PAT" },
                        { "name": "extra_key", "prompt": "Extra API Key" }
                    ]
                },
                "http": {
                    "allowlist": [{ "host": "api.github.com" }],
                    "credentials": {
                        "github": {
                            "secret_name": "gh_token",
                            "location": "AuthorizationBearer",
                            "host_patterns": ["api.github.com"]
                        }
                    }
                },
                "secrets": {
                    "allowed_names": ["gh_token", "gh_*"]
                }
            }"#,
        )
        .unwrap();

        let collected = collect_auth_secrets(&caps);

        // gh_token should appear once (from auth), with location merged from credentials.
        // extra_key should appear once (from setup).
        assert_eq!(collected.secrets.len(), 2);
        let gh = collected
            .secrets
            .iter()
            .find(|s| s.secret_name == "gh_token")
            .unwrap();
        assert_eq!(gh.description.as_deref(), Some("GitHub"));
        assert!(
            gh.location.is_some(),
            "location should be merged from http.credentials"
        );

        let extra = collected
            .secrets
            .iter()
            .find(|s| s.secret_name == "extra_key")
            .unwrap();
        assert_eq!(extra.description.as_deref(), Some("Extra API Key"));
        assert!(extra.location.is_none());

        // Secrets section should filter gh_token (in seen_names) but keep gh_* (wildcard).
        let secrets = caps.secrets.as_ref().unwrap();
        let extra_secrets: Vec<_> = secrets
            .allowed_names
            .iter()
            .filter(|name| !collected.seen_names.contains(name.as_str()))
            .collect();
        assert_eq!(extra_secrets, vec!["gh_*"]);

        // Verify store check: missing secret -> exists returns false.
        let store = test_secrets_store();
        assert!(!store.exists("default", "gh_token").await.unwrap());

        // Store gh_token and verify it's found.
        store
            .create(
                "default",
                CreateSecretParams::new("gh_token", "ghp_test123"),
            )
            .await
            .unwrap();
        assert!(store.exists("default", "gh_token").await.unwrap());
        // extra_key still missing.
        assert!(!store.exists("default", "extra_key").await.unwrap());
    }

    /// No auth sections → collect_auth_secrets returns empty.
    #[test]
    fn test_collect_auth_secrets_empty_caps() {
        let caps = CapabilitiesFile::default();
        let collected = collect_auth_secrets(&caps);
        assert!(collected.secrets.is_empty());
        assert!(collected.seen_names.is_empty());
    }
}
