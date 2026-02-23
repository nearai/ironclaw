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
        let wasm_path =
            crate::registry::artifacts::build_wasm_component(&source_dir, crate_name, true)
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

    pub async fn install_with_source_fallback(
        &self,
        manifest: &ExtensionManifest,
        force: bool,
    ) -> Result<InstallOutcome, RegistryError> {
        let has_artifact = manifest
            .artifacts
            .get("wasm32-wasip2")
            .and_then(|a| a.url.as_ref())
            .is_some();

        if !has_artifact {
            return self.install_from_source(manifest, force).await;
        }

        match self.install_from_artifact(manifest, force).await {
            Ok(outcome) => Ok(outcome),
            Err(RegistryError::AlreadyInstalled { name, path }) => {
                Err(RegistryError::AlreadyInstalled { name, path })
            }
            Err(artifact_err) => {
                tracing::warn!(
                    extension = %manifest.name,
                    error = %artifact_err,
                    "Artifact install failed; falling back to build-from-source"
                );
                match self.install_from_source(manifest, force).await {
                    Ok(mut outcome) => {
                        outcome.warnings.push(format!(
                            "Artifact install failed ({}); installed via source fallback.",
                            artifact_err
                        ));
                        Ok(outcome)
                    }
                    Err(source_err) => Err(RegistryError::ManifestRead {
                        path: self.repo_root.join(&manifest.source.dir),
                        reason: format!(
                            "artifact install failed: {}; source fallback failed: {}",
                            artifact_err, source_err
                        ),
                    }),
                }
            }
        }
    }

    /// Download and install a pre-built artifact.
    ///
    /// Supports two formats:
    /// - **tar.gz bundle**: Contains `{name}.wasm` + `{name}.capabilities.json`
    /// - **bare .wasm file**: Just the WASM binary (capabilities fetched separately if available)
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
        let bytes = download_artifact(url).await?;

        // Verify SHA256 if provided, warn otherwise
        if let Some(expected_sha) = &artifact.sha256 {
            verify_sha256(&bytes, expected_sha, url)?;
        } else {
            println!(
                "WARNING: No SHA256 checksum for '{}'; download is not cryptographically verified.",
                manifest.name
            );
        }

        let target_caps = target_dir.join(format!("{}.capabilities.json", manifest.name));

        // Detect format and extract
        let has_capabilities = if is_gzip(&bytes) {
            // tar.gz bundle: extract {name}.wasm and {name}.capabilities.json
            let extracted =
                extract_tar_gz(&bytes, &manifest.name, &target_wasm, &target_caps, url)?;
            extracted.has_capabilities
        } else {
            // Bare WASM file
            fs::write(&target_wasm, &bytes)
                .await
                .map_err(RegistryError::Io)?;

            // Try to get capabilities from:
            // 1. Separate capabilities_url in the artifact
            // 2. Source tree (legacy, requires repo)
            if let Some(ref caps_url) = artifact.capabilities_url {
                const MAX_CAPS_SIZE: usize = 1024 * 1024; // 1 MB
                match download_artifact(caps_url).await {
                    Ok(caps_bytes) if caps_bytes.len() <= MAX_CAPS_SIZE => {
                        fs::write(&target_caps, &caps_bytes)
                            .await
                            .map_err(RegistryError::Io)?;
                        true
                    }
                    Ok(caps_bytes) => {
                        tracing::warn!(
                            "Capabilities file too large ({} bytes, max {}), skipping",
                            caps_bytes.len(),
                            MAX_CAPS_SIZE
                        );
                        false
                    }
                    Err(e) => {
                        tracing::warn!("Failed to download capabilities from {}: {}", caps_url, e);
                        false
                    }
                }
            } else {
                // Legacy fallback: try source tree
                let caps_source = self
                    .repo_root
                    .join(&manifest.source.dir)
                    .join(&manifest.source.capabilities);
                if caps_source.exists() {
                    fs::copy(&caps_source, &target_caps)
                        .await
                        .map_err(RegistryError::Io)?;
                    true
                } else {
                    false
                }
            }
        };

        println!("  Installed to {}", target_wasm.display());

        let mut warnings = Vec::new();
        if !has_capabilities {
            warnings.push(format!(
                "No capabilities file found for '{}'. Auth and hooks may not work.",
                manifest.name
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

/// Download an artifact from a URL.
async fn download_artifact(url: &str) -> Result<bytes::Bytes, RegistryError> {
    let response = reqwest::get(url)
        .await
        .map_err(|e| RegistryError::DownloadFailed {
            url: url.to_string(),
            reason: format!("request failed: {}", e),
        })?;

    let response = response
        .error_for_status()
        .map_err(|e| RegistryError::DownloadFailed {
            url: url.to_string(),
            reason: e.to_string(),
        })?;

    response
        .bytes()
        .await
        .map_err(|e| RegistryError::DownloadFailed {
            url: url.to_string(),
            reason: format!("failed to read body: {}", e),
        })
}

/// Verify SHA256 of downloaded bytes.
fn verify_sha256(bytes: &[u8], expected: &str, url: &str) -> Result<(), RegistryError> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = format!("{:x}", hasher.finalize());

    if actual != expected {
        return Err(RegistryError::DownloadFailed {
            url: url.to_string(),
            reason: format!("SHA256 mismatch: expected {}, got {}", expected, actual),
        });
    }
    Ok(())
}

/// Check if bytes start with gzip magic number (0x1f 0x8b).
fn is_gzip(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b
}

/// Result of extracting a tar.gz bundle.
struct ExtractResult {
    has_capabilities: bool,
}

/// Extract a tar.gz archive, looking for `{name}.wasm` and `{name}.capabilities.json`.
fn extract_tar_gz(
    bytes: &[u8],
    name: &str,
    target_wasm: &Path,
    target_caps: &Path,
    url: &str,
) -> Result<ExtractResult, RegistryError> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    use std::io::Read as _;

    let decoder = GzDecoder::new(bytes);
    let mut archive = Archive::new(decoder);
    // Defense-in-depth: do not preserve permissions or extended attributes
    archive.set_preserve_permissions(false);
    #[cfg(any(unix, target_os = "redox"))]
    archive.set_unpack_xattrs(false);

    // 100 MB cap on decompressed entry size to prevent decompression bombs
    const MAX_ENTRY_SIZE: u64 = 100 * 1024 * 1024;

    let wasm_filename = format!("{}.wasm", name);
    let caps_filename = format!("{}.capabilities.json", name);
    let mut found_wasm = false;
    let mut found_caps = false;

    let entries = archive
        .entries()
        .map_err(|e| RegistryError::DownloadFailed {
            url: url.to_string(),
            reason: format!("failed to read tar.gz entries: {}", e),
        })?;

    for entry in entries {
        let mut entry = entry.map_err(|e| RegistryError::DownloadFailed {
            url: url.to_string(),
            reason: format!("failed to read tar.gz entry: {}", e),
        })?;

        if entry.size() > MAX_ENTRY_SIZE {
            return Err(RegistryError::DownloadFailed {
                url: url.to_string(),
                reason: format!(
                    "archive entry too large ({} bytes, max {} bytes)",
                    entry.size(),
                    MAX_ENTRY_SIZE
                ),
            });
        }

        let entry_path = entry
            .path()
            .map_err(|e| RegistryError::DownloadFailed {
                url: url.to_string(),
                reason: format!("invalid path in tar.gz: {}", e),
            })?
            .to_path_buf();

        // Match by filename (ignoring any directory prefix in the archive)
        let filename = entry_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        if filename == wasm_filename {
            let mut data = Vec::with_capacity(entry.size() as usize);
            std::io::Read::read_to_end(&mut entry.by_ref().take(MAX_ENTRY_SIZE), &mut data)
                .map_err(|e| RegistryError::DownloadFailed {
                    url: url.to_string(),
                    reason: format!("failed to read {} from archive: {}", wasm_filename, e),
                })?;
            std::fs::write(target_wasm, &data).map_err(RegistryError::Io)?;
            found_wasm = true;
        } else if filename == caps_filename {
            let mut data = Vec::with_capacity(entry.size() as usize);
            std::io::Read::read_to_end(&mut entry.by_ref().take(MAX_ENTRY_SIZE), &mut data)
                .map_err(|e| RegistryError::DownloadFailed {
                    url: url.to_string(),
                    reason: format!("failed to read {} from archive: {}", caps_filename, e),
                })?;
            std::fs::write(target_caps, &data).map_err(RegistryError::Io)?;
            found_caps = true;
        }
    }

    if !found_wasm {
        return Err(RegistryError::DownloadFailed {
            url: url.to_string(),
            reason: format!(
                "tar.gz archive does not contain '{}'. Archive may be malformed.",
                wasm_filename
            ),
        });
    }

    Ok(ExtractResult {
        has_capabilities: found_caps,
    })
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

    #[test]
    fn test_is_gzip() {
        assert!(is_gzip(&[0x1f, 0x8b, 0x08]));
        assert!(!is_gzip(&[0x00, 0x61, 0x73, 0x6d])); // WASM magic
        assert!(!is_gzip(&[0x1f])); // Too short
        assert!(!is_gzip(&[]));
    }

    #[test]
    fn test_verify_sha256_valid() {
        use sha2::{Digest, Sha256};
        let data = b"hello world";
        let mut hasher = Sha256::new();
        hasher.update(data);
        let hash = format!("{:x}", hasher.finalize());
        assert!(verify_sha256(data, &hash, "test://url").is_ok());
    }

    #[test]
    fn test_verify_sha256_invalid() {
        assert!(verify_sha256(b"data", "0000", "test://url").is_err());
    }

    #[test]
    fn test_extract_tar_gz() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use tar::Builder;

        // Create a tar.gz in memory with test.wasm and test.capabilities.json
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = Builder::new(&mut encoder);

            let wasm_data = b"\0asm\x01\x00\x00\x00";
            let mut header = tar::Header::new_gnu();
            header.set_size(wasm_data.len() as u64);
            header.set_cksum();
            builder
                .append_data(&mut header, "test.wasm", &wasm_data[..])
                .unwrap();

            let caps_data = br#"{"auth":null}"#;
            let mut header = tar::Header::new_gnu();
            header.set_size(caps_data.len() as u64);
            header.set_cksum();
            builder
                .append_data(&mut header, "test.capabilities.json", &caps_data[..])
                .unwrap();

            builder.finish().unwrap();
        }
        let gz_bytes = encoder.finish().unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let wasm_path = tmp.path().join("test.wasm");
        let caps_path = tmp.path().join("test.capabilities.json");

        let result =
            extract_tar_gz(&gz_bytes, "test", &wasm_path, &caps_path, "test://url").unwrap();

        assert!(wasm_path.exists());
        assert!(caps_path.exists());
        assert!(result.has_capabilities);
    }

    #[test]
    fn test_extract_tar_gz_missing_wasm() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use tar::Builder;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = Builder::new(&mut encoder);

            let data = b"not a wasm file";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_cksum();
            builder
                .append_data(&mut header, "wrong.wasm", &data[..])
                .unwrap();
            builder.finish().unwrap();
        }
        let gz_bytes = encoder.finish().unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let result = extract_tar_gz(
            &gz_bytes,
            "test",
            &tmp.path().join("test.wasm"),
            &tmp.path().join("test.capabilities.json"),
            "test://url",
        );

        assert!(result.is_err());
    }
}
