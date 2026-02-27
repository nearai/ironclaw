//! OpenClaw installation discovery.
//!
//! Scans well-known directories for OpenClaw (and legacy) installations,
//! enumerating config files, workspace memory, session transcripts, and
//! credential files.

use std::path::{Path, PathBuf};

use crate::import::ImportError;

/// Names tried when looking for the state directory (in order).
const STATE_DIR_NAMES: &[&str] = &[".openclaw", ".clawdbot", ".moldbot", ".moltbot"];

/// Config file names tried inside the state directory (in order).
const CONFIG_FILE_NAMES: &[&str] = &[
    "openclaw.json",
    "clawdbot.json",
    "moldbot.json",
    "moltbot.json",
];

/// Identity files we look for in the workspace root.
const IDENTITY_FILE_NAMES: &[&str] = &[
    "AGENTS.md",
    "SOUL.md",
    "IDENTITY.md",
    "USER.md",
    "HEARTBEAT.md",
    "MEMORY.md",
    "TOOLS.md",
    "BOOTSTRAP.md",
];

/// Metadata about an agent's session directory.
#[derive(Debug, Clone)]
pub struct SessionDirInfo {
    pub agent_id: String,
    pub dir: PathBuf,
    pub jsonl_files: Vec<PathBuf>,
}

/// A detected OpenClaw installation with all discoverable data paths.
#[derive(Debug)]
pub struct OpenClawInstallation {
    /// Root state directory (e.g., `~/.openclaw`).
    pub state_dir: PathBuf,
    /// Config file path (e.g., `~/.openclaw/openclaw.json`).
    pub config_file: PathBuf,
    /// Workspace directory (e.g., `~/.openclaw/workspace/`).
    pub workspace_dir: PathBuf,
    /// Per-agent session directories with `.jsonl` files.
    pub session_dirs: Vec<SessionDirInfo>,
    /// Identity files found in the workspace root: `(filename, abs_path)`.
    pub identity_files: Vec<(String, PathBuf)>,
    /// Memory markdown files from `workspace/memory/`.
    pub memory_files: Vec<PathBuf>,
    /// OAuth credentials file, if present.
    pub oauth_file: Option<PathBuf>,
}

impl OpenClawInstallation {
    /// Discover an OpenClaw installation at a specific path or the default
    /// location (`~/.openclaw` with legacy fallbacks).
    pub fn discover(path: Option<&Path>) -> Result<Self, ImportError> {
        let state_dir = if let Some(p) = path {
            if !p.is_dir() {
                return Err(ImportError::NotFound);
            }
            p.to_path_buf()
        } else {
            Self::find_default_state_dir()?
        };

        // Find config file
        let config_file = CONFIG_FILE_NAMES
            .iter()
            .map(|name| state_dir.join(name))
            .find(|p| p.is_file())
            .unwrap_or_else(|| state_dir.join("openclaw.json"));

        // Find workspace directory (respects OPENCLAW_PROFILE env var)
        let workspace_dir = if let Ok(profile) = std::env::var("OPENCLAW_PROFILE") {
            state_dir.join(format!("workspace-{}", profile))
        } else {
            state_dir.join("workspace")
        };

        // Discover identity files
        let identity_files = if workspace_dir.is_dir() {
            IDENTITY_FILE_NAMES
                .iter()
                .filter_map(|name| {
                    let path = workspace_dir.join(name);
                    if path.is_file() {
                        Some((name.to_string(), path))
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        // Discover memory files
        let memory_dir = workspace_dir.join("memory");
        let memory_files = if memory_dir.is_dir() {
            Self::scan_md_files(&memory_dir)
        } else {
            Vec::new()
        };

        // Discover session directories
        let agents_dir = state_dir.join("agents");
        let session_dirs = if agents_dir.is_dir() {
            Self::scan_session_dirs(&agents_dir)
        } else {
            Vec::new()
        };

        // Check for OAuth file
        let oauth_path = state_dir.join("credentials").join("oauth.json");
        let oauth_file = if oauth_path.is_file() {
            Some(oauth_path)
        } else {
            None
        };

        Ok(Self {
            state_dir,
            config_file,
            workspace_dir,
            session_dirs,
            identity_files,
            memory_files,
            oauth_file,
        })
    }

    /// Check if an OpenClaw installation exists at the default location.
    pub fn exists_at_default() -> bool {
        Self::find_default_state_dir().is_ok()
    }

    /// Find the default state directory by trying well-known names under `$HOME`.
    fn find_default_state_dir() -> Result<PathBuf, ImportError> {
        let home = dirs::home_dir().ok_or(ImportError::NotFound)?;

        for name in STATE_DIR_NAMES {
            let candidate = home.join(name);
            if candidate.is_dir() {
                return Ok(candidate);
            }
        }

        Err(ImportError::NotFound)
    }

    /// Scan a directory for `*.md` files.
    fn scan_md_files(dir: &Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };

        entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            })
            .collect()
    }

    /// Scan the `agents/` directory for per-agent session directories.
    fn scan_session_dirs(agents_dir: &Path) -> Vec<SessionDirInfo> {
        let Ok(entries) = std::fs::read_dir(agents_dir) else {
            return Vec::new();
        };

        entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|agent_entry| {
                let agent_id = agent_entry.file_name().to_string_lossy().to_string();
                let sessions_dir = agent_entry.path().join("sessions");
                if !sessions_dir.is_dir() {
                    return None;
                }

                let jsonl_files: Vec<PathBuf> = std::fs::read_dir(&sessions_dir)
                    .ok()?
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| {
                        p.is_file()
                            && p.extension()
                                .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
                    })
                    .collect();

                if jsonl_files.is_empty() {
                    return None;
                }

                Some(SessionDirInfo {
                    agent_id,
                    dir: sessions_dir,
                    jsonl_files,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_tempdir_with_mock_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Create workspace with identity files
        let ws = root.join("workspace");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join("AGENTS.md"), "# agents").unwrap();
        std::fs::write(ws.join("SOUL.md"), "# soul").unwrap();

        // Create memory dir with docs
        let mem = ws.join("memory");
        std::fs::create_dir_all(&mem).unwrap();
        std::fs::write(mem.join("notes.md"), "# notes").unwrap();
        std::fs::write(mem.join("tasks.md"), "# tasks").unwrap();

        // Create agent sessions
        let sessions = root.join("agents").join("agent1").join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join("sess1.jsonl"), "{}").unwrap();
        std::fs::write(sessions.join("sessions.json"), "{}").unwrap();

        // Create config
        std::fs::write(root.join("openclaw.json"), "{}").unwrap();

        // Create oauth
        let creds = root.join("credentials");
        std::fs::create_dir_all(&creds).unwrap();
        std::fs::write(creds.join("oauth.json"), "{}").unwrap();

        let install = OpenClawInstallation::discover(Some(root)).unwrap();

        assert_eq!(install.state_dir, root);
        assert_eq!(install.config_file, root.join("openclaw.json"));
        assert_eq!(install.workspace_dir, ws);
        assert_eq!(install.identity_files.len(), 2);
        assert_eq!(install.memory_files.len(), 2);
        assert_eq!(install.session_dirs.len(), 1);
        assert_eq!(install.session_dirs[0].agent_id, "agent1");
        assert_eq!(install.session_dirs[0].jsonl_files.len(), 1);
        assert!(install.oauth_file.is_some());
    }

    #[test]
    fn discover_legacy_names() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Use legacy name
        std::fs::write(root.join("clawdbot.json"), "{}").unwrap();
        std::fs::create_dir_all(root.join("workspace")).unwrap();

        let install = OpenClawInstallation::discover(Some(root)).unwrap();
        assert_eq!(install.config_file, root.join("clawdbot.json"));
    }

    #[test]
    fn discover_not_found_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let nonexistent = tmp.path().join("does_not_exist");
        let result = OpenClawInstallation::discover(Some(&nonexistent));
        assert!(result.is_err());
    }

    #[test]
    fn discover_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let install = OpenClawInstallation::discover(Some(tmp.path())).unwrap();
        assert!(install.identity_files.is_empty());
        assert!(install.memory_files.is_empty());
        assert!(install.session_dirs.is_empty());
        assert!(install.oauth_file.is_none());
    }
}
