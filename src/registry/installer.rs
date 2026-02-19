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
            .map_err(|e| RegistryError::Io(e))?;

        let target_wasm = target_dir.join(format!("{}.wasm", manifest.source.crate_name));

        // Check if already exists
        if target_wasm.exists() && !force {
            return Err(RegistryError::ExtensionNotFound(format!(
                "'{}' already installed at {}. Use --force to overwrite.",
                manifest.name,
                target_wasm.display()
            )));
        }

        // Build the WASM component
        println!(
            "Building {} '{}' from {}...",
            manifest.kind,
            manifest.display_name,
            source_dir.display()
        );
        let wasm_path =
            build_wasm_component(&source_dir).map_err(|e| RegistryError::ManifestRead {
                path: source_dir.clone(),
                reason: format!("build failed: {}", e),
            })?;

        // Copy WASM binary
        println!("  Installing to {}", target_wasm.display());
        fs::copy(&wasm_path, &target_wasm)
            .await
            .map_err(|e| RegistryError::Io(e))?;

        // Copy capabilities file
        let caps_source = source_dir.join(&manifest.source.capabilities);
        let target_caps =
            target_dir.join(format!("{}.capabilities.json", manifest.source.crate_name));
        let has_capabilities = if caps_source.exists() {
            fs::copy(&caps_source, &target_caps)
                .await
                .map_err(|e| RegistryError::Io(e))?;
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
            .map_err(|e| RegistryError::Io(e))?;

        let target_wasm = target_dir.join(format!("{}.wasm", manifest.source.crate_name));

        if target_wasm.exists() && !force {
            return Err(RegistryError::ExtensionNotFound(format!(
                "'{}' already installed. Use --force to overwrite.",
                manifest.name
            )));
        }

        // Download
        println!(
            "Downloading {} '{}'...",
            manifest.kind, manifest.display_name
        );
        let response = reqwest::get(url)
            .await
            .map_err(|e| RegistryError::ManifestRead {
                path: PathBuf::from(url.as_str()),
                reason: format!("download failed: {}", e),
            })?;

        let bytes = response
            .bytes()
            .await
            .map_err(|e| RegistryError::ManifestRead {
                path: PathBuf::from(url.as_str()),
                reason: format!("download failed: {}", e),
            })?;

        // Verify SHA256
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let actual_sha = format!("{:x}", hasher.finalize());

        if actual_sha != *expected_sha {
            return Err(RegistryError::ManifestRead {
                path: PathBuf::from(url.as_str()),
                reason: format!(
                    "SHA256 mismatch: expected {}, got {}",
                    expected_sha, actual_sha
                ),
            });
        }

        // Write file
        fs::write(&target_wasm, &bytes)
            .await
            .map_err(|e| RegistryError::Io(e))?;

        // Copy capabilities from source dir (still needed even for pre-built artifacts)
        let caps_source = self
            .repo_root
            .join(&manifest.source.dir)
            .join(&manifest.source.capabilities);
        let target_caps =
            target_dir.join(format!("{}.capabilities.json", manifest.source.crate_name));
        let has_capabilities = if caps_source.exists() {
            fs::copy(&caps_source, &target_caps)
                .await
                .map_err(|e| RegistryError::Io(e))?;
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
                if seen_providers.insert(key.to_string()) {
                    if let Some(url) = &auth.setup_url {
                        auth_hints.push(format!(
                            "  {} ({}): {}",
                            auth.provider.as_deref().unwrap_or(&manifest.name),
                            auth.method.as_deref().unwrap_or("manual"),
                            url
                        ));
                    }
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
fn build_wasm_component(source_dir: &Path) -> anyhow::Result<PathBuf> {
    use std::process::Command;

    // Check cargo-component availability
    let check = Command::new("cargo")
        .args(["component", "--version"])
        .output();

    if check.is_err() || !check.as_ref().map(|o| o.status.success()).unwrap_or(false) {
        anyhow::bail!("cargo-component not found. Install with: cargo install cargo-component");
    }

    let output = Command::new("cargo")
        .current_dir(source_dir)
        .args(["component", "build", "--release"])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Build failed:\n{}", stderr);
    }

    // Find the output wasm file
    let target_base = source_dir.join("target");
    let candidates = [
        "wasm32-wasip1",
        "wasm32-wasip2",
        "wasm32-wasi",
        "wasm32-unknown-unknown",
    ];

    for target in &candidates {
        let release_dir = target_base.join(target).join("release");
        if release_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&release_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("wasm") {
                        return Ok(path);
                    }
                }
            }
        }
    }

    anyhow::bail!(
        "Could not find built WASM file in {}/target/*/release/",
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
