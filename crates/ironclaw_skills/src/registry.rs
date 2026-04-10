//! Skill registry for discovering, loading, and managing available skills.
//!
//! Skills are discovered from multiple sources:
//! 1. Workspace skills directory (`<workspace>/skills/`) -- Trusted
//! 2. User skills directory (`~/.ironclaw/skills/`) -- Trusted
//! 3. Installed skills directory (`~/.ironclaw/installed_skills/`) -- Installed
//! 4. Bundled skills compiled into the binary -- Trusted
//!
//! Both flat (`skills/SKILL.md`) and subdirectory (`skills/<name>/SKILL.md`)
//! layouts are supported. Subdirectories without `SKILL.md` are treated as
//! bundle directories and recursed into (up to `SKILLS_MAX_SCAN_DEPTH`,
//! default 3). Earlier sources win on name collision (workspace overrides
//! user overrides installed overrides bundled).
//! Uses async I/O throughout to avoid blocking the tokio runtime.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::gating;
use crate::parser::{
    SkillParseError, parse_skill_md, parse_skill_md_for_install_recovery,
    split_skill_md_frontmatter,
};
use crate::types::{
    GatingRequirements, LoadedSkill, MAX_PROMPT_FILE_SIZE, SkillSource, SkillTrust,
};
use crate::validation::{normalize_line_endings, normalize_skill_identifier};

/// Maximum total number of skills that can be discovered across all sources.
/// Shared across workspace, user, and installed directories.
/// Prevents resource exhaustion from directories with thousands of entries.
const MAX_DISCOVERED_SKILLS: usize = 100;

/// Default recursion depth for bundle directory scanning.
const DEFAULT_MAX_SCAN_DEPTH: usize = 3;

fn to_lowercase_vec(items: &[String]) -> Vec<String> {
    items.iter().map(|s| s.to_lowercase()).collect()
}

fn parse_error_for_install(error_label: &str, error: SkillParseError) -> SkillRegistryError {
    let reason = error.to_string();
    match error {
        SkillParseError::InvalidName { name } => SkillRegistryError::ParseError { name, reason },
        _ => SkillRegistryError::ParseError {
            name: error_label.to_string(),
            reason,
        },
    }
}

/// Rewrite the `name` field in raw YAML frontmatter while preserving every
/// other key and value in the original mapping.
///
/// We deliberately operate on `serde_yml::Value` instead of the typed
/// `SkillManifest`: re-serializing through the typed struct silently drops
/// any unknown frontmatter fields published upstream (custom metadata, future
/// fields, vendor extensions). The recovery path must be lossless except for
/// the single field we are rewriting.
fn rewrite_frontmatter_name(
    frontmatter: &str,
    new_name: &str,
    error_label: &str,
) -> Result<String, SkillRegistryError> {
    let mut value: serde_yml::Value =
        serde_yml::from_str(frontmatter).map_err(|e| SkillRegistryError::ParseError {
            name: error_label.to_string(),
            reason: format!("Failed to parse SKILL.md frontmatter for rewrite: {}", e),
        })?;

    let mapping = value
        .as_mapping_mut()
        .ok_or_else(|| SkillRegistryError::ParseError {
            name: error_label.to_string(),
            reason: "SKILL.md frontmatter is not a YAML mapping".to_string(),
        })?;

    mapping.insert(
        serde_yml::Value::String("name".to_string()),
        serde_yml::Value::String(new_name.to_string()),
    );

    let yaml = serde_yml::to_string(&value).map_err(|e| SkillRegistryError::ParseError {
        name: error_label.to_string(),
        reason: format!("Failed to rewrite normalized SKILL.md: {}", e),
    })?;

    let yaml = yaml.strip_suffix("...\n").unwrap_or(&yaml);
    let yaml = yaml.strip_suffix("...").unwrap_or(yaml);
    Ok(yaml.to_string())
}

fn assemble_skill_md(yaml: &str, prompt_content: &str) -> String {
    let mut rendered = String::from("---\n");
    rendered.push_str(yaml);
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    rendered.push_str("---\n\n");
    rendered.push_str(prompt_content);
    rendered
}

fn normalize_install_content(
    normalized_content: &str,
    requested_identifier: Option<&str>,
) -> Result<(String, String), SkillRegistryError> {
    match parse_skill_md(normalized_content) {
        Ok(parsed) => Ok((parsed.manifest.name, normalized_content.to_string())),
        Err(SkillParseError::InvalidName { .. }) => {
            // Re-parse the typed manifest only to recover the original name and
            // confirm structural validity; the actual rewrite operates on raw
            // YAML below to preserve any unknown frontmatter fields.
            let parsed = parse_skill_md_for_install_recovery(normalized_content)
                .map_err(|e| parse_error_for_install("(install)", e))?;
            let original_name = parsed.manifest.name.clone();
            let normalized_name = requested_identifier
                .and_then(normalize_skill_identifier)
                .or_else(|| normalize_skill_identifier(&original_name))
                .ok_or_else(|| SkillRegistryError::ParseError {
                    name: original_name.clone(),
                    reason: format!(
                        "Invalid skill name '{}' could not be normalized to a safe install name",
                        original_name
                    ),
                })?;

            tracing::debug!(
                original_name = %original_name,
                normalized_name = %normalized_name,
                requested_identifier = requested_identifier.unwrap_or(""),
                "Normalizing invalid skill name during install"
            );

            let (frontmatter, prompt_content) = split_skill_md_frontmatter(normalized_content)
                .map_err(|e| parse_error_for_install("(install)", e))?;
            let rewritten_yaml =
                rewrite_frontmatter_name(&frontmatter, &normalized_name, &original_name)?;
            let rendered = assemble_skill_md(&rewritten_yaml, &prompt_content);
            Ok((normalized_name, rendered))
        }
        Err(e) => Err(parse_error_for_install("(install)", e)),
    }
}

/// Error type for skill registry operations.
#[derive(Debug, thiserror::Error)]
pub enum SkillRegistryError {
    #[error("Skill not found: {0}")]
    NotFound(String),

    #[error("Failed to read skill file {path}: {reason}")]
    ReadError { path: String, reason: String },

    #[error("Failed to parse SKILL.md for '{name}': {reason}")]
    ParseError { name: String, reason: String },

    #[error("Skill file too large for '{name}': {size} bytes (max {max} bytes)")]
    FileTooLarge { name: String, size: u64, max: u64 },

    #[error("Symlink detected in skills directory: {path}")]
    SymlinkDetected { path: String },

    #[error("Skill '{name}' failed gating: {reason}")]
    GatingFailed { name: String, reason: String },

    #[error(
        "Skill '{name}' prompt exceeds token budget: ~{approx_tokens} tokens but declares max_context_tokens={declared}"
    )]
    TokenBudgetExceeded {
        name: String,
        approx_tokens: usize,
        declared: usize,
    },

    #[error("Skill '{name}' already exists")]
    AlreadyExists { name: String },

    #[error("Cannot remove skill '{name}': {reason}")]
    CannotRemove { name: String, reason: String },

    #[error("Failed to write skill file {path}: {reason}")]
    WriteError { path: String, reason: String },
}

/// Registry of available skills.
pub struct SkillRegistry {
    /// All loaded skills.
    skills: Vec<LoadedSkill>,
    /// User skills directory (~/.ironclaw/skills/). Skills here are Trusted.
    user_dir: PathBuf,
    /// Registry-installed skills directory (~/.ironclaw/installed_skills/). Skills here are Installed.
    installed_dir: Option<PathBuf>,
    /// Optional workspace skills directory.
    workspace_dir: Option<PathBuf>,
    /// Bundled skill content compiled into the binary (name, raw SKILL.md content).
    /// Loaded as Trusted at lowest discovery priority.
    bundled_content: &'static [(String, String)],
    /// Maximum recursion depth for bundle directory scanning (default: 3).
    max_scan_depth: usize,
}

impl SkillRegistry {
    /// Create a new skill registry.
    pub fn new(user_dir: PathBuf) -> Self {
        Self {
            skills: Vec::new(),
            user_dir,
            installed_dir: None,
            workspace_dir: None,
            bundled_content: &[],
            max_scan_depth: DEFAULT_MAX_SCAN_DEPTH,
        }
    }

    /// Set the registry-installed skills directory.
    ///
    /// Skills installed via ClawHub or the skill tools are written here and
    /// loaded with `SkillTrust::Installed` (read-only tool access). This
    /// directory is separate from the user dir so that trust levels survive
    /// restarts correctly.
    pub fn with_installed_dir(mut self, dir: PathBuf) -> Self {
        self.installed_dir = Some(dir);
        self
    }

    /// Set a workspace skills directory.
    pub fn with_workspace_dir(mut self, dir: PathBuf) -> Self {
        self.workspace_dir = Some(dir);
        self
    }

    /// Set bundled skill content compiled into the binary.
    ///
    /// Each entry is `(skill_name, raw_skill_md_content)`. These skills are
    /// discovered at the lowest priority (after workspace, user, and installed)
    /// with `SkillTrust::Trusted` since they ship with the application binary.
    pub fn with_bundled_content(mut self, content: &'static [(String, String)]) -> Self {
        self.bundled_content = content;
        self
    }

    /// Set the maximum recursion depth for bundle directory scanning.
    pub fn with_max_scan_depth(mut self, depth: usize) -> Self {
        self.max_scan_depth = depth;
        self
    }

    /// Discover and load skills from all configured directories.
    ///
    /// Discovery order (earlier wins on name collision):
    /// 1. Workspace skills directory (if set) -- Trusted
    /// 2. User skills directory -- Trusted
    /// 3. Installed skills directory (if set) -- Installed
    pub async fn discover_all(&mut self) -> Vec<String> {
        let mut loaded_names: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        // 1. Workspace skills (highest priority)
        if let Some(ws_dir) = self.workspace_dir.clone() {
            let cap = MAX_DISCOVERED_SKILLS.saturating_sub(loaded_names.len());
            let skills = self
                .discover_from_dir(
                    &ws_dir,
                    SkillTrust::Trusted,
                    &SkillSource::Workspace,
                    cap,
                    0,
                )
                .await;
            self.absorb(skills, &mut seen, &mut loaded_names, "workspace");
        }

        // 2. User skills
        if loaded_names.len() < MAX_DISCOVERED_SKILLS {
            let cap = MAX_DISCOVERED_SKILLS.saturating_sub(loaded_names.len());
            let user_dir = self.user_dir.clone();
            let skills = self
                .discover_from_dir(&user_dir, SkillTrust::Trusted, &SkillSource::User, cap, 0)
                .await;
            self.absorb(skills, &mut seen, &mut loaded_names, "workspace");
        }

        // 3. Installed skills (registry-installed)
        if loaded_names.len() < MAX_DISCOVERED_SKILLS
            && let Some(inst_dir) = self.installed_dir.clone()
        {
            let cap = MAX_DISCOVERED_SKILLS.saturating_sub(loaded_names.len());
            let skills = self
                .discover_from_dir(
                    &inst_dir,
                    SkillTrust::Installed,
                    &SkillSource::Installed,
                    cap,
                    0,
                )
                .await;
            self.absorb(skills, &mut seen, &mut loaded_names, "workspace or user");
        }

        // 4. Bundled skills (compiled into binary, lowest priority)
        if !self.bundled_content.is_empty() {
            let bundled = self.load_bundled_skills(&seen).await;
            for (name, skill) in bundled {
                seen.insert(name.clone());
                loaded_names.push(name);
                self.skills.push(skill);
            }
        }

        if loaded_names.len() >= MAX_DISCOVERED_SKILLS {
            tracing::warn!(
                "Global skill discovery cap reached ({} skills)",
                MAX_DISCOVERED_SKILLS
            );
        }

        // Post-discovery companion-skill check. `requires.skills` is advisory
        // metadata only (gating does not enforce it), so a user who drops
        // `ceo-assistant/SKILL.md` into their workspace can silently get a
        // degraded experience when its companions (`commitment-triage`,
        // `commitment-digest`, …) aren't present. Walk every loaded skill
        // and warn once per missing companion so the gap is visible in the
        // log without blocking load.
        let loaded_set: HashSet<&str> = loaded_names.iter().map(String::as_str).collect();
        for skill in &self.skills {
            for companion in &skill.manifest.requires.skills {
                if !loaded_set.contains(companion.as_str()) {
                    tracing::warn!(
                        "Skill '{}' declares companion '{}' in `requires.skills`, but it is not loaded. \
                         Install it via `skill_install` or place a SKILL.md for it in ~/.ironclaw/skills/ \
                         to avoid a degraded experience.",
                        skill.manifest.name,
                        companion
                    );
                }
            }
        }

        loaded_names
    }

    /// Dedup and absorb discovered skills into the registry.
    fn absorb(
        &mut self,
        skills: Vec<(String, LoadedSkill)>,
        seen: &mut HashSet<String>,
        loaded_names: &mut Vec<String>,
        override_source: &str,
    ) {
        for (name, skill) in skills {
            if seen.contains(&name) {
                tracing::debug!(
                    "Skipping skill '{}' (overridden by {})",
                    name,
                    override_source
                );
                continue;
            }
            seen.insert(name.clone());
            loaded_names.push(name);
            self.skills.push(skill);
        }
    }

    /// Discover skills from a single directory, recursing into bundle directories.
    ///
    /// Supports three layouts:
    /// - Flat: `dir/SKILL.md` (skill name derived from parent dir or file stem)
    /// - Subdirectory: `dir/<name>/SKILL.md`
    /// - Bundle: `dir/<bundle>/<name>/SKILL.md` (bundle has no `SKILL.md`, recursed into)
    async fn discover_from_dir<F>(
        &self,
        dir: &Path,
        trust: SkillTrust,
        make_source: &F,
        remaining_cap: usize,
        current_depth: usize,
    ) -> Vec<(String, LoadedSkill)>
    where
        F: Fn(PathBuf) -> SkillSource + Send + Sync,
    {
        let mut results = Vec::new();

        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(entries) => entries,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    tracing::debug!("Skills directory does not exist: {:?}", dir);
                } else {
                    tracing::warn!("Failed to read skills directory {:?}: {}", dir, e);
                }
                return results;
            }
        };

        let mut count = 0usize;
        while let Ok(Some(entry)) = entries.next_entry().await {
            if count >= remaining_cap {
                tracing::warn!(
                    "Skill discovery cap reached ({} skills in this scan), skipping remaining",
                    count
                );
                break;
            }

            let path = entry.path();
            let meta = match tokio::fs::symlink_metadata(&path).await {
                Ok(m) => m,
                Err(e) => {
                    tracing::debug!("Failed to stat {:?}: {}", path, e);
                    continue;
                }
            };

            if meta.is_symlink() {
                tracing::warn!(
                    "Skipping symlink in skills directory: {:?}",
                    path.file_name().unwrap_or_default()
                );
                continue;
            }

            // Case 1: Subdirectory containing SKILL.md
            if meta.is_dir() {
                let skill_md = path.join("SKILL.md");
                if tokio::fs::try_exists(&skill_md).await.unwrap_or(false) {
                    count += 1;
                    let source = make_source(path.clone());
                    match self.load_skill_md(&skill_md, trust, source).await {
                        Ok((name, skill)) => {
                            tracing::debug!("Loaded skill: {}", name);
                            results.push((name, skill));
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to load skill from {:?}: {}",
                                path.file_name().unwrap_or_default(),
                                e
                            );
                        }
                    }
                } else if current_depth < self.max_scan_depth {
                    tracing::debug!(
                        "Recursing into bundle directory {:?} (depth {})",
                        path.file_name().unwrap_or_default(),
                        current_depth + 1
                    );
                    let nested = Box::pin(self.discover_from_dir(
                        &path,
                        trust,
                        make_source,
                        remaining_cap.saturating_sub(count),
                        current_depth + 1,
                    ))
                    .await;
                    count += nested.len();
                    results.extend(nested);
                }
                continue;
            }

            // Case 2: Flat SKILL.md directly in the directory
            if meta.is_file()
                && let Some(fname) = path.file_name().and_then(|f| f.to_str())
                && fname == "SKILL.md"
            {
                count += 1;
                let source = make_source(dir.to_path_buf());
                match self.load_skill_md(&path, trust, source).await {
                    Ok((name, skill)) => {
                        tracing::debug!("Loaded skill: {}", name);
                        results.push((name, skill));
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load skill from {:?}: {}", fname, e);
                    }
                }
            }
        }

        results
    }

    /// Load a single SKILL.md file.
    async fn load_skill_md(
        &self,
        path: &Path,
        trust: SkillTrust,
        source: SkillSource,
    ) -> Result<(String, LoadedSkill), SkillRegistryError> {
        load_and_validate_skill(path, trust, source).await
    }

    /// Load bundled skills from in-memory content, skipping names already seen.
    async fn load_bundled_skills(&self, seen: &HashSet<String>) -> Vec<(String, LoadedSkill)> {
        let mut results = Vec::new();
        for (name, content) in self.bundled_content {
            if seen.contains(name) {
                tracing::debug!(
                    "Skipping bundled skill '{}' (overridden by user/workspace/installed)",
                    name
                );
                continue;
            }
            match load_from_content(
                content,
                SkillTrust::Trusted,
                SkillSource::Bundled(PathBuf::from(name)),
            )
            .await
            {
                Ok((loaded_name, skill)) => {
                    tracing::debug!("Loaded bundled skill: {}", loaded_name);
                    results.push((loaded_name, skill));
                }
                Err(e) => {
                    tracing::debug!("Skipping bundled skill '{}': {}", name, e);
                }
            }
        }
        results
    }

    /// Get all loaded skills.
    pub fn skills(&self) -> &[LoadedSkill] {
        &self.skills
    }

    /// Get the number of loaded skills.
    pub fn count(&self) -> usize {
        self.skills.len()
    }

    /// Retain only skills whose names are in the given allowlist.
    ///
    /// If `names` is empty, this is a no-op (all skills are kept).
    pub fn retain_only(&mut self, names: &[&str]) {
        if names.is_empty() {
            return;
        }
        let names_set: HashSet<&str> = names.iter().copied().collect();
        self.skills
            .retain(|s| names_set.contains(s.manifest.name.as_str()));
    }

    /// Check if a skill with the given name is loaded.
    pub fn has(&self, name: &str) -> bool {
        self.skills.iter().any(|s| s.manifest.name == name)
    }

    /// Find a skill by name.
    pub fn find_by_name(&self, name: &str) -> Option<&LoadedSkill> {
        self.skills.iter().find(|s| s.manifest.name == name)
    }

    /// Resolve the on-disk install content and final in-memory skill name.
    ///
    /// Install flows use this to recover from invalid published names (for
    /// example, catalog display names containing spaces) without relaxing the
    /// parser for ordinary local skill discovery.
    pub fn resolve_install_content(
        normalized_content: &str,
        requested_identifier: Option<&str>,
    ) -> Result<(String, String), SkillRegistryError> {
        normalize_install_content(normalized_content, requested_identifier)
    }

    /// Perform the disk I/O and loading for a skill install.
    ///
    /// This is a static method so it doesn't borrow `&self`, allowing callers
    /// to drop their registry lock before awaiting.
    pub async fn prepare_install_to_disk(
        user_dir: &Path,
        skill_name: &str,
        normalized_content: &str,
    ) -> Result<(String, LoadedSkill), SkillRegistryError> {
        let skill_dir = user_dir.join(skill_name);
        tokio::fs::create_dir_all(&skill_dir).await.map_err(|e| {
            SkillRegistryError::WriteError {
                path: skill_dir.display().to_string(),
                reason: e.to_string(),
            }
        })?;

        let skill_path = skill_dir.join("SKILL.md");
        tokio::fs::write(&skill_path, normalized_content)
            .await
            .map_err(|e| SkillRegistryError::WriteError {
                path: skill_path.display().to_string(),
                reason: e.to_string(),
            })?;

        // Load by re-reading from disk (validates round-trip)
        let source = SkillSource::User(skill_dir);
        load_and_validate_skill(&skill_path, SkillTrust::Installed, source).await
    }

    /// Commit a prepared skill into the in-memory registry.
    ///
    /// This is a fast, synchronous operation that only adds to the Vec.
    /// Call after `prepare_install` completes.
    pub fn commit_install(
        &mut self,
        name: &str,
        skill: LoadedSkill,
    ) -> Result<(), SkillRegistryError> {
        // Re-check for duplicates (another thread may have installed between prepare and commit)
        if self.has(name) {
            return Err(SkillRegistryError::AlreadyExists {
                name: name.to_string(),
            });
        }
        self.skills.push(skill);
        tracing::debug!("Installed skill: {}", name);
        Ok(())
    }

    /// Install a skill at runtime from SKILL.md content.
    ///
    /// Convenience method that parses, writes to disk, and commits in-memory.
    /// When called through tool execution where a lock is involved, prefer using
    /// `prepare_install_to_disk` + `commit_install` separately to minimize lock
    /// hold time.
    pub async fn install_skill(&mut self, content: &str) -> Result<String, SkillRegistryError> {
        let normalized = normalize_line_endings(content);
        let (skill_name, install_content) = normalize_install_content(&normalized, None)?;
        if self.has(&skill_name) {
            return Err(SkillRegistryError::AlreadyExists {
                name: skill_name.clone(),
            });
        }
        let user_dir = self.user_dir.clone();
        let (name, skill) =
            Self::prepare_install_to_disk(&user_dir, &skill_name, &install_content).await?;
        self.commit_install(&name, skill)?;
        Ok(name)
    }

    /// Validate that a skill can be removed and return its filesystem path.
    ///
    /// Performs validation without modifying state. Callers can then do async
    /// filesystem cleanup without holding the registry lock, and call
    /// `commit_remove` afterward.
    pub fn validate_remove(&self, name: &str) -> Result<PathBuf, SkillRegistryError> {
        let idx = self
            .skills
            .iter()
            .position(|s| s.manifest.name == name)
            .ok_or_else(|| SkillRegistryError::NotFound(name.to_string()))?;

        let skill = &self.skills[idx];

        match &skill.source {
            SkillSource::User(path) | SkillSource::Installed(path) => Ok(path.clone()),
            SkillSource::Workspace(_) => Err(SkillRegistryError::CannotRemove {
                name: name.to_string(),
                reason: "workspace skills cannot be removed via this interface".to_string(),
            }),
            SkillSource::Bundled(_) => Err(SkillRegistryError::CannotRemove {
                name: name.to_string(),
                reason: "bundled skills cannot be removed".to_string(),
            }),
        }
    }

    /// Remove a skill's files from disk (async I/O).
    ///
    /// Call after `validate_remove` and before `commit_remove`.
    pub async fn delete_skill_files(path: &Path) -> Result<(), SkillRegistryError> {
        let skill_md = path.join("SKILL.md");
        if tokio::fs::try_exists(&skill_md).await.unwrap_or(false) {
            tokio::fs::remove_file(&skill_md).await.map_err(|e| {
                SkillRegistryError::WriteError {
                    path: skill_md.display().to_string(),
                    reason: e.to_string(),
                }
            })?;
            // Remove the directory if empty
            let _ = tokio::fs::remove_dir(path).await;
        }
        Ok(())
    }

    /// Remove a skill from the in-memory registry.
    ///
    /// Fast synchronous operation. Call after filesystem cleanup.
    pub fn commit_remove(&mut self, name: &str) -> Result<(), SkillRegistryError> {
        let idx = self
            .skills
            .iter()
            .position(|s| s.manifest.name == name)
            .ok_or_else(|| SkillRegistryError::NotFound(name.to_string()))?;

        self.skills.remove(idx);
        tracing::debug!("Removed skill: {}", name);
        Ok(())
    }

    /// Remove a skill by name.
    ///
    /// Convenience method that combines validation, file deletion, and in-memory
    /// removal. When called through tool execution, prefer using the split
    /// validate/delete/commit methods to minimize lock hold time.
    pub async fn remove_skill(&mut self, name: &str) -> Result<(), SkillRegistryError> {
        let path = self.validate_remove(name)?;
        Self::delete_skill_files(&path).await?;
        self.commit_remove(name)
    }

    /// Clear all loaded skills and re-discover from disk.
    pub async fn reload(&mut self) -> Vec<String> {
        self.skills.clear();
        self.discover_all().await
    }

    /// Get the user skills directory path.
    pub fn user_dir(&self) -> &Path {
        &self.user_dir
    }

    /// Get the installed skills directory path, if configured.
    pub fn installed_dir(&self) -> Option<&Path> {
        self.installed_dir.as_deref()
    }

    /// Get the directory where new registry installs should be written.
    ///
    /// Returns the installed_dir if configured (preferred), otherwise falls
    /// back to user_dir. In practice, the installed_dir is always set when
    /// the app is running; the fallback exists for test registries.
    pub fn install_target_dir(&self) -> &Path {
        self.installed_dir.as_deref().unwrap_or(&self.user_dir)
    }
}

/// Load and validate a single SKILL.md file from disk.
///
/// Reads the file, checks for symlinks and size limits, then delegates to
/// `build_loaded_skill` for parsing, validation, and construction.
async fn load_and_validate_skill(
    path: &Path,
    trust: SkillTrust,
    source: SkillSource,
) -> Result<(String, LoadedSkill), SkillRegistryError> {
    // Check for symlink at the file level
    let file_meta =
        tokio::fs::symlink_metadata(path)
            .await
            .map_err(|e| SkillRegistryError::ReadError {
                path: path.display().to_string(),
                reason: e.to_string(),
            })?;

    if file_meta.is_symlink() {
        return Err(SkillRegistryError::SymlinkDetected {
            path: path.display().to_string(),
        });
    }

    // Read and check size
    let raw_bytes = tokio::fs::read(path)
        .await
        .map_err(|e| SkillRegistryError::ReadError {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;

    if raw_bytes.len() as u64 > MAX_PROMPT_FILE_SIZE {
        return Err(SkillRegistryError::FileTooLarge {
            name: path.display().to_string(),
            size: raw_bytes.len() as u64,
            max: MAX_PROMPT_FILE_SIZE,
        });
    }

    let raw_content = String::from_utf8(raw_bytes).map_err(|e| SkillRegistryError::ReadError {
        path: path.display().to_string(),
        reason: format!("Invalid UTF-8: {}", e),
    })?;

    let normalized_content = normalize_line_endings(&raw_content);
    let error_label = path.display().to_string();

    build_loaded_skill(&normalized_content, &error_label, trust, source).await
}

/// Load and validate a skill from in-memory content (no disk I/O).
///
/// Used for bundled skills compiled into the binary.
async fn load_from_content(
    raw_content: &str,
    trust: SkillTrust,
    source: SkillSource,
) -> Result<(String, LoadedSkill), SkillRegistryError> {
    if raw_content.len() as u64 > MAX_PROMPT_FILE_SIZE {
        return Err(SkillRegistryError::FileTooLarge {
            name: "(bundled)".to_string(),
            size: raw_content.len() as u64,
            max: MAX_PROMPT_FILE_SIZE,
        });
    }

    let normalized_content = normalize_line_endings(raw_content);

    build_loaded_skill(&normalized_content, "(bundled)", trust, source).await
}

/// Parse, validate, gate-check, and construct a `LoadedSkill` from normalized content.
///
/// Shared implementation used by both `load_and_validate_skill` (disk) and
/// `load_from_content` (in-memory). The `error_label` is used in error messages
/// to identify the source (file path or "(bundled)").
async fn build_loaded_skill(
    normalized_content: &str,
    error_label: &str,
    trust: SkillTrust,
    source: SkillSource,
) -> Result<(String, LoadedSkill), SkillRegistryError> {
    let parsed = parse_skill_md(normalized_content).map_err(|e: SkillParseError| match e {
        SkillParseError::InvalidName { ref name } => SkillRegistryError::ParseError {
            name: name.clone(),
            reason: e.to_string(),
        },
        _ => SkillRegistryError::ParseError {
            name: error_label.to_string(),
            reason: e.to_string(),
        },
    })?;

    let manifest = parsed.manifest;
    let prompt_content = parsed.prompt_content;

    // Check gating requirements
    {
        let result = gating::check_requirements(&manifest.requires).await;
        if !result.passed {
            return Err(SkillRegistryError::GatingFailed {
                name: manifest.name.clone(),
                reason: result.failures.join("; "),
            });
        }
    }

    // Check token budget (reject if prompt is > 2x declared budget)
    // ~4 bytes per token for English prose = ~0.25 tokens per byte
    let approx_tokens = (prompt_content.len() as f64 * 0.25) as usize;
    let declared = manifest.activation.max_context_tokens;
    if declared > 0 && approx_tokens > declared * 2 {
        return Err(SkillRegistryError::TokenBudgetExceeded {
            name: manifest.name.clone(),
            approx_tokens,
            declared,
        });
    }

    let content_hash = compute_hash(&prompt_content);
    let compiled_patterns = LoadedSkill::compile_patterns(&manifest.activation.patterns);
    let lowercased_keywords = to_lowercase_vec(&manifest.activation.keywords);
    let lowercased_exclude_keywords = to_lowercase_vec(&manifest.activation.exclude_keywords);
    let lowercased_tags = to_lowercase_vec(&manifest.activation.tags);

    let name = manifest.name.clone();
    let skill = LoadedSkill {
        manifest,
        prompt_content,
        trust,
        source,
        content_hash,
        compiled_patterns,
        lowercased_keywords,
        lowercased_exclude_keywords,
        lowercased_tags,
    };

    Ok((name, skill))
}

/// Compute SHA-256 hash of content in the format "sha256:hex...".
pub fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    format!("sha256:{}", hex::encode(result))
}

/// Helper to check gating for a `GatingRequirements`. Useful for callers that
/// don't have the full skill loaded yet.
pub async fn check_gating(requirements: &GatingRequirements) -> crate::gating::GatingResult {
    gating::check_requirements(requirements).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn test_discover_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_all().await;
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_discover_nonexistent_dir() {
        let mut registry = SkillRegistry::new(PathBuf::from("/nonexistent/skills"));
        let loaded = registry.discover_all().await;
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_load_subdirectory_layout() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("test-skill");
        fs::create_dir(&skill_dir).unwrap();

        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: test-skill\ndescription: A test skill\nactivation:\n  keywords: [\"test\"]\n---\n\nYou are a helpful test assistant.\n",
        ).unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_all().await;

        assert_eq!(loaded, vec!["test-skill"]);
        assert_eq!(registry.count(), 1);

        let skill = &registry.skills()[0];
        assert_eq!(skill.trust, SkillTrust::Trusted);
        assert!(skill.prompt_content.contains("helpful test assistant"));
    }

    #[tokio::test]
    async fn test_workspace_overrides_user() {
        let user_dir = tempfile::tempdir().unwrap();
        let ws_dir = tempfile::tempdir().unwrap();

        // Create skill in user dir
        let user_skill = user_dir.path().join("my-skill");
        fs::create_dir(&user_skill).unwrap();
        fs::write(
            user_skill.join("SKILL.md"),
            "---\nname: my-skill\n---\n\nUser version.\n",
        )
        .unwrap();

        // Create same-named skill in workspace dir
        let ws_skill = ws_dir.path().join("my-skill");
        fs::create_dir(&ws_skill).unwrap();
        fs::write(
            ws_skill.join("SKILL.md"),
            "---\nname: my-skill\n---\n\nWorkspace version.\n",
        )
        .unwrap();

        let mut registry = SkillRegistry::new(user_dir.path().to_path_buf())
            .with_workspace_dir(ws_dir.path().to_path_buf());
        let loaded = registry.discover_all().await;

        assert_eq!(loaded, vec!["my-skill"]);
        assert_eq!(registry.count(), 1);
        assert!(registry.skills()[0].prompt_content.contains("Workspace"));
    }

    #[tokio::test]
    async fn test_gating_failure_skips_skill() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("gated-skill");
        fs::create_dir(&skill_dir).unwrap();

        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: gated-skill\nrequires:\n  bins: [\"__nonexistent_bin__\"]\n---\n\nGated prompt.\n",
        ).unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_all().await;
        assert!(loaded.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_symlink_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let real_dir = dir.path().join("real-skill");
        fs::create_dir(&real_dir).unwrap();
        fs::write(
            real_dir.join("SKILL.md"),
            "---\nname: real-skill\n---\n\nTest.\n",
        )
        .unwrap();

        let skills_dir = dir.path().join("skills");
        fs::create_dir(&skills_dir).unwrap();
        std::os::unix::fs::symlink(&real_dir, skills_dir.join("linked-skill")).unwrap();

        let mut registry = SkillRegistry::new(skills_dir);
        let loaded = registry.discover_all().await;
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_file_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("big-skill");
        fs::create_dir(&skill_dir).unwrap();

        let big_content = format!(
            "---\nname: big-skill\n---\n\n{}",
            "x".repeat((MAX_PROMPT_FILE_SIZE + 1) as usize)
        );
        fs::write(skill_dir.join("SKILL.md"), &big_content).unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_all().await;
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_invalid_skill_md_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("bad-skill");
        fs::create_dir(&skill_dir).unwrap();

        // Missing frontmatter
        fs::write(skill_dir.join("SKILL.md"), "Just plain text").unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_all().await;
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_line_ending_normalization() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("crlf-skill");
        fs::create_dir(&skill_dir).unwrap();

        fs::write(
            skill_dir.join("SKILL.md"),
            "---\r\nname: crlf-skill\r\n---\r\n\r\nline1\r\nline2\r\n",
        )
        .unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.discover_all().await;

        assert_eq!(registry.count(), 1);
        let skill = &registry.skills()[0];
        assert_eq!(skill.prompt_content, "line1\nline2\n");
    }

    #[tokio::test]
    async fn test_token_budget_rejection() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("big-prompt");
        fs::create_dir(&skill_dir).unwrap();

        let big_prompt = "word ".repeat(4000);
        let content = format!(
            "---\nname: big-prompt\nactivation:\n  max_context_tokens: 100\n---\n\n{}",
            big_prompt
        );
        fs::write(skill_dir.join("SKILL.md"), &content).unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_all().await;
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn test_has_and_find_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("my-skill");
        fs::create_dir(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\n---\n\nPrompt.\n",
        )
        .unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.discover_all().await;

        assert!(registry.has("my-skill"));
        assert!(!registry.has("nonexistent"));
        assert!(registry.find_by_name("my-skill").is_some());
        assert!(registry.find_by_name("nonexistent").is_none());
    }

    #[tokio::test]
    async fn test_install_skill_from_content() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = SkillRegistry::new(dir.path().to_path_buf());

        let content =
            "---\nname: test-install\ndescription: Installed skill\n---\n\nInstalled prompt.\n";
        let name = registry.install_skill(content).await.unwrap();

        assert_eq!(name, "test-install");
        assert!(registry.has("test-install"));
        assert_eq!(registry.count(), 1);

        // Verify file was written to disk
        let skill_path = dir.path().join("test-install").join("SKILL.md");
        assert!(skill_path.exists());
    }

    #[test]
    fn test_resolve_install_content_prefers_requested_slug_for_invalid_name() {
        let content = "---\nname: Mortgage Calculator\ndescription: Installed skill\n---\n\nInstalled prompt.\n";

        let (name, rewritten) =
            SkillRegistry::resolve_install_content(content, Some("finance/mortgage-calculator"))
                .unwrap();

        assert_eq!(name, "finance-mortgage-calculator");
        assert!(rewritten.contains("name: finance-mortgage-calculator"));
        assert!(rewritten.contains("Installed prompt."));
    }

    #[test]
    fn test_resolve_install_content_slugifies_invalid_name_without_slug() {
        let content = "---\nname: Mortgage Calculator\n---\n\nPrompt.\n";

        let (name, rewritten) = SkillRegistry::resolve_install_content(content, None).unwrap();

        assert_eq!(name, "mortgage-calculator");
        assert!(rewritten.contains("name: mortgage-calculator"));
    }

    #[tokio::test]
    async fn test_install_skill_normalizes_invalid_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = SkillRegistry::new(dir.path().to_path_buf());

        let content = "---\nname: Mortgage Calculator\ndescription: Installed skill\n---\n\nInstalled prompt.\n";
        let name = registry.install_skill(content).await.unwrap();

        assert_eq!(name, "mortgage-calculator");
        assert!(registry.has("mortgage-calculator"));

        let skill_path = dir.path().join("mortgage-calculator").join("SKILL.md");
        assert!(skill_path.exists());

        let written = fs::read_to_string(skill_path).unwrap();
        assert!(written.contains("name: mortgage-calculator"));
    }

    #[test]
    fn test_resolve_install_content_preserves_unknown_frontmatter_fields() {
        // Published manifests may carry custom keys (vendor extensions, future
        // fields) that the typed `SkillManifest` does not know about. Recovery
        // must rewrite only `name` without dropping unknown keys.
        let content = "---\nname: Mortgage Calculator\ndescription: Computes payments\nx-publisher: acme\ncustom_meta:\n  rating: 5\n  tags:\n    - finance\n    - calculator\n---\n\nInstalled prompt.\n";

        let (name, rewritten) = SkillRegistry::resolve_install_content(content, None).unwrap();

        assert_eq!(name, "mortgage-calculator");
        assert!(rewritten.contains("name: mortgage-calculator"));
        assert!(
            rewritten.contains("x-publisher: acme"),
            "unknown top-level key was dropped: {rewritten}"
        );
        assert!(
            rewritten.contains("custom_meta:"),
            "unknown nested mapping was dropped: {rewritten}"
        );
        assert!(
            rewritten.contains("rating: 5"),
            "nested scalar was dropped: {rewritten}"
        );
        assert!(
            rewritten.contains("- finance") && rewritten.contains("- calculator"),
            "nested sequence was dropped: {rewritten}"
        );
        assert!(rewritten.contains("Installed prompt."));
    }

    #[test]
    fn test_resolve_install_content_preserves_owner_for_invalid_slug_name() {
        let content = "---\nname: Mortgage Calculator\n---\n\nPrompt.\n";

        let (name, rewritten) =
            SkillRegistry::resolve_install_content(content, Some("alice/mortgage-calculator"))
                .unwrap();

        assert_eq!(name, "alice-mortgage-calculator");
        assert!(rewritten.contains("name: alice-mortgage-calculator"));
    }

    #[tokio::test]
    async fn test_install_duplicate_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = SkillRegistry::new(dir.path().to_path_buf());

        let content = "---\nname: dup-skill\n---\n\nPrompt.\n";
        registry.install_skill(content).await.unwrap();

        let result = registry.install_skill(content).await;
        assert!(matches!(
            result,
            Err(SkillRegistryError::AlreadyExists { .. })
        ));
    }

    #[tokio::test]
    async fn test_remove_user_skill() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = SkillRegistry::new(dir.path().to_path_buf());

        let content = "---\nname: removable\n---\n\nPrompt.\n";
        registry.install_skill(content).await.unwrap();
        assert!(registry.has("removable"));

        registry.remove_skill("removable").await.unwrap();
        assert!(!registry.has("removable"));
        assert_eq!(registry.count(), 0);
    }

    #[tokio::test]
    async fn test_remove_workspace_skill_rejected() {
        let user_dir = tempfile::tempdir().unwrap();
        let ws_dir = tempfile::tempdir().unwrap();

        let ws_skill = ws_dir.path().join("ws-skill");
        fs::create_dir(&ws_skill).unwrap();
        fs::write(
            ws_skill.join("SKILL.md"),
            "---\nname: ws-skill\n---\n\nWorkspace prompt.\n",
        )
        .unwrap();

        let mut registry = SkillRegistry::new(user_dir.path().to_path_buf())
            .with_workspace_dir(ws_dir.path().to_path_buf());
        registry.discover_all().await;

        let result = registry.remove_skill("ws-skill").await;
        assert!(matches!(
            result,
            Err(SkillRegistryError::CannotRemove { .. })
        ));
    }

    #[tokio::test]
    async fn test_remove_nonexistent_fails() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = SkillRegistry::new(dir.path().to_path_buf());

        let result = registry.remove_skill("nonexistent").await;
        assert!(matches!(result, Err(SkillRegistryError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_reload_clears_and_rediscovers() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("persist-skill");
        fs::create_dir(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: persist-skill\n---\n\nPrompt.\n",
        )
        .unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.discover_all().await;
        assert_eq!(registry.count(), 1);

        let loaded = registry.reload().await;
        assert_eq!(loaded, vec!["persist-skill"]);
        assert_eq!(registry.count(), 1);
    }

    #[tokio::test]
    async fn test_load_flat_layout() {
        let dir = tempfile::tempdir().unwrap();

        // Place a SKILL.md directly in the skills directory (flat layout)
        fs::write(
            dir.path().join("SKILL.md"),
            "---\nname: flat-skill\ndescription: A flat layout skill\nactivation:\n  keywords: [\"flat\"]\n---\n\nYou are a flat layout test skill.\n",
        ).unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_all().await;

        assert_eq!(loaded, vec!["flat-skill"]);
        assert_eq!(registry.count(), 1);

        let skill = &registry.skills()[0];
        assert_eq!(skill.trust, SkillTrust::Trusted);
        assert!(skill.prompt_content.contains("flat layout test skill"));
    }

    #[tokio::test]
    async fn test_mixed_flat_and_subdirectory_layout() {
        let dir = tempfile::tempdir().unwrap();

        // Flat layout: SKILL.md directly in the skills directory
        fs::write(
            dir.path().join("SKILL.md"),
            "---\nname: flat-skill\n---\n\nFlat prompt.\n",
        )
        .unwrap();

        // Subdirectory layout: <name>/SKILL.md
        let sub_dir = dir.path().join("sub-skill");
        fs::create_dir(&sub_dir).unwrap();
        fs::write(
            sub_dir.join("SKILL.md"),
            "---\nname: sub-skill\n---\n\nSub prompt.\n",
        )
        .unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_all().await;

        assert_eq!(registry.count(), 2);
        assert!(loaded.contains(&"flat-skill".to_string()));
        assert!(loaded.contains(&"sub-skill".to_string()));
    }

    #[tokio::test]
    async fn test_lowercased_fields_populated() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("case-skill");
        fs::create_dir(&skill_dir).unwrap();

        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: case-skill\nactivation:\n  keywords: [\"Write\", \"EDIT\"]\n  tags: [\"Email\", \"PROSE\"]\n---\n\nTest prompt.\n",
        ).unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.discover_all().await;

        let skill = registry.find_by_name("case-skill").unwrap();
        assert_eq!(skill.lowercased_keywords, vec!["write", "edit"]);
        assert_eq!(skill.lowercased_tags, vec!["email", "prose"]);
    }

    #[tokio::test]
    async fn test_retain_only_empty_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("SKILL.md"),
            "---\nname: keep-me\ndescription: test\nactivation:\n  keywords: [\"test\"]\n---\n\nKeep this skill.\n",
        )
        .unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.discover_all().await;
        assert_eq!(registry.count(), 1);

        registry.retain_only(&[]);
        assert_eq!(
            registry.count(),
            1,
            "empty retain_only should keep all skills"
        );
    }

    #[test]
    fn test_compute_hash_deterministic() {
        let h1 = compute_hash("hello world");
        let h2 = compute_hash("hello world");
        assert_eq!(h1, h2);
        assert!(h1.starts_with("sha256:"));
    }

    #[test]
    fn test_compute_hash_different_content() {
        let h1 = compute_hash("hello");
        let h2 = compute_hash("world");
        assert_ne!(h1, h2);
    }

    /// Skills in the installed_dir are discovered with SkillTrust::Installed,
    /// not Trusted. This ensures registry-installed skills do not gain full
    /// tool access after an agent restart.
    #[tokio::test]
    async fn test_installed_dir_uses_installed_trust() {
        let user_dir = tempfile::tempdir().unwrap();
        let inst_dir = tempfile::tempdir().unwrap();

        // Place a skill in the installed dir
        let skill_dir = inst_dir.path().join("registry-skill");
        fs::create_dir(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: registry-skill\nversion: \"1.2.3\"\n---\n\nInstalled prompt.\n",
        )
        .unwrap();

        let mut registry = SkillRegistry::new(user_dir.path().to_path_buf())
            .with_installed_dir(inst_dir.path().to_path_buf());
        let loaded = registry.discover_all().await;

        assert_eq!(loaded, vec!["registry-skill"]);
        let skill = registry.find_by_name("registry-skill").unwrap();
        assert_eq!(
            skill.trust,
            SkillTrust::Installed,
            "installed_dir skills must be Installed"
        );
        assert_eq!(skill.manifest.version, "1.2.3");
    }

    /// install_target_dir() returns installed_dir when set, user_dir otherwise.
    #[test]
    fn test_install_target_dir_prefers_installed_dir() {
        let user_dir = PathBuf::from("/tmp/user-skills");
        let inst_dir = PathBuf::from("/tmp/installed-skills");

        let registry = SkillRegistry::new(user_dir.clone()).with_installed_dir(inst_dir.clone());
        assert_eq!(registry.install_target_dir(), inst_dir.as_path());

        let registry_no_inst = SkillRegistry::new(user_dir.clone());
        assert_eq!(registry_no_inst.install_target_dir(), user_dir.as_path());
    }

    /// User skills (user_dir) remain Trusted even when installed_dir is set.
    #[tokio::test]
    async fn test_user_dir_stays_trusted_with_installed_dir() {
        let user_dir = tempfile::tempdir().unwrap();
        let inst_dir = tempfile::tempdir().unwrap();

        let skill_dir = user_dir.path().join("my-skill");
        fs::create_dir(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\n---\n\nUser prompt.\n",
        )
        .unwrap();

        let mut registry = SkillRegistry::new(user_dir.path().to_path_buf())
            .with_installed_dir(inst_dir.path().to_path_buf());
        registry.discover_all().await;

        let skill = registry.find_by_name("my-skill").unwrap();
        assert_eq!(skill.trust, SkillTrust::Trusted);
    }

    #[tokio::test]
    async fn test_bundled_skills_loaded() {
        let dir = tempfile::tempdir().unwrap();

        // Leak the vec so we get a &'static slice
        let bundled: &'static [(String, String)] = Box::leak(Box::new(vec![(
            "bundled-skill".to_string(),
            "---\nname: bundled-skill\ndescription: A bundled test\nactivation:\n  keywords: [\"test\"]\n---\n\nBundled prompt.\n".to_string(),
        )]));

        let mut registry =
            SkillRegistry::new(dir.path().to_path_buf()).with_bundled_content(bundled);
        let loaded = registry.discover_all().await;

        assert_eq!(loaded, vec!["bundled-skill"]);
        assert_eq!(registry.count(), 1);

        let skill = registry.find_by_name("bundled-skill").unwrap();
        assert_eq!(skill.trust, SkillTrust::Trusted);
        assert!(matches!(skill.source, SkillSource::Bundled(_)));
        assert!(skill.prompt_content.contains("Bundled prompt."));
    }

    #[tokio::test]
    async fn test_bundled_skill_overridden_by_user() {
        let user_dir = tempfile::tempdir().unwrap();

        // User skill
        let skill_dir = user_dir.path().join("my-skill");
        fs::create_dir(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\n---\n\nUser version.\n",
        )
        .unwrap();

        // Bundled skill with same name
        let bundled: &'static [(String, String)] = Box::leak(Box::new(vec![(
            "my-skill".to_string(),
            "---\nname: my-skill\n---\n\nBundled version.\n".to_string(),
        )]));

        let mut registry =
            SkillRegistry::new(user_dir.path().to_path_buf()).with_bundled_content(bundled);
        let loaded = registry.discover_all().await;

        assert_eq!(loaded, vec!["my-skill"]);
        assert_eq!(registry.count(), 1);
        // User version wins over bundled
        assert!(
            registry.skills()[0]
                .prompt_content
                .contains("User version.")
        );
    }

    #[tokio::test]
    async fn test_bundled_skill_gating_failure_skipped() {
        let dir = tempfile::tempdir().unwrap();

        let bundled: &'static [(String, String)] = Box::leak(Box::new(vec![(
            "gated".to_string(),
            "---\nname: gated\nrequires:\n  bins: [\"__nonexistent__\"]\n---\n\nGated.\n"
                .to_string(),
        )]));

        let mut registry =
            SkillRegistry::new(dir.path().to_path_buf()).with_bundled_content(bundled);
        let loaded = registry.discover_all().await;

        assert!(loaded.is_empty(), "gated bundled skill should be skipped");
    }

    #[tokio::test]
    async fn test_bundled_skill_cannot_be_removed() {
        let dir = tempfile::tempdir().unwrap();

        let bundled: &'static [(String, String)] = Box::leak(Box::new(vec![(
            "permanent".to_string(),
            "---\nname: permanent\n---\n\nCannot remove.\n".to_string(),
        )]));

        let mut registry =
            SkillRegistry::new(dir.path().to_path_buf()).with_bundled_content(bundled);
        registry.discover_all().await;

        let result = registry.remove_skill("permanent").await;
        assert!(matches!(
            result,
            Err(SkillRegistryError::CannotRemove { .. })
        ));
    }

    #[tokio::test]
    async fn test_discover_nested_bundle_directory() {
        let dir = tempfile::tempdir().unwrap();

        // Bundle directory (no SKILL.md)
        let bundle = dir.path().join("my-org");
        fs::create_dir(&bundle).unwrap();

        // Two skills inside the bundle
        let skill_a = bundle.join("skill-a");
        fs::create_dir(&skill_a).unwrap();
        fs::write(
            skill_a.join("SKILL.md"),
            "---\nname: skill-a\n---\n\nSkill A prompt.\n",
        )
        .unwrap();

        let skill_b = bundle.join("skill-b");
        fs::create_dir(&skill_b).unwrap();
        fs::write(
            skill_b.join("SKILL.md"),
            "---\nname: skill-b\n---\n\nSkill B prompt.\n",
        )
        .unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_all().await;

        assert_eq!(registry.count(), 2);
        assert!(loaded.contains(&"skill-a".to_string()));
        assert!(loaded.contains(&"skill-b".to_string()));
    }

    #[tokio::test]
    async fn test_discover_respects_depth_limit() {
        let dir = tempfile::tempdir().unwrap();

        // Create skill nested 3 levels deep (a/b/c/deep-skill/SKILL.md)
        let nested = dir.path().join("a").join("b").join("c").join("deep-skill");
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            nested.join("SKILL.md"),
            "---\nname: deep-skill\n---\n\nDeep prompt.\n",
        )
        .unwrap();

        // Depth 2 should NOT find it (3 intermediate dirs: a, b, c)
        let mut registry = SkillRegistry::new(dir.path().to_path_buf()).with_max_scan_depth(2);
        let loaded = registry.discover_all().await;
        assert!(loaded.is_empty(), "depth=2 should not reach 3 levels deep");

        // Depth 3 SHOULD find it
        let mut registry = SkillRegistry::new(dir.path().to_path_buf()).with_max_scan_depth(3);
        let loaded = registry.discover_all().await;
        assert_eq!(loaded, vec!["deep-skill"]);
    }

    #[tokio::test]
    async fn test_discover_cap_spans_recursive_levels() {
        let dir = tempfile::tempdir().unwrap();

        // Spread skills across two bundle directories so the cap must be
        // shared across separate recursive calls (not just within one).
        // Each bundle has 60 skills; with a global cap of 100, the second
        // bundle should be cut short.
        for bundle_name in &["bundle-a", "bundle-b"] {
            let bundle = dir.path().join(bundle_name);
            fs::create_dir(&bundle).unwrap();
            for i in 0..60 {
                let skill_dir = bundle.join(format!("{}-skill-{:02}", bundle_name, i));
                fs::create_dir(&skill_dir).unwrap();
                fs::write(
                    skill_dir.join("SKILL.md"),
                    format!(
                        "---\nname: {}-skill-{:02}\n---\n\nPrompt.\n",
                        bundle_name, i
                    ),
                )
                .unwrap();
            }
        }

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.discover_all().await;

        assert!(
            registry.count() <= MAX_DISCOVERED_SKILLS,
            "global cap should limit total to {} but got {}",
            MAX_DISCOVERED_SKILLS,
            registry.count()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_symlink_rejected_in_nested_directory() {
        let dir = tempfile::tempdir().unwrap();

        // Real skill outside the bundle
        let real_dir = dir.path().join("real-skill");
        fs::create_dir(&real_dir).unwrap();
        fs::write(
            real_dir.join("SKILL.md"),
            "---\nname: real-skill\n---\n\nReal prompt.\n",
        )
        .unwrap();

        // Bundle directory with a symlink inside
        let bundle = dir.path().join("bundle");
        fs::create_dir(&bundle).unwrap();
        std::os::unix::fs::symlink(&real_dir, bundle.join("linked-skill")).unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_all().await;

        // The real skill at top level is found, but the symlinked one inside bundle is rejected
        assert_eq!(loaded, vec!["real-skill"]);
        assert_eq!(registry.count(), 1);
    }

    #[tokio::test]
    async fn test_discover_nested_plus_direct() {
        let dir = tempfile::tempdir().unwrap();

        // Direct skill at depth 1
        let direct = dir.path().join("direct-skill");
        fs::create_dir(&direct).unwrap();
        fs::write(
            direct.join("SKILL.md"),
            "---\nname: direct-skill\n---\n\nDirect prompt.\n",
        )
        .unwrap();

        // Bundle with nested skill
        let bundle = dir.path().join("bundle");
        fs::create_dir(&bundle).unwrap();
        let nested = bundle.join("nested-skill");
        fs::create_dir(&nested).unwrap();
        fs::write(
            nested.join("SKILL.md"),
            "---\nname: nested-skill\n---\n\nNested prompt.\n",
        )
        .unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_all().await;

        assert_eq!(registry.count(), 2);
        assert!(loaded.contains(&"direct-skill".to_string()));
        assert!(loaded.contains(&"nested-skill".to_string()));
    }

    #[tokio::test]
    async fn test_discover_dedup_direct_vs_bundle_same_name() {
        let dir = tempfile::tempdir().unwrap();

        // Direct skill at depth 1
        let direct = dir.path().join("my-skill");
        fs::create_dir(&direct).unwrap();
        fs::write(
            direct.join("SKILL.md"),
            "---\nname: my-skill\n---\n\nDirect version.\n",
        )
        .unwrap();

        // Bundle directory containing a skill with the same name
        let bundle = dir.path().join("org-bundle");
        fs::create_dir(&bundle).unwrap();
        let nested = bundle.join("my-skill");
        fs::create_dir(&nested).unwrap();
        fs::write(
            nested.join("SKILL.md"),
            "---\nname: my-skill\n---\n\nBundle version.\n",
        )
        .unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let loaded = registry.discover_all().await;

        // Only one instance should survive dedup
        assert_eq!(registry.count(), 1);
        assert_eq!(loaded, vec!["my-skill"]);
    }

    #[tokio::test]
    async fn test_global_cap_shared_across_sources() {
        // The global cap (MAX_DISCOVERED_SKILLS=100) is shared across all
        // sources. Workspace skills are discovered first, consuming part of
        // the budget, leaving less for user skills.
        let user_dir = tempfile::tempdir().unwrap();
        let ws_dir = tempfile::tempdir().unwrap();

        // 10 workspace skills (discovered first, highest priority)
        for i in 0..10 {
            let skill_dir = ws_dir.path().join(format!("ws-skill-{:02}", i));
            fs::create_dir(&skill_dir).unwrap();
            fs::write(
                skill_dir.join("SKILL.md"),
                format!("---\nname: ws-skill-{:02}\n---\n\nPrompt.\n", i),
            )
            .unwrap();
        }

        // 120 user skills (more than the remaining budget of 90)
        for i in 0..120 {
            let skill_dir = user_dir.path().join(format!("user-skill-{:03}", i));
            fs::create_dir(&skill_dir).unwrap();
            fs::write(
                skill_dir.join("SKILL.md"),
                format!("---\nname: user-skill-{:03}\n---\n\nPrompt.\n", i),
            )
            .unwrap();
        }

        let mut registry = SkillRegistry::new(user_dir.path().to_path_buf())
            .with_workspace_dir(ws_dir.path().to_path_buf());
        registry.discover_all().await;

        // Total capped at 100 globally
        assert_eq!(registry.count(), MAX_DISCOVERED_SKILLS);

        // All 10 workspace skills must be present (discovered first)
        for i in 0..10 {
            assert!(
                registry
                    .find_by_name(&format!("ws-skill-{:02}", i))
                    .is_some(),
                "workspace skill ws-skill-{:02} should be discoverable",
                i
            );
        }
    }
}
