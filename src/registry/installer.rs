//! Install extensions from the registry: build-from-source or download pre-built artifacts.

use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};

use tokio::fs;

use crate::bootstrap::ironclaw_base_dir;
use crate::registry::catalog::RegistryError;
use crate::registry::manifest::{BundleDefinition, ExtensionManifest, ManifestKind};

// GitHub-only by design. New trusted hosts (e.g. a NEAR AI CDN) must be
// explicitly added here; unknown hosts fall back to source build with a
// warning rather than surfacing a clear "host not allowed" error.
const ALLOWED_ARTIFACT_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "github-releases.githubusercontent.com",
    "raw.githubusercontent.com",
];

fn should_attempt_source_fallback(err: &RegistryError) -> bool {
    !matches!(
        err,
        RegistryError::AlreadyInstalled { .. }
            | RegistryError::ChecksumMismatch { .. }
            | RegistryError::InvalidManifest { .. }
    )
}

fn is_allowed_artifact_host(host: &str) -> bool {
    ALLOWED_ARTIFACT_HOSTS
        .iter()
        .any(|allowed| host.eq_ignore_ascii_case(allowed))
        || host.ends_with(".githubusercontent.com")
}

fn validate_artifact_url(
    manifest_name: &str,
    field: &'static str,
    url: &str,
) -> Result<(), RegistryError> {
    let parsed = reqwest::Url::parse(url).map_err(|e| RegistryError::InvalidManifest {
        name: manifest_name.to_string(),
        field,
        reason: format!("invalid URL: {}", e),
    })?;

    if parsed.scheme() != "https" {
        return Err(RegistryError::InvalidManifest {
            name: manifest_name.to_string(),
            field,
            reason: "URL must use https".to_string(),
        });
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| RegistryError::InvalidManifest {
            name: manifest_name.to_string(),
            field,
            reason: "URL host is missing".to_string(),
        })?;

    if host.parse::<IpAddr>().is_ok() || !is_allowed_artifact_host(host) {
        return Err(RegistryError::InvalidManifest {
            name: manifest_name.to_string(),
            field,
            reason: format!("host '{}' is not allowed", host),
        });
    }

    Ok(())
}

fn validate_manifest_install_inputs(manifest: &ExtensionManifest) -> Result<(), RegistryError> {
    let is_valid_name = !manifest.name.is_empty()
        && manifest
            .name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');

    if !is_valid_name {
        return Err(RegistryError::InvalidManifest {
            name: manifest.name.clone(),
            field: "name",
            reason: "name must contain only lowercase letters, digits, '-' or '_'".to_string(),
        });
    }

    let expected_prefix = match manifest.kind {
        ManifestKind::Tool => "tools-src/",
        ManifestKind::Channel => "channels-src/",
    };

    if !manifest.source.dir.starts_with(expected_prefix) {
        return Err(RegistryError::InvalidManifest {
            name: manifest.name.clone(),
            field: "source.dir",
            reason: format!("must start with '{}'", expected_prefix),
        });
    }

    let source_path = Path::new(&manifest.source.dir);
    let has_unsafe_component = source_path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) | Component::CurDir
        )
    });

    if source_path.is_absolute() || has_unsafe_component {
        return Err(RegistryError::InvalidManifest {
            name: manifest.name.clone(),
            field: "source.dir",
            reason: "must be a safe relative path without traversal segments".to_string(),
        });
    }

    let has_path_separator = manifest.source.capabilities.contains('/')
        || manifest.source.capabilities.contains('\\')
        || manifest.source.capabilities.contains("..");

    if has_path_separator {
        return Err(RegistryError::InvalidManifest {
            name: manifest.name.clone(),
            field: "source.capabilities",
            reason: "must be a file name without path separators".to_string(),
        });
    }

    Ok(())
}

fn download_failure_reason(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "request timed out".to_string()
    } else if error.is_connect() {
        "connection failed".to_string()
    } else if error.is_request() {
        "request failed".to_string()
    } else {
        "network error".to_string()
    }
}

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
        let base_dir = ironclaw_base_dir();
        Self {
            repo_root,
            tools_dir: base_dir.join("tools"),
            channels_dir: base_dir.join("channels"),
        }
    }

    /// Install a single extension by building from source.
    pub async fn install_from_source(
        &self,
        manifest: &ExtensionManifest,
        force: bool,
    ) -> Result<InstallOutcome, RegistryError> {
        validate_manifest_install_inputs(manifest)?;

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
        // Validate upfront so we fail fast on bad manifests regardless of
        // which install path runs, without relying on inner methods to
        // catch it first.
        validate_manifest_install_inputs(manifest)?;

        let has_artifact = manifest
            .artifacts
            .get("wasm32-wasip2")
            .and_then(|a| a.url.as_ref())
            .is_some();

        if !has_artifact {
            return self.install_from_source(manifest, force).await;
        }

        let source_dir = self.repo_root.join(&manifest.source.dir);

        match self.install_from_artifact(manifest, force).await {
            Ok(outcome) => Ok(outcome),
            Err(artifact_err) => {
                if !should_attempt_source_fallback(&artifact_err) {
                    return Err(artifact_err);
                }

                if !source_dir.is_dir() {
                    return Err(RegistryError::SourceFallbackUnavailable {
                        name: manifest.name.clone(),
                        source_dir,
                        artifact_error: Box::new(artifact_err),
                    });
                }

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
                    Err(source_err) => Err(RegistryError::InstallFallbackFailed {
                        name: manifest.name.clone(),
                        artifact_error: Box::new(artifact_err),
                        source_error: Box::new(source_err),
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
        validate_manifest_install_inputs(manifest)?;

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

        validate_artifact_url(&manifest.name, "artifacts.wasm32-wasip2.url", url)?;

        // Require SHA256 — refuse to install unverified binaries. Check before
        // downloading to avoid wasting bandwidth on manifests that are missing
        // checksums.
        let expected_sha =
            artifact
                .sha256
                .as_ref()
                .ok_or_else(|| RegistryError::InvalidManifest {
                    name: manifest.name.clone(),
                    field: "artifacts.wasm32-wasip2.sha256",
                    reason: "sha256 is required for artifact downloads".to_string(),
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
        verify_sha256(&bytes, expected_sha, url)?;

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
                validate_artifact_url(
                    &manifest.name,
                    "artifacts.wasm32-wasip2.capabilities_url",
                    caps_url,
                )?;
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
            reason: download_failure_reason(&e),
        })?;

    let response = response
        .error_for_status()
        .map_err(|e| RegistryError::DownloadFailed {
            url: url.to_string(),
            reason: format!(
                "http status {}",
                e.status()
                    .map_or("unknown".to_string(), |status| status.as_u16().to_string())
            ),
        })?;

    response
        .bytes()
        .await
        .map_err(|e| RegistryError::DownloadFailed {
            url: url.to_string(),
            reason: format!("failed to read response body: {}", e),
        })
}

/// Verify SHA256 of downloaded bytes.
fn verify_sha256(bytes: &[u8], expected: &str, url: &str) -> Result<(), RegistryError> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = format!("{:x}", hasher.finalize());

    if actual != expected {
        return Err(RegistryError::ChecksumMismatch {
            url: url.to_string(),
            expected_sha256: expected.to_string(),
            actual_sha256: actual,
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
    use std::collections::HashMap;

    use crate::registry::manifest::{ArtifactSpec, SourceSpec};

    fn test_manifest(
        name: &str,
        source_dir: &str,
        artifact_url: Option<String>,
        sha256: Option<&str>,
    ) -> ExtensionManifest {
        test_manifest_with_kind(name, source_dir, artifact_url, sha256, ManifestKind::Tool)
    }

    fn test_manifest_with_kind(
        name: &str,
        source_dir: &str,
        artifact_url: Option<String>,
        sha256: Option<&str>,
        kind: ManifestKind,
    ) -> ExtensionManifest {
        let mut artifacts = HashMap::new();
        if artifact_url.is_some() || sha256.is_some() {
            artifacts.insert(
                "wasm32-wasip2".to_string(),
                ArtifactSpec {
                    url: artifact_url,
                    sha256: sha256.map(ToString::to_string),
                    capabilities_url: None,
                },
            );
        }

        ExtensionManifest {
            name: name.to_string(),
            display_name: name.to_string(),
            kind,
            version: "0.1.0".to_string(),
            description: "test manifest".to_string(),
            keywords: Vec::new(),
            source: SourceSpec {
                dir: source_dir.to_string(),
                capabilities: format!("{}.capabilities.json", name),
                crate_name: name.to_string(),
            },
            artifacts,
            auth_summary: None,
            tags: Vec::new(),
        }
    }

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
        let err = verify_sha256(b"data", "0000", "test://url").expect_err("checksum mismatch");
        assert!(matches!(err, RegistryError::ChecksumMismatch { .. }));
    }

    #[tokio::test]
    async fn test_install_from_source_rejects_path_traversal_name() {
        let temp = tempfile::tempdir().expect("tempdir");
        let installer = RegistryInstaller::new(
            temp.path().to_path_buf(),
            temp.path().join("tools"),
            temp.path().join("channels"),
        );

        let manifest = test_manifest("../evil", "tools-src/evil", None, None);

        let result = installer.install_from_source(&manifest, false).await;
        match result {
            Err(RegistryError::InvalidManifest { field, .. }) => {
                assert_eq!(field, "name");
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_install_from_artifact_rejects_non_https_url() {
        let temp = tempfile::tempdir().expect("tempdir");
        let installer = RegistryInstaller::new(
            temp.path().to_path_buf(),
            temp.path().join("tools"),
            temp.path().join("channels"),
        );

        let manifest = test_manifest(
            "demo",
            "tools-src/demo",
            Some(
                "http://github.com/nearai/ironclaw/releases/latest/download/demo.wasm".to_string(),
            ),
            None,
        );

        let result = installer.install_from_artifact(&manifest, false).await;
        match result {
            Err(RegistryError::InvalidManifest { field, .. }) => {
                assert_eq!(field, "artifacts.wasm32-wasip2.url");
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_install_from_artifact_rejects_disallowed_host() {
        let temp = tempfile::tempdir().expect("tempdir");
        let installer = RegistryInstaller::new(
            temp.path().to_path_buf(),
            temp.path().join("tools"),
            temp.path().join("channels"),
        );

        let manifest = test_manifest(
            "demo",
            "tools-src/demo",
            Some("https://169.254.169.254/latest/meta-data".to_string()),
            None,
        );

        let result = installer.install_from_artifact(&manifest, false).await;
        match result {
            Err(RegistryError::InvalidManifest { field, .. }) => {
                assert_eq!(field, "artifacts.wasm32-wasip2.url");
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_install_from_artifact_rejects_null_sha256() {
        let temp = tempfile::tempdir().expect("tempdir");
        let installer = RegistryInstaller::new(
            temp.path().to_path_buf(),
            temp.path().join("tools"),
            temp.path().join("channels"),
        );

        // Valid URL but no sha256 — should be rejected before any download attempt
        let manifest = test_manifest(
            "demo",
            "tools-src/demo",
            Some(
                "https://github.com/nearai/ironclaw/releases/latest/download/demo-wasm32-wasip2.tar.gz".to_string(),
            ),
            None, // sha256 = null
        );

        let result = installer.install_from_artifact(&manifest, false).await;
        match result {
            Err(RegistryError::InvalidManifest { field, reason, .. }) => {
                assert_eq!(field, "artifacts.wasm32-wasip2.sha256");
                assert!(reason.contains("required"), "reason: {}", reason);
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[test]
    fn test_should_attempt_source_fallback_policy() {
        let download = RegistryError::DownloadFailed {
            url: "https://github.com/nearai/ironclaw/releases/latest/download/demo.wasm"
                .to_string(),
            reason: "http status 404".to_string(),
        };
        assert!(should_attempt_source_fallback(&download));

        let already = RegistryError::AlreadyInstalled {
            name: "demo".to_string(),
            path: PathBuf::from("/tmp/demo.wasm"),
        };
        assert!(!should_attempt_source_fallback(&already));

        let checksum = RegistryError::ChecksumMismatch {
            url: "https://github.com/nearai/ironclaw/releases/latest/download/demo.wasm"
                .to_string(),
            expected_sha256: "deadbeef".to_string(),
            actual_sha256: "feedface".to_string(),
        };
        assert!(!should_attempt_source_fallback(&checksum));

        let invalid = RegistryError::InvalidManifest {
            name: "demo".to_string(),
            field: "artifacts.wasm32-wasip2.url",
            reason: "host not allowed".to_string(),
        };
        assert!(!should_attempt_source_fallback(&invalid));
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

    #[tokio::test]
    async fn test_install_from_source_rejects_wrong_prefix_for_channel() {
        let temp = tempfile::tempdir().expect("tempdir");
        let installer = RegistryInstaller::new(
            temp.path().to_path_buf(),
            temp.path().join("tools"),
            temp.path().join("channels"),
        );

        // Channel manifest with tools-src/ prefix should be rejected
        let manifest = test_manifest_with_kind(
            "telegram",
            "tools-src/telegram",
            None,
            None,
            ManifestKind::Channel,
        );

        let result = installer.install_from_source(&manifest, false).await;
        match result {
            Err(RegistryError::InvalidManifest { field, reason, .. }) => {
                assert_eq!(field, "source.dir");
                assert!(reason.contains("channels-src/"), "reason: {}", reason);
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_install_from_source_accepts_correct_channel_prefix() {
        let temp = tempfile::tempdir().expect("tempdir");
        let installer = RegistryInstaller::new(
            temp.path().to_path_buf(),
            temp.path().join("tools"),
            temp.path().join("channels"),
        );

        // Channel manifest with channels-src/ prefix should pass validation
        // (will fail later because source dir doesn't exist, which is fine)
        let manifest = test_manifest_with_kind(
            "telegram",
            "channels-src/telegram",
            None,
            None,
            ManifestKind::Channel,
        );

        let result = installer.install_from_source(&manifest, false).await;
        match result {
            Err(RegistryError::ManifestRead { reason, .. }) => {
                assert!(
                    reason.contains("source directory does not exist"),
                    "reason: {}",
                    reason
                );
            }
            other => panic!("unexpected result: {:?}", other),
        }
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
