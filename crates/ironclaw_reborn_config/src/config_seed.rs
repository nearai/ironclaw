//! First-run `config.toml` seeding for Reborn runtime startup.
//!
//! Keep this separate from `config_file.rs`: parsing/editing the operator TOML
//! and deciding when to create a first-run file are different responsibilities.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::{REBORN_CONFIG_API_VERSION, RebornConfigFile, RebornConfigFileError, RebornProfile};

#[derive(Debug, Error)]
pub enum RebornConfigSeedError {
    #[error("create Reborn config parent `{}`: {source}", path.display())]
    CreateParent {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("create temporary Reborn config file near `{}`: {source}", path.display())]
    CreateTemp {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("write temporary Reborn config file `{}`: {source}", path.display())]
    WriteTemp {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("validate seeded Reborn config `{}`: {source}", path.display())]
    Validate {
        path: PathBuf,
        source: Box<RebornConfigFileError>,
    },
    #[error("persist seeded Reborn config `{}`: {source}", path.display())]
    Persist {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebornConfigSeedOutcome {
    Seeded,
    AlreadyPresent,
}

/// Atomically seed a sparse first-run `config.toml` if the file is missing.
///
/// The first-run seed is intentionally smaller than `ironclaw-reborn config
/// init`: it records only the API version and safe default boot profile without
/// installing an active LLM slot or pinning compiled runtime defaults. That
/// preserves the existing "missing config" behavior for env-driven provider
/// selection while giving operators an editable TOML on first real runtime
/// start.
pub fn seed_default_config_file_if_missing(
    path: &Path,
) -> Result<RebornConfigSeedOutcome, RebornConfigSeedError> {
    let text = first_run_config_toml();
    RebornConfigFile::parse_text(&text, path).map_err(|source| {
        RebornConfigSeedError::Validate {
            path: path.to_path_buf(),
            source: Box::new(source),
        }
    })?;

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    if let Some(parent) = parent {
        fs::create_dir_all(parent).map_err(|source| RebornConfigSeedError::CreateParent {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let parent = parent.unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|source| {
        RebornConfigSeedError::CreateTemp {
            path: path.to_path_buf(),
            source,
        }
    })?;
    tmp.write_all(text.as_bytes())
        .map_err(|source| RebornConfigSeedError::WriteTemp {
            path: tmp.path().to_path_buf(),
            source,
        })?;
    tmp.persist_noclobber(path).map_or_else(
        |error| {
            if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                Ok(RebornConfigSeedOutcome::AlreadyPresent)
            } else {
                Err(RebornConfigSeedError::Persist {
                    path: path.to_path_buf(),
                    source: error.error,
                })
            }
        },
        |_| Ok(RebornConfigSeedOutcome::Seeded),
    )
}

fn first_run_config_toml() -> String {
    let profile = RebornProfile::default();
    format!(
        r#"# IronClaw Reborn first-run configuration.
#
# This sparse file is created automatically the first time an
# `ironclaw-reborn` command starts the runtime. It records the stable,
# safe default boot choice only; other omitted fields continue to use
# compiled defaults unless you set them here. One-off env/CLI choices
# are not persisted into this file.
#
# Precedence on each field:
#   compiled defaults < this file < env vars < CLI flags.
#
# For a fully commented operator template, run:
#   ironclaw-reborn config init --force
#
# Secrets stay out of this file. Store token values in environment variables
# or a secret store, and reference them here only by env-var NAME.

api_version = "{api_version}"

[boot]
profile = "{profile}"
"#,
        api_version = REBORN_CONFIG_API_VERSION,
        profile = profile.as_str(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_default_config_file_writes_sparse_first_run_config() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("reborn").join("config.toml");

        let outcome = seed_default_config_file_if_missing(&path).expect("seed should succeed");

        assert_eq!(outcome, RebornConfigSeedOutcome::Seeded);
        let text = std::fs::read_to_string(&path).expect("seeded config readable");
        assert!(
            text.contains("api_version = \"ironclaw.runtime/v1\""),
            "seeded config should stamp api_version: {text}"
        );
        assert!(
            text.contains("profile = \"local-dev\""),
            "seeded config should record profile: {text}"
        );
        assert!(
            !text.contains("[llm.default]"),
            "first-run seed must not force an active LLM slot: {text}"
        );
        assert!(
            !text.contains("[runner]")
                && !text.contains("[skills]")
                && !text.contains("[identity]"),
            "first-run seed must not pin compiled defaults: {text}"
        );
        let parsed = RebornConfigFile::load(&path)
            .expect("load seeded config")
            .expect("seeded config present");
        assert!(parsed.default_llm_slot().is_none());
    }

    #[test]
    fn seed_default_config_file_does_not_clobber_existing_config() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.toml");
        std::fs::write(&path, "api_version = \"ironclaw.runtime/v1\"\n").expect("write config");

        let outcome = seed_default_config_file_if_missing(&path)
            .expect("seed should treat existing config as ok");

        assert_eq!(outcome, RebornConfigSeedOutcome::AlreadyPresent);
        let text = std::fs::read_to_string(&path).expect("config readable");
        assert_eq!(text, "api_version = \"ironclaw.runtime/v1\"\n");
    }

    #[test]
    fn seed_default_config_file_handles_concurrent_first_run_race() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.toml");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));

        let handles = (0..8)
            .map(|_| {
                let path = path.clone();
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    seed_default_config_file_if_missing(&path)
                })
            })
            .collect::<Vec<_>>();

        let outcomes = handles
            .into_iter()
            .map(|handle| handle.join().expect("seed thread should not panic"))
            .collect::<Result<Vec<_>, _>>()
            .expect("concurrent seeds should not fail");

        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == RebornConfigSeedOutcome::Seeded)
                .count(),
            1,
            "exactly one concurrent first-run seed should create the file: {outcomes:?}"
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == RebornConfigSeedOutcome::AlreadyPresent)
                .count(),
            outcomes.len() - 1,
            "losing concurrent seeds should observe the existing file: {outcomes:?}"
        );
        let parsed = RebornConfigFile::load(&path)
            .expect("load seeded config")
            .expect("seeded config present");
        assert!(parsed.default_llm_slot().is_none());
    }

    #[test]
    fn seed_default_config_file_reports_parent_creation_failure() {
        let temp = tempfile::tempdir().expect("tempdir");
        let blocking_file = temp.path().join("not-a-directory");
        std::fs::write(&blocking_file, "file").expect("write blocking file");
        let path = blocking_file.join("config.toml");

        let error = seed_default_config_file_if_missing(&path)
            .expect_err("regular file in parent path should fail parent creation");

        assert!(
            matches!(error, RebornConfigSeedError::CreateParent { .. }),
            "expected CreateParent error, got {error:?}"
        );
    }
}
