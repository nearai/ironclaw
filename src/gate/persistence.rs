//! File-backed persistence for pending gates.

use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use async_trait::async_trait;
use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};

use crate::bootstrap::ironclaw_base_dir;

use super::pending::{PendingGate, PendingGateKey};
use super::store::{GatePersistence, GateStoreError};

#[derive(Debug, Default, Serialize, Deserialize)]
struct PendingGateFile {
    version: u8,
    gates: Vec<PendingGate>,
}

/// JSON-file persistence for pending gates under `~/.ironclaw/`.
#[derive(Debug, Clone)]
pub struct FileGatePersistence {
    path: PathBuf,
}

impl FileGatePersistence {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn default_path() -> PathBuf {
        ironclaw_base_dir().join("pending-gates.json")
    }

    pub fn with_default_path() -> Self {
        Self::new(Self::default_path())
    }

    fn open_locked_file(&self) -> Result<std::fs::File, GateStoreError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| GateStoreError::Persistence {
                reason: format!("create parent dir '{}': {e}", parent.display()),
            })?;
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.path)
            .map_err(|e| GateStoreError::Persistence {
                reason: format!("open '{}': {e}", self.path.display()),
            })?;

        file.lock_exclusive()
            .map_err(|e| GateStoreError::Persistence {
                reason: format!("lock '{}': {e}", self.path.display()),
            })?;
        Ok(file)
    }

    fn read_state(&self, file: &mut std::fs::File) -> Result<PendingGateFile, GateStoreError> {
        file.seek(SeekFrom::Start(0))
            .map_err(|e| GateStoreError::Persistence {
                reason: format!("seek '{}': {e}", self.path.display()),
            })?;

        let mut content = String::new();
        file.read_to_string(&mut content)
            .map_err(|e| GateStoreError::Persistence {
                reason: format!("read '{}': {e}", self.path.display()),
            })?;

        if content.trim().is_empty() {
            return Ok(PendingGateFile {
                version: 1,
                gates: Vec::new(),
            });
        }

        serde_json::from_str(&content).map_err(|e| GateStoreError::Persistence {
            reason: format!("parse '{}': {e}", self.path.display()),
        })
    }

    fn write_state(
        &self,
        file: &mut std::fs::File,
        state: &PendingGateFile,
    ) -> Result<(), GateStoreError> {
        let json = serde_json::to_vec_pretty(state).map_err(|e| GateStoreError::Persistence {
            reason: format!("serialize '{}': {e}", self.path.display()),
        })?;

        file.set_len(0).map_err(|e| GateStoreError::Persistence {
            reason: format!("truncate '{}': {e}", self.path.display()),
        })?;
        file.seek(SeekFrom::Start(0))
            .map_err(|e| GateStoreError::Persistence {
                reason: format!("seek '{}': {e}", self.path.display()),
            })?;
        file.write_all(&json)
            .and_then(|_| file.flush())
            .and_then(|_| file.sync_all())
            .map_err(|e| GateStoreError::Persistence {
                reason: format!("write '{}': {e}", self.path.display()),
            })
    }
}

#[async_trait]
impl GatePersistence for FileGatePersistence {
    async fn save(&self, gate: &PendingGate) -> Result<(), GateStoreError> {
        let mut file = self.open_locked_file()?;
        let mut state = self.read_state(&mut file)?;
        state.gates.retain(|existing| existing.key() != gate.key());
        state.gates.push(gate.clone());
        self.write_state(&mut file, &state)
    }

    async fn remove(&self, key: &PendingGateKey) -> Result<(), GateStoreError> {
        let mut file = self.open_locked_file()?;
        let mut state = self.read_state(&mut file)?;
        state.gates.retain(|existing| existing.key() != *key);
        self.write_state(&mut file, &state)
    }

    async fn load_all(&self) -> Result<Vec<PendingGate>, GateStoreError> {
        let mut file = self.open_locked_file()?;
        Ok(self.read_state(&mut file)?.gates)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use ironclaw_engine::{ConversationId, ResumeKind, ThreadId};

    use super::*;

    fn sample_gate() -> PendingGate {
        PendingGate {
            request_id: uuid::Uuid::new_v4(),
            gate_name: "approval".into(),
            user_id: "user1".into(),
            thread_id: ThreadId::new(),
            scope_thread_id: None,
            conversation_id: ConversationId::new(),
            source_channel: "web".into(),
            action_name: "shell".into(),
            call_id: "call_1".into(),
            parameters: serde_json::json!({"cmd":"ls"}),
            display_parameters: None,
            description: "pending".into(),
            resume_kind: ResumeKind::Approval { allow_always: true },
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::minutes(30),
            original_message: None,
            resume_output: None,
            approval_already_granted: false,
        }
    }

    #[tokio::test]
    async fn file_persistence_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let persistence = FileGatePersistence::new(dir.path().join("pending-gates.json"));
        let gate = sample_gate();
        let key = gate.key();

        persistence.save(&gate).await.unwrap();
        let loaded = persistence.load_all().await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].request_id, gate.request_id);

        persistence.remove(&key).await.unwrap();
        assert!(persistence.load_all().await.unwrap().is_empty());
    }
}
