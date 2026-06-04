//! Read/write authority for the user-overlay LLM provider catalog,
//! `$IRONCLAW_REBORN_HOME/providers.json`.
//!
//! The merged catalog (compiled-in built-ins + this overlay) is *read*
//! through `ironclaw_llm::ProviderRegistry`. This module owns the *write*
//! side the webui2 settings surface needs: adding, editing, and removing
//! the operator's custom provider definitions. Built-in providers are never
//! stored here — an overlay entry whose `id` matches a built-in simply
//! overrides it, because `ProviderRegistry::new` resolves later entries last.
//!
//! Writes are atomic (temp file + rename) and guarded by the same exclusive
//! `.lock` sidecar discipline `ironclaw_reborn_config` uses for `config.toml`,
//! so concurrent CLI / webui edits cannot interleave.
//!
//! API-key *values* never live in this file — the catalog rejects inline
//! secrets. Keys are stored separately in the scoped secret store and
//! injected into the resolved `LlmConfig` at provider-build time.

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
};

use ironclaw_llm::registry::ProviderDefinition;
use thiserror::Error;

/// Owns the user-overlay providers file and its write discipline.
#[derive(Debug, Clone)]
pub struct ProviderRepo {
    path: PathBuf,
}

impl ProviderRepo {
    /// Construct a repo over an explicit overlay path (typically
    /// `$IRONCLAW_REBORN_HOME/providers.json`).
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The overlay file path this repo manages.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the user-overlay provider definitions.
    ///
    /// Returns an empty list when the overlay file is absent. A malformed
    /// overlay is a hard error — a caller about to rewrite the file must not
    /// silently drop the operator's other custom providers.
    pub fn load(&self) -> Result<Vec<ProviderDefinition>, ProviderRepoError> {
        match fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).map_err(|source| ProviderRepoError::Parse {
                path: self.path.clone(),
                source,
            }),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(source) => Err(ProviderRepoError::Read {
                path: self.path.clone(),
                source,
            }),
        }
    }

    /// Async wrapper for [`Self::load`]. Keeps filesystem work off Tokio
    /// worker threads when called from HTTP handlers.
    pub async fn load_async(&self) -> Result<Vec<ProviderDefinition>, ProviderRepoError> {
        let repo = self.clone();
        tokio::task::spawn_blocking(move || repo.load())
            .await
            .map_err(|source| ProviderRepoError::Task {
                reason: source.to_string(),
            })?
    }

    /// Insert or replace a custom provider definition by `id`, atomically.
    ///
    /// Returns `true` when an existing entry with the same id was replaced,
    /// `false` when the definition was appended.
    pub fn upsert(&self, definition: ProviderDefinition) -> Result<bool, ProviderRepoError> {
        let _lock = self.acquire_lock()?;
        let mut overlay = self.load()?;
        let replaced = if let Some(slot) = overlay
            .iter_mut()
            .find(|existing| existing.id.eq_ignore_ascii_case(&definition.id))
        {
            *slot = definition;
            true
        } else {
            overlay.push(definition);
            false
        };
        self.write_overlay(&overlay)?;
        Ok(replaced)
    }

    /// Async wrapper for [`Self::upsert`]. The lock/read/write section is
    /// synchronous by design, so run it on the blocking pool.
    pub async fn upsert_async(
        &self,
        definition: ProviderDefinition,
    ) -> Result<bool, ProviderRepoError> {
        let repo = self.clone();
        tokio::task::spawn_blocking(move || repo.upsert(definition))
            .await
            .map_err(|source| ProviderRepoError::Task {
                reason: source.to_string(),
            })?
    }

    /// Remove a custom provider definition by `id`, atomically.
    ///
    /// Returns whether an entry was removed. Built-ins are never stored in
    /// the overlay, so removing a built-in id is a no-op that returns
    /// `false`; the caller decides whether that is an error.
    pub fn delete(&self, id: &str) -> Result<bool, ProviderRepoError> {
        let _lock = self.acquire_lock()?;
        let mut overlay = self.load()?;
        let before = overlay.len();
        overlay.retain(|existing| !existing.id.eq_ignore_ascii_case(id));
        let removed = overlay.len() != before;
        if removed {
            self.write_overlay(&overlay)?;
        }
        Ok(removed)
    }

    /// Async wrapper for [`Self::delete`]. Keeps the exclusive filesystem lock
    /// out of the async worker thread.
    pub async fn delete_async(&self, id: &str) -> Result<bool, ProviderRepoError> {
        let repo = self.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || repo.delete(&id))
            .await
            .map_err(|source| ProviderRepoError::Task {
                reason: source.to_string(),
            })?
    }

    fn write_overlay(&self, overlay: &[ProviderDefinition]) -> Result<(), ProviderRepoError> {
        let text = serde_json::to_string_pretty(overlay).map_err(|source| {
            ProviderRepoError::Serialize {
                path: self.path.clone(),
                source,
            }
        })?;

        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|source| ProviderRepoError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
        let mut tmp =
            tempfile::NamedTempFile::new_in(parent).map_err(|source| ProviderRepoError::Write {
                path: self.path.clone(),
                source,
            })?;
        tmp.write_all(text.as_bytes())
            .map_err(|source| ProviderRepoError::Write {
                path: tmp.path().to_path_buf(),
                source,
            })?;
        tmp.persist(&self.path)
            .map_err(|error| ProviderRepoError::Write {
                path: self.path.clone(),
                source: error.error,
            })?;
        Ok(())
    }

    fn acquire_lock(&self) -> Result<fs::File, ProviderRepoError> {
        use fs4::FileExt as _;

        let lock_path = lock_path_for(&self.path);
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent).map_err(|source| ProviderRepoError::Lock {
                path: lock_path.clone(),
                source,
            })?;
        }
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|source| ProviderRepoError::Lock {
                path: lock_path.clone(),
                source,
            })?;
        file.lock_exclusive()
            .map_err(|source| ProviderRepoError::Lock {
                path: lock_path,
                source,
            })?;
        Ok(file)
    }
}

fn lock_path_for(path: &Path) -> PathBuf {
    let Some(file_name) = path.file_name() else {
        return path.with_extension("lock");
    };
    let mut lock_name = file_name.to_os_string();
    lock_name.push(".lock");
    path.with_file_name(lock_name)
}

/// Errors surfaced when reading or rewriting the provider overlay.
#[derive(Debug, Error)]
pub enum ProviderRepoError {
    #[error("could not read provider overlay `{}`: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not parse provider overlay `{}` as JSON: {source}", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("could not serialize provider overlay `{}`: {source}", path.display())]
    Serialize {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("could not lock provider overlay `{}`: {source}", path.display())]
    Lock {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not write provider overlay `{}`: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("provider overlay blocking task failed: {reason}")]
    Task { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_llm::registry::ProviderProtocol;

    fn custom_provider(id: &str, model: &str) -> ProviderDefinition {
        ProviderDefinition {
            id: id.to_string(),
            aliases: Vec::new(),
            protocol: ProviderProtocol::OpenAiCompletions,
            default_base_url: Some("https://api.example.test/v1".to_string()),
            base_url_env: None,
            base_url_required: false,
            api_key_env: Some("EXAMPLE_API_KEY".to_string()),
            api_key_required: true,
            model_env: "EXAMPLE_MODEL".to_string(),
            default_model: model.to_string(),
            description: "test custom provider".to_string(),
            extra_headers_env: None,
            unsupported_params: Vec::new(),
            setup: None,
        }
    }

    fn repo_in(dir: &Path) -> ProviderRepo {
        ProviderRepo::new(dir.join("providers.json"))
    }

    #[test]
    fn load_missing_overlay_returns_empty() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = repo_in(temp.path());
        assert!(repo.load().expect("load").is_empty());
    }

    #[test]
    fn upsert_appends_then_replaces_by_id() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = repo_in(temp.path());

        let appended = repo
            .upsert(custom_provider("acme", "acme-small"))
            .expect("first upsert");
        assert!(!appended, "first upsert appends");

        let replaced = repo
            .upsert(custom_provider("ACME", "acme-large"))
            .expect("second upsert");
        assert!(replaced, "case-insensitive id replaces rather than dupes");

        let overlay = repo.load().expect("load");
        assert_eq!(overlay.len(), 1, "no duplicate entry");
        assert_eq!(overlay[0].default_model, "acme-large");
    }

    #[test]
    fn delete_removes_only_matching_id() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = repo_in(temp.path());
        repo.upsert(custom_provider("acme", "m1"))
            .expect("upsert a");
        repo.upsert(custom_provider("globex", "m2"))
            .expect("upsert b");

        assert!(repo.delete("acme").expect("delete"), "removed acme");
        assert!(!repo.delete("nope").expect("delete"), "no-op for unknown");

        let overlay = repo.load().expect("load");
        assert_eq!(overlay.len(), 1);
        assert_eq!(overlay[0].id, "globex");
    }

    #[test]
    fn overlay_never_contains_an_api_key_value_field() {
        // ProviderDefinition has no `api_key` field; the serialized overlay
        // must therefore carry only `api_key_env` (a name), never a value.
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = repo_in(temp.path());
        repo.upsert(custom_provider("acme", "m1")).expect("upsert");

        let raw = std::fs::read_to_string(repo.path()).expect("read overlay");
        assert!(raw.contains("\"api_key_env\""), "overlay: {raw}");
        assert!(
            !raw.contains("\"api_key\""),
            "overlay must not hold a value: {raw}"
        );
    }

    #[test]
    fn malformed_overlay_is_a_hard_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = repo_in(temp.path());
        std::fs::write(repo.path(), "{ not valid json").expect("write garbage");
        assert!(matches!(repo.load(), Err(ProviderRepoError::Parse { .. })));
    }
}
