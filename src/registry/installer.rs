//! Install extensions from the registry: build-from-source or download pre-built artifacts.

use std::path::{Path, PathBuf};

use tokio::fs;

use crate::registry::catalog::RegistryError;
use crate::registry::manifest::{BundleDefinition, ExtensionManifest, ManifestKind};

/// Result of installing a single extension from the registry.
#[derive(Debug)]
pub struct InstallOutcome {
    /// Extension name.
    pub name: String,
    /// Whether this is a tool or channel.
    pub kind: ManifestKind,
    /// Destination path of the installed WASM binary.
    pub wasm_path: PathBuf,
    /// Whether a capabilities file was also installed.
    pub has_capabilities: bool,
    /// Any warning messages.
    pub warnings: Vec<String>,
}

/// Handles installing extensions from registry manifests.
pub struct RegistryInstaller {
    /// Root of the repo (parent of `registry/`), used to resolve `source.dir`.
    repo_root: PathBuf,
    /// Directory for installed tools (`~/.ironclaw/tools/`).
    tools_dir: PathBuf,
    /// Directory for installed channels (`~/.ironclaw/channels/`).
    channels_dir: PathBuf,
}

impl RegistryInstaller {
    pub fn new(repo_root: PathBuf, tools_dir: PathBuf, channels_dir: PathBuf) -> Self {
        Self {
            repo_root,
            tools_dir,
            channels_dir,
        }
    }

    /// Default installer using standard paths.
    pub fn with_defaults(repo_root: PathBuf) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            repo_root,
            tools_dir: home.join(".ironclaw").join("tools"),
            channels_dir: home.join(".ironclaw").join("channels"),
        }
    }

    /// Install a single extension by building from source.
    pub async fn install_from_source(
        &self,
        manifest: &ExtensionManifest,
        force: bool,
    ) -> Result<InstallOutcome, RegistryError> {
        let source_dir = self.repo_root.join(&manifest.source.dir);
        if !source_dir.exists() {
            return Err(RegistryError::ManifestRead {
                path: source_dir.clone(),
                reason: "source directory does not exist".to_string(),
            });
        }

        let target_dir = match manifest.kind {
            ManifestKind::Tool => &self.tools_dir,
            ManifestKind::Channel => &self.channels_dir,
        };

        fs::create_dir_all(target_dir)
            .await
            .map_err(RegistryError::Io)?;

        // Use manifest.name for installed filenames so discovery, auth, and
        // CLI commands (`ironclaw tool auth <name>`) all agree on the stem.
        let target_wasm = target_dir.join(format!("{}.wasm", manifest.name));

        // Check if already exists
        if target_wasm.exists() && !force {
            return Err(RegistryError::AlreadyInstalled {
                name: manifest.name.clone(),
                path: target_wasm,
            });
        }

        // Build the WASM component
        println!(
            "Building {} '{}' from {}...",
            manifest.kind,
            manifest.display_name,
            source_dir.display()
        );
        let crate_name = &manifest.source.crate_name;
        let wasm_path = build_wasm_component(&source_dir, crate_name)
            .await
            .map_err(|e| RegistryError::ManifestRead {
                path: source_dir.clone(),
                reason: format!("build failed: {}", e),
            })?;

        // Copy WASM binary
        println!("  Installing to {}", target_wasm.display());
        fs::copy(&wasm_path, &target_wasm)
            .await
            .map_err(RegistryError::Io)?;

        // Copy capabilities file
        let caps_source = source_dir.join(&manifest.source.capabilities);
        let target_caps = target_dir.join(format!("{}.capabilities.json", manifest.name));
        let has_capabilities = if caps_source.exists() {
            fs::copy(&caps_source, &target_caps)
                .await
                .map_err(RegistryError::Io)?;
            true
        } else {
            false
        };

        let mut warnings = Vec::new();
        if !has_capabilities {
            warnings.push(format!(
                "No capabilities file found at {}",
                caps_source.display()
            ));
        }

        Ok(InstallOutcome {
            name: manifest.name.clone(),
            kind: manifest.kind,
            wasm_path: target_wasm,
            has_capabilities,
            warnings,
        })
    }

    /// Download and install a pre-built artifact.
    pub async fn install_from_artifact(
        &self,
        manifest: &ExtensionManifest,
        force: bool,
    ) -> Result<InstallOutcome, RegistryError> {
        let artifact = manifest.artifacts.get("wasm32-wasip2").ok_or_else(|| {
            RegistryError::ExtensionNotFound(format!(
                "No wasm32-wasip2 artifact for '{}'",
                manifest.name
            ))
        })?;

        let url = artifact.url.as_ref().ok_or_else(|| {
            RegistryError::ExtensionNotFound(format!(
                "No artifact URL for '{}'. Use --build to build from source.",
                manifest.name
            ))
        })?;

        let expected_sha = artifact.sha256.as_ref().ok_or_else(|| {
            RegistryError::ExtensionNotFound(format!(
                "No SHA256 hash for '{}'. Cannot verify download.",
                manifest.name
            ))
        })?;

        let target_dir = match manifest.kind {
            ManifestKind::Tool => &self.tools_dir,
            ManifestKind::Channel => &self.channels_dir,
        };

        fs::create_dir_all(target_dir)
            .await
            .map_err(RegistryError::Io)?;

        let target_wasm = target_dir.join(format!("{}.wasm", manifest.name));

        if target_wasm.exists() && !force {
            return Err(RegistryError::AlreadyInstalled {
                name: manifest.name.clone(),
                path: target_wasm,
            });
        }

        // Download
        println!(
            "Downloading {} '{}'...",
            manifest.kind, manifest.display_name
        );
        let response = reqwest::get(url)
            .await
            .map_err(|e| RegistryError::DownloadFailed {
                url: url.clone(),
                reason: format!("request failed: {}", e),
            })?;

        let response = response
            .error_for_status()
            .map_err(|e| RegistryError::DownloadFailed {
                url: url.clone(),
                reason: e.to_string(),
            })?;

        let bytes = response
            .bytes()
            .await
            .map_err(|e| RegistryError::DownloadFailed {
                url: url.clone(),
                reason: format!("failed to read body: {}", e),
            })?;

        // Verify SHA256
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let actual_sha = format!("{:x}", hasher.finalize());

        if actual_sha != *expected_sha {
            return Err(RegistryError::DownloadFailed {
                url: url.clone(),
                reason: format!(
                    "SHA256 mismatch: expected {}, got {}",
                    expected_sha, actual_sha
                ),
            });
        }

        // Write file
        fs::write(&target_wasm, &bytes)
            .await
            .map_err(RegistryError::Io)?;

        // Copy capabilities from source dir (still needed even for pre-built artifacts).
        // NOTE: This requires the source tree to be present. When pre-built artifact
        // distribution is implemented, capabilities should be bundled with the artifact
        // or fetched from a separate URL.
        let caps_source = self
            .repo_root
            .join(&manifest.source.dir)
            .join(&manifest.source.capabilities);
        let target_caps = target_dir.join(format!("{}.capabilities.json", manifest.name));
        let has_capabilities = if caps_source.exists() {
            fs::copy(&caps_source, &target_caps)
                .await
                .map_err(RegistryError::Io)?;
            true
        } else {
            false
        };

        println!("  Installed to {}", target_wasm.display());

        Ok(InstallOutcome {
            name: manifest.name.clone(),
            kind: manifest.kind,
            wasm_path: target_wasm,
            has_capabilities,
            warnings: Vec::new(),
        })
    }

    /// Install a single manifest, choosing build vs download based on artifact availability and flags.
    pub async fn install(
        &self,
        manifest: &ExtensionManifest,
        force: bool,
        prefer_build: bool,
    ) -> Result<InstallOutcome, RegistryError> {
        let has_artifact = manifest
            .artifacts
            .get("wasm32-wasip2")
            .and_then(|a| a.url.as_ref())
            .is_some();

        if prefer_build || !has_artifact {
            self.install_from_source(manifest, force).await
        } else {
            self.install_from_artifact(manifest, force).await
        }
    }

    /// Install all extensions in a bundle.
    /// Returns the outcomes and any shared auth hints.
    pub async fn install_bundle(
        &self,
        manifests: &[&ExtensionManifest],
        bundle: &BundleDefinition,
        force: bool,
        prefer_build: bool,
    ) -> (Vec<InstallOutcome>, Vec<String>) {
        let mut outcomes = Vec::new();
        let mut errors = Vec::new();

        for manifest in manifests {
            match self.install(manifest, force, prefer_build).await {
                Ok(outcome) => outcomes.push(outcome),
                Err(e) => errors.push(format!("{}: {}", manifest.name, e)),
            }
        }

        // Collect auth hints
        let mut auth_hints = Vec::new();
        if let Some(shared) = &bundle.shared_auth {
            auth_hints.push(format!(
                "Bundle uses shared auth '{}'. Run `ironclaw tool auth <any-member>` to authenticate all members.",
                shared
            ));
        }

        // Collect unique auth providers that need setup
        let mut seen_providers = std::collections::HashSet::new();
        for manifest in manifests {
            if let Some(auth) = &manifest.auth_summary {
                let key = auth
                    .shared_auth
                    .as_deref()
                    .unwrap_or(manifest.name.as_str());
                if seen_providers.insert(key.to_string())
                    && let Some(url) = &auth.setup_url
                {
                    auth_hints.push(format!(
                        "  {} ({}): {}",
                        auth.provider.as_deref().unwrap_or(&manifest.name),
                        auth.method.as_deref().unwrap_or("manual"),
                        url
                    ));
                }
            }
        }

        if !errors.is_empty() {
            auth_hints.push(format!(
                "\nFailed to install {} extension(s):",
                errors.len()
            ));
            for err in errors {
                auth_hints.push(format!("  - {}", err));
            }
        }

        (outcomes, auth_hints)
    }
}

/// Build a WASM component from a source directory using `cargo component build --release`.
///
/// Uses `tokio::process::Command` with inherited stdio so build progress is visible.
/// Looks for the specific `{crate_name}.wasm` in the release directory rather than
/// picking the first `.wasm` file found.
async fn build_wasm_component(source_dir: &Path, crate_name: &str) -> anyhow::Result<PathBuf> {
    use tokio::process::Command;

    // Check cargo-component availability
    let check = Command::new("cargo")
        .args(["component", "--version"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;

    if check.is_err() || !check.as_ref().map(|s| s.success()).unwrap_or(false) {
        anyhow::bail!("cargo-component not found. Install with: cargo install cargo-component");
    }

    // Use status() with inherited stdio so build output streams to the terminal.
    let status = Command::new("cargo")
        .current_dir(source_dir)
        .args(["component", "build", "--release"])
        .status()
        .await?;

    if !status.success() {
        anyhow::bail!("Build failed (exit code: {})", status);
    }

    // Look for the specific crate's WASM file (Cargo uses underscores in artifact names).
    let wasm_filename = format!("{}.wasm", crate_name.replace('-', "_"));
    let target_base = source_dir.join("target");
    let candidates = [
        "wasm32-wasip1",
        "wasm32-wasip2",
        "wasm32-wasi",
        "wasm32-unknown-unknown",
    ];

    for target in &candidates {
        let wasm_path = target_base
            .join(target)
            .join("release")
            .join(&wasm_filename);
        if wasm_path.exists() {
            return Ok(wasm_path);
        }
    }

    anyhow::bail!(
        "Could not find {} in {}/target/*/release/",
        wasm_filename,
        source_dir.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_installer_creation() {
        let installer = RegistryInstaller::new(
            PathBuf::from("/repo"),
            PathBuf::from("/home/.ironclaw/tools"),
            PathBuf::from("/home/.ironclaw/channels"),
        );
        assert_eq!(installer.repo_root, PathBuf::from("/repo"));
    }
}
