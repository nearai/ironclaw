use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use tokio::fs;
use tokio::sync::Mutex as AsyncMutex;

use crate::bootstrap::ironclaw_base_dir;
use crate::registry::catalog::RegistryError;
use crate::registry::hub_manifest::{
    DEFAULT_HUB_MANIFEST_URL, HubManifest, HubSkillEntry, HubToolEntry, Provenance,
};
use crate::registry::installer::{download_artifact, validate_artifact_url, verify_sha256};

const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_METADATA_BYTES: usize = 1024 * 1024;
const MAX_WASM_BYTES: usize = 16 * 1024 * 1024;

static INSTALL_LOCKS: LazyLock<std::sync::Mutex<HashMap<String, Arc<AsyncMutex<()>>>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

struct InstallLock {
    key: String,
    mutex: Arc<AsyncMutex<()>>,
}

impl InstallLock {
    async fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.mutex.lock().await
    }
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        let mut guard = INSTALL_LOCKS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(existing) = guard.get(&self.key)
            && Arc::ptr_eq(existing, &self.mutex)
            && Arc::strong_count(&self.mutex) <= 2
        {
            guard.remove(&self.key);
        }
    }
}

fn acquire_install_lock(key: &str) -> InstallLock {
    let mut guard = INSTALL_LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mutex = if let Some(existing) = guard.get(key) {
        Arc::clone(existing)
    } else {
        let fresh = Arc::new(AsyncMutex::new(()));
        guard.insert(key.to_string(), Arc::clone(&fresh));
        fresh
    };
    InstallLock {
        key: key.to_string(),
        mutex,
    }
}

const MANIFEST_CACHE_TTL: Duration = Duration::from_secs(60);
const MANIFEST_CACHE_MAX_ENTRIES: usize = 64;

struct CachedManifest {
    manifest: Arc<HubManifest>,
    fetched_at: Instant,
}

static MANIFEST_CACHE: LazyLock<std::sync::Mutex<HashMap<String, CachedManifest>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

fn manifest_cache_get(url: &str, now: Instant) -> Option<Arc<HubManifest>> {
    let guard = MANIFEST_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = guard.get(url)?;
    (now.duration_since(entry.fetched_at) <= MANIFEST_CACHE_TTL)
        .then(|| Arc::clone(&entry.manifest))
}

fn manifest_cache_put(url: &str, manifest: Arc<HubManifest>, now: Instant) {
    let mut guard = MANIFEST_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.len() >= MANIFEST_CACHE_MAX_ENTRIES && !guard.contains_key(url) {
        guard.retain(|_, e| now.duration_since(e.fetched_at) <= MANIFEST_CACHE_TTL);
        if guard.len() >= MANIFEST_CACHE_MAX_ENTRIES
            && let Some(victim) = guard.keys().next().cloned()
        {
            guard.remove(&victim);
        }
    }
    guard.insert(
        url.to_string(),
        CachedManifest {
            manifest,
            fetched_at: now,
        },
    );
}

async fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), RegistryError> {
    let file_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("artifact");
    let temp_name = format!("{}.tmp.{}", file_name, uuid::Uuid::new_v4());
    let temp = target.with_file_name(temp_name);
    if let Err(e) = fs::write(&temp, bytes).await {
        cleanup_partial_artifact(&temp).await;
        return Err(RegistryError::Io(e));
    }
    if let Err(e) = confirm_written_size(&temp, bytes.len()).await {
        cleanup_partial_artifact(&temp).await;
        return Err(e);
    }
    if let Err(e) = fs::rename(&temp, target).await {
        cleanup_partial_artifact(&temp).await;
        return Err(RegistryError::Io(e));
    }
    Ok(())
}

pub(crate) fn validate_hub_artifact_name(
    name: &str,
    field: &'static str,
) -> Result<(), RegistryError> {
    let is_valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');

    if is_valid {
        Ok(())
    } else {
        Err(RegistryError::InvalidManifest {
            name: name.to_string(),
            field,
            reason: "name must be non-empty and contain only lowercase letters, digits, '-', '_'"
                .to_string(),
        })
    }
}

#[derive(Debug)]
pub struct HubInstallOutcome {
    pub name: String,
    pub version: String,
    pub release_tag: String,
    pub provenance: Provenance,
    pub primary_path: PathBuf,
    pub metadata_path: Option<PathBuf>,
}

pub struct HubInstaller {
    manifest_url: String,
    tools_dir: PathBuf,
    skills_dir: PathBuf,
}

impl HubInstaller {
    pub fn new(manifest_url: String, tools_dir: PathBuf, skills_dir: PathBuf) -> Self {
        Self {
            manifest_url,
            tools_dir,
            skills_dir,
        }
    }

    pub fn with_defaults() -> Self {
        let base = ironclaw_base_dir();
        Self::new(
            DEFAULT_HUB_MANIFEST_URL.to_string(),
            base.join("tools"),
            base.join("skills"),
        )
    }

    pub fn with_manifest_url(mut self, url: String) -> Self {
        self.manifest_url = url;
        self
    }

    pub fn with_tools_dir(mut self, dir: PathBuf) -> Self {
        self.tools_dir = dir;
        self
    }

    pub fn with_skills_dir(mut self, dir: PathBuf) -> Self {
        self.skills_dir = dir;
        self
    }

    pub fn manifest_url(&self) -> &str {
        &self.manifest_url
    }

    pub fn tools_dir(&self) -> &Path {
        &self.tools_dir
    }

    pub fn skills_dir(&self) -> &Path {
        &self.skills_dir
    }

    pub async fn fetch_manifest_cached(&self) -> Result<HubManifest, RegistryError> {
        let now = Instant::now();
        if let Some(hit) = manifest_cache_get(&self.manifest_url, now) {
            return Ok((*hit).clone());
        }
        let manifest = Arc::new(self.fetch_manifest().await?);
        manifest_cache_put(&self.manifest_url, Arc::clone(&manifest), now);
        Ok((*manifest).clone())
    }

    pub async fn fetch_manifest(&self) -> Result<HubManifest, RegistryError> {
        validate_artifact_url("hub-manifest", "manifest_url", &self.manifest_url)?;

        let bytes = download_artifact(&self.manifest_url, MAX_MANIFEST_BYTES as u64).await?;
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(RegistryError::DownloadFailed {
                url: self.manifest_url.clone(),
                reason: format!(
                    "manifest exceeds {} byte cap (got {})",
                    MAX_MANIFEST_BYTES,
                    bytes.len()
                ),
            });
        }

        serde_json::from_slice::<HubManifest>(&bytes).map_err(|e| RegistryError::ManifestParse {
            path: PathBuf::from(&self.manifest_url),
            reason: e.to_string(),
        })
    }

    pub async fn install_tool_from_manifest(
        &self,
        manifest: &HubManifest,
        name: &str,
        force: bool,
    ) -> Result<HubInstallOutcome, RegistryError> {
        let entry = manifest
            .find_tool(name)
            .ok_or_else(|| RegistryError::ExtensionNotFound(format!("tool '{}'", name)))?;
        self.install_tool_entry(entry, &manifest.release_tag, force)
            .await
    }

    pub async fn install_skill_from_manifest(
        &self,
        manifest: &HubManifest,
        name: &str,
        force: bool,
    ) -> Result<HubInstallOutcome, RegistryError> {
        let entry = manifest
            .find_skill(name)
            .ok_or_else(|| RegistryError::ExtensionNotFound(format!("skill '{}'", name)))?;
        self.install_skill_entry(entry, &manifest.release_tag, force)
            .await
    }

    fn tool_wasm_path(&self, name: &str) -> PathBuf {
        self.tools_dir.join(format!("{name}.wasm"))
    }

    fn skill_md_path(&self, name: &str) -> PathBuf {
        self.skills_dir.join(name).join("SKILL.md")
    }

    pub async fn install_tool_entry(
        &self,
        entry: &HubToolEntry,
        release_tag: &str,
        force: bool,
    ) -> Result<HubInstallOutcome, RegistryError> {
        validate_tool_entry(entry)?;

        if !force {
            let target = self.tool_wasm_path(&entry.name);
            if target.exists() {
                return Err(RegistryError::AlreadyInstalled {
                    name: entry.name.clone(),
                    path: target,
                });
            }
        }

        let wasm_bytes = download_artifact(&entry.wasm.url, MAX_WASM_BYTES as u64).await?;
        let caps_bytes =
            download_artifact(&entry.capabilities.url, MAX_METADATA_BYTES as u64).await?;

        self.install_tool_from_bytes(entry, release_tag, &wasm_bytes, &caps_bytes, force)
            .await
    }

    pub async fn install_skill_entry(
        &self,
        entry: &HubSkillEntry,
        release_tag: &str,
        force: bool,
    ) -> Result<HubInstallOutcome, RegistryError> {
        validate_skill_entry(entry)?;

        if !force {
            let target = self.skill_md_path(&entry.name);
            if target.exists() {
                return Err(RegistryError::AlreadyInstalled {
                    name: entry.name.clone(),
                    path: target,
                });
            }
        }

        let md_bytes = download_artifact(&entry.skill_md.url, MAX_METADATA_BYTES as u64).await?;

        self.install_skill_from_bytes(entry, release_tag, &md_bytes, force)
            .await
    }

    pub async fn install_tool_from_bytes(
        &self,
        entry: &HubToolEntry,
        release_tag: &str,
        wasm_bytes: &[u8],
        caps_bytes: &[u8],
        force: bool,
    ) -> Result<HubInstallOutcome, RegistryError> {
        validate_hub_artifact_name(&entry.name, "tools[].name")?;

        if wasm_bytes.len() > MAX_WASM_BYTES {
            return Err(RegistryError::DownloadFailed {
                url: entry.wasm.url.clone(),
                reason: format!(
                    "wasm exceeds {} byte cap (got {})",
                    MAX_WASM_BYTES,
                    wasm_bytes.len()
                ),
            });
        }
        if caps_bytes.len() > MAX_METADATA_BYTES {
            return Err(RegistryError::DownloadFailed {
                url: entry.capabilities.url.clone(),
                reason: format!(
                    "capabilities exceeds {} byte cap (got {})",
                    MAX_METADATA_BYTES,
                    caps_bytes.len()
                ),
            });
        }

        verify_sha256(wasm_bytes, &entry.wasm.sha256, &entry.wasm.url)?;
        verify_sha256(
            caps_bytes,
            &entry.capabilities.sha256,
            &entry.capabilities.url,
        )?;

        fs::create_dir_all(&self.tools_dir)
            .await
            .map_err(RegistryError::Io)?;

        let target_wasm = self.tool_wasm_path(&entry.name);
        let target_caps = self
            .tools_dir
            .join(format!("{}.capabilities.json", entry.name));

        let lock = acquire_install_lock(&format!("tool:{}", entry.name));
        let _guard = lock.lock().await;

        if target_wasm.exists() && !force {
            return Err(RegistryError::AlreadyInstalled {
                name: entry.name.clone(),
                path: target_wasm,
            });
        }

        write_atomic(&target_wasm, wasm_bytes).await?;
        if let Err(e) = write_atomic(&target_caps, caps_bytes).await {
            cleanup_partial_artifact(&target_wasm).await;
            return Err(e);
        }

        Ok(HubInstallOutcome {
            name: entry.name.clone(),
            version: entry.version.clone(),
            release_tag: release_tag.to_string(),
            provenance: entry.provenance,
            primary_path: target_wasm,
            metadata_path: Some(target_caps),
        })
    }

    pub async fn install_skill_from_bytes(
        &self,
        entry: &HubSkillEntry,
        release_tag: &str,
        md_bytes: &[u8],
        force: bool,
    ) -> Result<HubInstallOutcome, RegistryError> {
        validate_hub_artifact_name(&entry.name, "skills[].name")?;

        if md_bytes.len() > MAX_METADATA_BYTES {
            return Err(RegistryError::DownloadFailed {
                url: entry.skill_md.url.clone(),
                reason: format!(
                    "SKILL.md exceeds {} byte cap (got {})",
                    MAX_METADATA_BYTES,
                    md_bytes.len()
                ),
            });
        }

        verify_sha256(md_bytes, &entry.skill_md.sha256, &entry.skill_md.url)?;

        let skill_dir = self.skills_dir.join(&entry.name);
        fs::create_dir_all(&skill_dir)
            .await
            .map_err(RegistryError::Io)?;

        let target_md = self.skill_md_path(&entry.name);

        let lock = acquire_install_lock(&format!("skill:{}", entry.name));
        let _guard = lock.lock().await;

        if target_md.exists() && !force {
            return Err(RegistryError::AlreadyInstalled {
                name: entry.name.clone(),
                path: target_md,
            });
        }

        write_atomic(&target_md, md_bytes).await?;

        Ok(HubInstallOutcome {
            name: entry.name.clone(),
            version: entry.version.clone(),
            release_tag: release_tag.to_string(),
            provenance: entry.provenance,
            primary_path: target_md,
            metadata_path: None,
        })
    }
}

async fn confirm_written_size(path: &Path, expected: usize) -> Result<(), RegistryError> {
    let metadata = fs::metadata(path).await.map_err(RegistryError::Io)?;
    let actual = metadata.len() as usize;
    if actual != expected {
        return Err(RegistryError::DownloadFailed {
            url: path.display().to_string(),
            reason: format!(
                "on-disk size mismatch after write: expected {} bytes, got {}",
                expected, actual
            ),
        });
    }
    Ok(())
}

async fn cleanup_partial_artifact(path: &Path) {
    match fs::remove_file(path).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!("failed to remove partial artifact {}: {e}", path.display()),
    }
}

fn validate_tool_entry(entry: &HubToolEntry) -> Result<(), RegistryError> {
    validate_hub_artifact_name(&entry.name, "tools[].name")?;
    validate_artifact_url(&entry.name, "tools[].wasm.url", &entry.wasm.url)?;
    validate_artifact_url(
        &entry.name,
        "tools[].capabilities.url",
        &entry.capabilities.url,
    )?;
    Ok(())
}

fn validate_skill_entry(entry: &HubSkillEntry) -> Result<(), RegistryError> {
    validate_hub_artifact_name(&entry.name, "skills[].name")?;
    validate_artifact_url(&entry.name, "skills[].skill_md.url", &entry.skill_md.url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::hub_manifest::{
        HubArtifact, HubManifest, HubSkillEntry, HubToolEntry, Provenance,
    };
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    fn build_tool_entry(name: &str, wasm_bytes: &[u8], caps_bytes: &[u8]) -> HubToolEntry {
        HubToolEntry {
            name: name.to_string(),
            crate_name: format!("{}-tool", name),
            version: "0.1.0".to_string(),
            description: format!("{} integration", name),
            provenance: Provenance::Official,
            wasm: HubArtifact {
                url: format!(
                    "https://github.com/nearai/ironhub/releases/download/test/{}.wasm",
                    name
                ),
                size_bytes: wasm_bytes.len() as u64,
                sha256: sha256_hex(wasm_bytes),
            },
            capabilities: HubArtifact {
                url: format!(
                    "https://github.com/nearai/ironhub/releases/download/test/{}.capabilities.json",
                    name
                ),
                size_bytes: caps_bytes.len() as u64,
                sha256: sha256_hex(caps_bytes),
            },
        }
    }

    fn build_skill_entry(name: &str, md_bytes: &[u8]) -> HubSkillEntry {
        HubSkillEntry {
            name: name.to_string(),
            trunk: "test-tool".to_string(),
            version: "1.0.0".to_string(),
            description: format!("{} skill", name),
            provenance: Provenance::Official,
            skill_md: HubArtifact {
                url: format!(
                    "https://github.com/nearai/ironhub/releases/download/test/{}.SKILL.md",
                    name
                ),
                size_bytes: md_bytes.len() as u64,
                sha256: sha256_hex(md_bytes),
            },
        }
    }

    fn build_manifest(tools: Vec<HubToolEntry>, skills: Vec<HubSkillEntry>) -> HubManifest {
        HubManifest {
            version: "1".to_string(),
            generated_at: "2026-05-13T00:00:00Z".to_string(),
            release_tag: "test-release".to_string(),
            repo: "nearai/ironhub".to_string(),
            tools,
            skills,
        }
    }

    fn installer_in(tmp: &TempDir) -> HubInstaller {
        HubInstaller::new(
            DEFAULT_HUB_MANIFEST_URL.to_string(),
            tmp.path().join("tools"),
            tmp.path().join("skills"),
        )
    }

    #[test]
    fn defaults_use_ironclaw_base_dirs() {
        let installer = HubInstaller::with_defaults();
        let base = ironclaw_base_dir();
        assert_eq!(installer.tools_dir(), base.join("tools"));
        assert_eq!(installer.skills_dir(), base.join("skills"));
        assert_eq!(installer.manifest_url(), DEFAULT_HUB_MANIFEST_URL);
    }

    #[test]
    fn pinned_manifest_url_replaces_default() {
        let pinned =
            "https://github.com/nearai/ironhub/releases/download/test/tools.json".to_string();
        let installer = HubInstaller::with_defaults().with_manifest_url(pinned.clone());
        assert_eq!(installer.manifest_url(), pinned);
    }

    #[test]
    fn with_tools_dir_overrides_default() {
        let custom = PathBuf::from("/custom/tools");
        let installer = HubInstaller::with_defaults().with_tools_dir(custom.clone());
        assert_eq!(installer.tools_dir(), custom);
        let base = ironclaw_base_dir();
        assert_eq!(installer.skills_dir(), base.join("skills"));
    }

    #[test]
    fn with_skills_dir_overrides_default() {
        let custom = PathBuf::from("/custom/skills");
        let installer = HubInstaller::with_defaults().with_skills_dir(custom.clone());
        assert_eq!(installer.skills_dir(), custom);
        let base = ironclaw_base_dir();
        assert_eq!(installer.tools_dir(), base.join("tools"));
    }

    #[tokio::test]
    async fn install_tool_from_bytes_serializes_concurrent_same_name_installs() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = Arc::new(installer_in(&tmp));
        let wasm = b"fake-wasm-bytes";
        let caps = br#"{"name":"clickup"}"#;
        let entry_a = build_tool_entry("clickup", wasm, caps);
        let entry_b = entry_a.clone();
        let a = Arc::clone(&installer);
        let b = Arc::clone(&installer);
        let (r1, r2) = tokio::join!(
            async move {
                a.install_tool_from_bytes(&entry_a, "test-release", wasm, caps, false)
                    .await
            },
            async move {
                b.install_tool_from_bytes(&entry_b, "test-release", wasm, caps, false)
                    .await
            },
        );

        let outcomes = [r1, r2];
        let oks = outcomes.iter().filter(|r| r.is_ok()).count();
        let already_installed = outcomes
            .iter()
            .filter(|r| matches!(r, Err(RegistryError::AlreadyInstalled { .. })))
            .count();
        assert_eq!(oks, 1, "exactly one concurrent install must succeed");
        assert_eq!(
            already_installed, 1,
            "the other must see AlreadyInstalled, not a race-induced failure"
        );

        let target_wasm = tmp.path().join("tools/clickup.wasm");
        let target_caps = tmp.path().join("tools/clickup.capabilities.json");
        assert!(
            target_wasm.exists(),
            "wasm must survive: losing install's cleanup must not delete the winner's artifact"
        );
        assert!(
            target_caps.exists(),
            "capabilities must survive the loser's cleanup"
        );
        let stray: Vec<_> = std::fs::read_dir(tmp.path().join("tools"))
            .expect("read tools dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(
            stray.is_empty(),
            "no temp files must remain on disk after install"
        );
    }

    #[test]
    fn manifest_cache_hits_within_ttl_and_expires_after() {
        let url = "https://hub.ironclaw.com/manifest-cache-test.json";
        let manifest = Arc::new(build_manifest(vec![], vec![]));
        let base = Instant::now();
        manifest_cache_put(url, Arc::clone(&manifest), base);
        assert!(
            manifest_cache_get(url, base).is_some(),
            "a fresh entry must be a cache hit"
        );
        let later = base + MANIFEST_CACHE_TTL + Duration::from_secs(1);
        assert!(
            manifest_cache_get(url, later).is_none(),
            "an entry past its TTL must miss"
        );
    }

    #[tokio::test]
    async fn install_lock_entry_reclaimed_after_install() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let wasm = b"fake-wasm-bytes";
        let caps = br#"{"name":"reclaim-probe"}"#;
        let entry = build_tool_entry("reclaim-probe", wasm, caps);
        installer
            .install_tool_from_bytes(&entry, "test-release", wasm, caps, false)
            .await
            .expect("install succeeds");
        let retained = INSTALL_LOCKS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key("tool:reclaim-probe");
        assert!(
            !retained,
            "install lock entry must be reclaimed after the install completes"
        );
    }

    #[tokio::test]
    async fn install_tool_entry_skips_download_when_already_installed() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        std::fs::create_dir_all(tmp.path().join("tools")).expect("tools dir");
        std::fs::write(tmp.path().join("tools/clickup.wasm"), b"already here").expect("seed");

        let entry = build_tool_entry("clickup", b"fake-wasm-bytes", br#"{"name":"clickup"}"#);
        let result = installer
            .install_tool_entry(&entry, "test-release", false)
            .await;
        assert!(
            matches!(result, Err(RegistryError::AlreadyInstalled { .. })),
            "force=false re-install must short-circuit before downloading, got {result:?}"
        );
    }

    #[tokio::test]
    async fn install_tool_from_bytes_writes_artifacts_and_returns_outcome() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let wasm = b"fake-wasm-bytes";
        let caps = br#"{"name":"clickup"}"#;
        let entry = build_tool_entry("clickup", wasm, caps);

        let outcome = installer
            .install_tool_from_bytes(&entry, "test-release", wasm, caps, false)
            .await
            .expect("install succeeds");

        assert_eq!(outcome.name, "clickup");
        assert_eq!(outcome.version, "0.1.0");
        assert_eq!(outcome.release_tag, "test-release");
        assert_eq!(outcome.primary_path, tmp.path().join("tools/clickup.wasm"));
        assert_eq!(
            outcome.metadata_path,
            Some(tmp.path().join("tools/clickup.capabilities.json"))
        );

        let wasm_on_disk = fs::read(&outcome.primary_path).await.expect("read wasm");
        assert_eq!(wasm_on_disk, wasm);
        let caps_on_disk = fs::read(outcome.metadata_path.as_ref().unwrap())
            .await
            .expect("read caps");
        assert_eq!(caps_on_disk, caps);
    }

    #[tokio::test]
    async fn install_tool_from_bytes_rejects_wasm_sha_mismatch() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let wasm = b"original-wasm";
        let caps = br#"{"name":"clickup"}"#;
        let entry = build_tool_entry("clickup", wasm, caps);

        let tampered_wasm = b"tampered-wasm";
        let err = installer
            .install_tool_from_bytes(&entry, "test-release", tampered_wasm, caps, false)
            .await
            .expect_err("sha mismatch must fail");
        assert!(matches!(err, RegistryError::ChecksumMismatch { .. }));
        assert!(!tmp.path().join("tools/clickup.wasm").exists());
    }

    #[tokio::test]
    async fn install_tool_from_bytes_rejects_caps_sha_mismatch() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let wasm = b"original-wasm";
        let caps = br#"{"name":"clickup"}"#;
        let entry = build_tool_entry("clickup", wasm, caps);

        let tampered_caps = br#"{"name":"tampered"}"#;
        let err = installer
            .install_tool_from_bytes(&entry, "test-release", wasm, tampered_caps, false)
            .await
            .expect_err("sha mismatch must fail");
        assert!(matches!(err, RegistryError::ChecksumMismatch { .. }));
        assert!(!tmp.path().join("tools/clickup.wasm").exists());
        assert!(!tmp.path().join("tools/clickup.capabilities.json").exists());
    }

    #[tokio::test]
    async fn install_tool_from_bytes_rejects_oversized_caps() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let wasm = b"fake-wasm";
        let big_caps = vec![b'x'; MAX_METADATA_BYTES + 1];
        let entry = build_tool_entry("clickup", wasm, &big_caps);

        let err = installer
            .install_tool_from_bytes(&entry, "test-release", wasm, &big_caps, false)
            .await
            .expect_err("oversized caps must fail");
        match err {
            RegistryError::DownloadFailed { reason, .. } => {
                assert!(reason.contains("capabilities exceeds"));
            }
            other => panic!("expected DownloadFailed, got {:?}", other),
        }
        assert!(!tmp.path().join("tools/clickup.wasm").exists());
    }

    #[tokio::test]
    async fn install_tool_from_bytes_rejects_oversized_wasm() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let big_wasm = vec![b'x'; MAX_WASM_BYTES + 1];
        let caps = br#"{"name":"clickup"}"#;
        let entry = build_tool_entry("clickup", &big_wasm, caps);

        let err = installer
            .install_tool_from_bytes(&entry, "test-release", &big_wasm, caps, false)
            .await
            .expect_err("oversized wasm must fail");
        match err {
            RegistryError::DownloadFailed { reason, .. } => {
                assert!(reason.contains("wasm exceeds"));
            }
            other => panic!("expected DownloadFailed, got {:?}", other),
        }
        assert!(!tmp.path().join("tools/clickup.wasm").exists());
    }

    #[tokio::test]
    async fn install_tool_from_bytes_refuses_overwrite_without_force() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let wasm = b"wasm";
        let caps = br#"{"name":"clickup"}"#;
        let entry = build_tool_entry("clickup", wasm, caps);

        installer
            .install_tool_from_bytes(&entry, "test-release", wasm, caps, false)
            .await
            .expect("first install");

        let err = installer
            .install_tool_from_bytes(&entry, "test-release", wasm, caps, false)
            .await
            .expect_err("second install must fail without force");
        assert!(matches!(err, RegistryError::AlreadyInstalled { .. }));
    }

    #[tokio::test]
    async fn install_tool_from_bytes_overwrites_with_force() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let original = b"original-wasm";
        let caps = br#"{"name":"clickup"}"#;
        let entry_v1 = build_tool_entry("clickup", original, caps);

        installer
            .install_tool_from_bytes(&entry_v1, "test-release", original, caps, false)
            .await
            .expect("first install");

        let updated = b"updated-wasm-bytes";
        let entry_v2 = build_tool_entry("clickup", updated, caps);
        installer
            .install_tool_from_bytes(&entry_v2, "test-release", updated, caps, true)
            .await
            .expect("forced reinstall");

        let on_disk = fs::read(tmp.path().join("tools/clickup.wasm"))
            .await
            .expect("read");
        assert_eq!(on_disk, updated);
    }

    #[tokio::test]
    async fn install_skill_from_bytes_writes_skill_md() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let md = b"# Chief of Staff\n\nactivation:\n  keywords:\n    - briefing\n";
        let entry = build_skill_entry("chief-of-staff", md);

        let outcome = installer
            .install_skill_from_bytes(&entry, "test-release", md, false)
            .await
            .expect("skill install");
        assert_eq!(outcome.name, "chief-of-staff");
        assert_eq!(
            outcome.primary_path,
            tmp.path().join("skills/chief-of-staff/SKILL.md")
        );
        assert!(outcome.metadata_path.is_none());

        let on_disk = fs::read(outcome.primary_path).await.expect("read");
        assert_eq!(on_disk, md);
    }

    #[tokio::test]
    async fn install_skill_from_bytes_rejects_sha_mismatch() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let md = b"# original";
        let entry = build_skill_entry("test-skill", md);

        let tampered = b"# tampered";
        let err = installer
            .install_skill_from_bytes(&entry, "test-release", tampered, false)
            .await
            .expect_err("sha mismatch must fail");
        assert!(matches!(err, RegistryError::ChecksumMismatch { .. }));
        assert!(!tmp.path().join("skills/test-skill/SKILL.md").exists());
    }

    #[tokio::test]
    async fn install_skill_from_bytes_rejects_oversized() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let big = vec![b'x'; MAX_METADATA_BYTES + 1];
        let entry = build_skill_entry("test-skill", &big);

        let err = installer
            .install_skill_from_bytes(&entry, "test-release", &big, false)
            .await
            .expect_err("oversized must fail");
        match err {
            RegistryError::DownloadFailed { reason, .. } => {
                assert!(reason.contains("SKILL.md exceeds"));
            }
            other => panic!("expected DownloadFailed, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn install_skill_from_bytes_refuses_overwrite_without_force() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let md = b"# skill";
        let entry = build_skill_entry("test-skill", md);

        installer
            .install_skill_from_bytes(&entry, "test-release", md, false)
            .await
            .expect("first install");

        let err = installer
            .install_skill_from_bytes(&entry, "test-release", md, false)
            .await
            .expect_err("second install must fail");
        assert!(matches!(err, RegistryError::AlreadyInstalled { .. }));
    }

    #[tokio::test]
    async fn install_skill_from_bytes_overwrites_with_force() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let original = b"# v1";
        let entry_v1 = build_skill_entry("test-skill", original);
        installer
            .install_skill_from_bytes(&entry_v1, "test-release", original, false)
            .await
            .expect("first install");

        let updated = b"# v2";
        let entry_v2 = build_skill_entry("test-skill", updated);
        installer
            .install_skill_from_bytes(&entry_v2, "test-release", updated, true)
            .await
            .expect("forced reinstall");

        let on_disk = fs::read(tmp.path().join("skills/test-skill/SKILL.md"))
            .await
            .expect("read");
        assert_eq!(on_disk, updated);
    }

    #[tokio::test]
    async fn install_tool_entry_rejects_non_https_url() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let wasm = b"wasm";
        let caps = br#"{}"#;
        let mut entry = build_tool_entry("clickup", wasm, caps);
        entry.wasm.url =
            "http://github.com/nearai/ironhub/releases/download/test/clickup.wasm".to_string();

        let err = installer
            .install_tool_entry(&entry, "test-release", false)
            .await
            .expect_err("non-https must fail before fetch");
        match err {
            RegistryError::InvalidManifest { reason, .. } => {
                assert!(reason.contains("https"));
            }
            other => panic!("expected InvalidManifest, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn install_tool_entry_rejects_disallowed_host() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let wasm = b"wasm";
        let caps = br#"{}"#;
        let mut entry = build_tool_entry("clickup", wasm, caps);
        entry.wasm.url = "https://evil.example.com/clickup.wasm".to_string();

        let err = installer
            .install_tool_entry(&entry, "test-release", false)
            .await
            .expect_err("disallowed host must fail before fetch");
        match err {
            RegistryError::InvalidManifest { reason, .. } => {
                assert!(reason.contains("not allowed"));
            }
            other => panic!("expected InvalidManifest, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn install_tool_entry_rejects_disallowed_caps_host() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let wasm = b"wasm";
        let caps = br#"{}"#;
        let mut entry = build_tool_entry("clickup", wasm, caps);
        entry.capabilities.url = "https://evil.example.com/clickup.capabilities.json".to_string();

        let err = installer
            .install_tool_entry(&entry, "test-release", false)
            .await
            .expect_err("disallowed caps host must fail before fetch");
        assert!(matches!(err, RegistryError::InvalidManifest { .. }));
    }

    #[tokio::test]
    async fn install_skill_entry_rejects_disallowed_host() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let md = b"# skill";
        let mut entry = build_skill_entry("test-skill", md);
        entry.skill_md.url = "https://evil.example.com/test-skill.SKILL.md".to_string();

        let err = installer
            .install_skill_entry(&entry, "test-release", false)
            .await
            .expect_err("disallowed skill host must fail before fetch");
        assert!(matches!(err, RegistryError::InvalidManifest { .. }));
    }

    #[tokio::test]
    async fn fetch_manifest_rejects_non_https_url() {
        let installer = HubInstaller::with_defaults().with_manifest_url(
            "http://github.com/nearai/ironhub/releases/latest/download/tools.json".to_string(),
        );

        let err = installer
            .fetch_manifest()
            .await
            .expect_err("non-https manifest url must fail");
        assert!(matches!(err, RegistryError::InvalidManifest { .. }));
    }

    #[tokio::test]
    async fn fetch_manifest_rejects_disallowed_host() {
        let installer = HubInstaller::with_defaults()
            .with_manifest_url("https://evil.example.com/tools.json".to_string());

        let err = installer
            .fetch_manifest()
            .await
            .expect_err("disallowed manifest host must fail");
        assert!(matches!(err, RegistryError::InvalidManifest { .. }));
    }

    #[tokio::test]
    async fn install_tool_from_manifest_reports_missing_tool() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let manifest = build_manifest(vec![build_tool_entry("clickup", b"wasm", br#"{}"#)], vec![]);

        let err = installer
            .install_tool_from_manifest(&manifest, "absent", false)
            .await
            .expect_err("missing tool must fail");
        match err {
            RegistryError::ExtensionNotFound(msg) => {
                assert!(msg.contains("absent"));
                assert!(msg.contains("tool"));
            }
            other => panic!("expected ExtensionNotFound, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn install_skill_from_manifest_reports_missing_skill() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let manifest = build_manifest(vec![], vec![build_skill_entry("present", b"# md")]);

        let err = installer
            .install_skill_from_manifest(&manifest, "absent", false)
            .await
            .expect_err("missing skill must fail");
        match err {
            RegistryError::ExtensionNotFound(msg) => {
                assert!(msg.contains("absent"));
                assert!(msg.contains("skill"));
            }
            other => panic!("expected ExtensionNotFound, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn install_tool_from_manifest_delegates_to_entry_validation() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let wasm = b"wasm";
        let caps = br#"{"name":"clickup"}"#;
        let mut entry = build_tool_entry("clickup", wasm, caps);
        entry.wasm.url = "https://evil.example.com/clickup.wasm".to_string();
        let manifest = build_manifest(vec![entry], vec![]);

        let err = installer
            .install_tool_from_manifest(&manifest, "clickup", false)
            .await
            .expect_err("delegation must surface entry validation error");
        assert!(matches!(err, RegistryError::InvalidManifest { .. }));
    }

    #[tokio::test]
    async fn install_skill_from_manifest_delegates_to_entry_validation() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let md = b"# skill";
        let mut entry = build_skill_entry("test-skill", md);
        entry.skill_md.url = "https://evil.example.com/test-skill.SKILL.md".to_string();
        let manifest = build_manifest(vec![], vec![entry]);

        let err = installer
            .install_skill_from_manifest(&manifest, "test-skill", false)
            .await
            .expect_err("delegation must surface entry validation error");
        assert!(matches!(err, RegistryError::InvalidManifest { .. }));
    }

    #[test]
    fn validate_hub_artifact_name_accepts_valid_names() {
        assert!(validate_hub_artifact_name("clickup", "test").is_ok());
        assert!(validate_hub_artifact_name("evm-rpc", "test").is_ok());
        assert!(validate_hub_artifact_name("microsoft_365", "test").is_ok());
        assert!(validate_hub_artifact_name("a1-b2_c3", "test").is_ok());
    }

    #[test]
    fn validate_hub_artifact_name_rejects_traversal_and_unsafe_chars() {
        for bad in [
            "",
            "..",
            "../evil",
            "/etc/passwd",
            "evil/sub",
            "evil\\sub",
            "evil.wasm",
            "Uppercase",
            "name with space",
            "name\nnewline",
            "name\0null",
            "name@host",
        ] {
            assert!(
                validate_hub_artifact_name(bad, "test").is_err(),
                "expected rejection for {:?}",
                bad
            );
        }
    }

    #[tokio::test]
    async fn install_tool_from_bytes_rejects_traversal_in_entry_name() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let wasm = b"wasm";
        let caps = br#"{}"#;
        let mut entry = build_tool_entry("clickup", wasm, caps);
        entry.name = "../evil".to_string();

        let err = installer
            .install_tool_from_bytes(&entry, "test-release", wasm, caps, false)
            .await
            .expect_err("traversal in entry name must fail");
        assert!(matches!(err, RegistryError::InvalidManifest { .. }));
        assert!(!tmp.path().join("tools/../evil.wasm").exists());
    }

    #[tokio::test]
    async fn install_tool_from_bytes_rejects_absolute_path_in_entry_name() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let wasm = b"wasm";
        let caps = br#"{}"#;
        let mut entry = build_tool_entry("clickup", wasm, caps);
        entry.name = "/tmp/escape-me".to_string();

        let err = installer
            .install_tool_from_bytes(&entry, "test-release", wasm, caps, false)
            .await
            .expect_err("absolute name must fail");
        assert!(matches!(err, RegistryError::InvalidManifest { .. }));
        assert!(!std::path::Path::new("/tmp/escape-me.wasm").exists());
    }

    #[tokio::test]
    async fn install_tool_entry_rejects_bad_name_before_fetch() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let wasm = b"wasm";
        let caps = br#"{}"#;
        let mut entry = build_tool_entry("clickup", wasm, caps);
        entry.name = "../evil".to_string();

        let err = installer
            .install_tool_entry(&entry, "test-release", false)
            .await
            .expect_err("bad name must fail before any HTTP fetch");
        assert!(matches!(err, RegistryError::InvalidManifest { .. }));
    }

    #[tokio::test]
    async fn install_skill_from_bytes_rejects_traversal_in_entry_name() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let md = b"# skill";
        let mut entry = build_skill_entry("test-skill", md);
        entry.name = "../evil".to_string();

        let err = installer
            .install_skill_from_bytes(&entry, "test-release", md, false)
            .await
            .expect_err("traversal in skill name must fail");
        assert!(matches!(err, RegistryError::InvalidManifest { .. }));
        assert!(!tmp.path().join("skills/../evil/SKILL.md").exists());
    }

    #[tokio::test]
    async fn install_skill_from_bytes_rejects_absolute_path_in_entry_name() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let md = b"# skill";
        let mut entry = build_skill_entry("test-skill", md);
        entry.name = "/tmp/escape-skill".to_string();

        let err = installer
            .install_skill_from_bytes(&entry, "test-release", md, false)
            .await
            .expect_err("absolute skill name must fail");
        assert!(matches!(err, RegistryError::InvalidManifest { .. }));
    }

    #[tokio::test]
    async fn install_skill_entry_rejects_bad_name_before_fetch() {
        let tmp = TempDir::new().expect("tempdir");
        let installer = installer_in(&tmp);
        let md = b"# skill";
        let mut entry = build_skill_entry("test-skill", md);
        entry.name = "../evil".to_string();

        let err = installer
            .install_skill_entry(&entry, "test-release", false)
            .await
            .expect_err("bad name must fail before any HTTP fetch");
        assert!(matches!(err, RegistryError::InvalidManifest { .. }));
    }

    #[tokio::test]
    async fn confirm_written_size_passes_on_match() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("artifact.bin");
        let bytes = b"hello world";
        fs::write(&path, bytes).await.expect("write");
        confirm_written_size(&path, bytes.len())
            .await
            .expect("matching size must pass");
    }

    #[tokio::test]
    async fn confirm_written_size_rejects_truncation() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("artifact.bin");
        fs::write(&path, b"actual").await.expect("write");
        let err = confirm_written_size(&path, 9999)
            .await
            .expect_err("size mismatch must fail");
        match err {
            RegistryError::DownloadFailed { reason, .. } => {
                assert!(reason.contains("on-disk size mismatch"));
            }
            other => panic!("expected DownloadFailed, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn cleanup_partial_artifact_removes_file_and_tolerates_missing() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("orphan.wasm");
        fs::write(&path, b"partial").await.expect("write");
        assert!(path.exists());
        cleanup_partial_artifact(&path).await;
        assert!(!path.exists(), "partial artifact must be removed");
        cleanup_partial_artifact(&path).await;
        assert!(!path.exists(), "second call on missing path is a no-op");
    }
}
