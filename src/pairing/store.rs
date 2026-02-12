//! Pairing store: pending requests and allowFrom list.
//!
//! Stored in ~/.ironclaw/{channel}-pairing.json and {channel}-allowFrom.json.

use std::collections::HashSet;
use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fs4::FileExt;
use rand::Rng;
use serde::{Deserialize, Serialize};

const PAIRING_CODE_LENGTH: usize = 8;
const PAIRING_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// TTL for pending pairing requests (minutes, not hours — reduces brute-force window).
const PAIRING_PENDING_TTL_SECS: u64 = 15 * 60;
const PAIRING_PENDING_MAX: usize = 3;
/// Max failed approve attempts per channel before rate limit kicks in.
const PAIRING_APPROVE_RATE_LIMIT: usize = 10;
/// Time window for rate limit (seconds).
const PAIRING_APPROVE_RATE_WINDOW_SECS: u64 = 5 * 60;

/// Error from pairing store operations.
#[derive(Debug, thiserror::Error)]
pub enum PairingStoreError {
    #[error("Invalid channel: {0}")]
    InvalidChannel(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Rate limit: too many failed approve attempts; try again later")]
    ApproveRateLimited,
}

/// Result of upserting a pairing request.
#[derive(Debug)]
pub struct UpsertResult {
    pub code: String,
    pub created: bool,
}

/// A pending pairing request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingRequest {
    pub id: String,
    pub code: String,
    pub created_at: String,
    pub last_seen_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PairingStoreFile {
    version: u8,
    requests: Vec<PairingRequest>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AllowFromStoreFile {
    version: u8,
    #[serde(rename = "allowFrom")]
    allow_from: Vec<String>,
}

fn default_pairing_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw")
}

fn safe_channel_key(channel: &str) -> Result<String, PairingStoreError> {
    let raw = channel.trim().to_lowercase();
    if raw.is_empty() {
        return Err(PairingStoreError::InvalidChannel("empty".to_string()));
    }
    let safe = raw
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect::<String>()
        .replace("..", "_");
    if safe.is_empty() || safe == "_" {
        return Err(PairingStoreError::InvalidChannel(channel.to_string()));
    }
    Ok(safe)
}

fn pairing_path(base_dir: &Path, channel: &str) -> Result<PathBuf, PairingStoreError> {
    let key = safe_channel_key(channel)?;
    Ok(base_dir.join(format!("{}-pairing.json", key)))
}

fn allow_from_path(base_dir: &Path, channel: &str) -> Result<PathBuf, PairingStoreError> {
    let key = safe_channel_key(channel)?;
    Ok(base_dir.join(format!("{}-allowFrom.json", key)))
}

fn approve_attempts_path(base_dir: &Path, channel: &str) -> Result<PathBuf, PairingStoreError> {
    let key = safe_channel_key(channel)?;
    Ok(base_dir.join(format!("{}-approve-attempts.json", key)))
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ApproveAttemptsFile {
    failed_at: Vec<u64>,
}

fn now_iso() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    #[allow(clippy::cast_possible_wrap)]
    chrono::DateTime::from_timestamp(now.as_secs() as i64, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| now.as_secs().to_string())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn parse_timestamp(value: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.timestamp() as u64)
        .or_else(|| value.parse::<u64>().ok())
}

fn is_expired(req: &PairingRequest, now_secs: u64) -> bool {
    let created = parse_timestamp(&req.created_at).unwrap_or(0);
    now_secs.saturating_sub(created) > PAIRING_PENDING_TTL_SECS
}

fn random_code() -> String {
    let mut rng = rand::thread_rng();
    (0..PAIRING_CODE_LENGTH)
        .map(|_| {
            let idx = rng.gen_range(0..PAIRING_ALPHABET.len());
            PAIRING_ALPHABET[idx] as char
        })
        .collect()
}

fn generate_unique_code(existing: &HashSet<String>) -> String {
    let mut rng = rand::thread_rng();
    for _ in 0..500 {
        let code = random_code();
        if !existing.contains(&code) {
            return code;
        }
    }
    // Fallback: add suffix
    format!("{}{:04}", random_code(), rng.gen_range(0..10000))
}

/// Pairing store for a channel.
#[derive(Debug, Clone)]
pub struct PairingStore {
    base_dir: PathBuf,
}

impl PairingStore {
    /// Create a new pairing store using default directory (~/.ironclaw).
    pub fn new() -> Self {
        Self {
            base_dir: default_pairing_dir(),
        }
    }

    /// Create a pairing store with a custom base directory (for testing).
    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// List pending pairing requests for a channel.
    pub fn list_pending(&self, channel: &str) -> Result<Vec<PairingRequest>, PairingStoreError> {
        let path = pairing_path(&self.base_dir, channel)?;
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(e) => return Err(e.into()),
        };

        let file: PairingStoreFile = serde_json::from_str(&content).unwrap_or(PairingStoreFile {
            version: 1,
            requests: Vec::new(),
        });

        let now = now_secs();
        let original_len = file.requests.len();
        let mut requests: Vec<_> = file
            .requests
            .into_iter()
            .filter(|r| !is_expired(r, now))
            .collect();

        if requests.len() != original_len {
            self.write_pairing_file(channel, &requests)?;
        }

        requests.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(requests)
    }

    /// Upsert a pairing request. Returns (code, created).
    pub fn upsert_request(
        &self,
        channel: &str,
        id: &str,
        meta: Option<serde_json::Value>,
    ) -> Result<UpsertResult, PairingStoreError> {
        let path = pairing_path(&self.base_dir, channel)?;
        fs::create_dir_all(path.parent().unwrap())?;

        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;

        file.lock_exclusive()?;

        let content = fs::read_to_string(&path).unwrap_or_default();
        let mut store: PairingStoreFile =
            serde_json::from_str(&content).unwrap_or(PairingStoreFile {
                version: 1,
                requests: Vec::new(),
            });

        let now = now_iso();
        let now_secs = now_secs();
        let id = id.trim().to_string();
        if id.is_empty() {
            fs4::FileExt::unlock(&file)?;
            return Err(PairingStoreError::InvalidChannel("empty id".to_string()));
        }

        store.requests.retain(|r| !is_expired(r, now_secs));
        let existing_codes: HashSet<String> = store
            .requests
            .iter()
            .map(|r| r.code.to_uppercase())
            .collect();

        if let Some(idx) = store.requests.iter().position(|r| r.id == id) {
            let req = &mut store.requests[idx];
            let code = if req.code.is_empty() {
                generate_unique_code(&existing_codes)
            } else {
                req.code.clone()
            };
            req.last_seen_at = now.clone();
            req.code = code.clone();
            if let Some(m) = meta {
                req.meta = Some(m);
            }
            self.write_pairing_file_locked(&mut file, channel, &store.requests)?;
            fs4::FileExt::unlock(&file)?;
            return Ok(UpsertResult {
                code,
                created: false,
            });
        }

        if store.requests.len() >= PAIRING_PENDING_MAX {
            fs4::FileExt::unlock(&file)?;
            return Ok(UpsertResult {
                code: String::new(),
                created: false,
            });
        }

        let code = generate_unique_code(&existing_codes);
        store.requests.push(PairingRequest {
            id: id.clone(),
            code: code.clone(),
            created_at: now.clone(),
            last_seen_at: now,
            meta,
        });

        self.write_pairing_file_locked(&mut file, channel, &store.requests)?;
        fs4::FileExt::unlock(&file)?;

        Ok(UpsertResult {
            code,
            created: true,
        })
    }

    fn is_approve_rate_limited(&self, channel: &str) -> Result<bool, PairingStoreError> {
        let path = approve_attempts_path(&self.base_dir, channel)?;
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        let mut data: ApproveAttemptsFile = serde_json::from_str(&content).unwrap_or_default();
        let now = now_secs();
        let cutoff = now.saturating_sub(PAIRING_APPROVE_RATE_WINDOW_SECS);
        data.failed_at.retain(|&t| t >= cutoff);
        Ok(data.failed_at.len() >= PAIRING_APPROVE_RATE_LIMIT)
    }

    fn record_failed_approve(&self, channel: &str) -> Result<(), PairingStoreError> {
        let path = approve_attempts_path(&self.base_dir, channel)?;
        fs::create_dir_all(path.parent().unwrap())?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        file.lock_exclusive()?;
        let content = fs::read_to_string(&path).unwrap_or_default();
        let mut data: ApproveAttemptsFile = serde_json::from_str(&content).unwrap_or_default();
        let now = now_secs();
        data.failed_at.push(now);
        let cutoff = now.saturating_sub(PAIRING_APPROVE_RATE_WINDOW_SECS);
        data.failed_at.retain(|&t| t >= cutoff);
        let json = serde_json::to_string_pretty(&data)?;
        fs::write(&path, json)?;
        fs4::FileExt::unlock(&file)?;
        Ok(())
    }

    /// Approve a pairing code and add the sender to allowFrom.
    pub fn approve(
        &self,
        channel: &str,
        code: &str,
    ) -> Result<Option<PairingRequest>, PairingStoreError> {
        let code = code.trim().to_uppercase();
        if code.is_empty() {
            return Ok(None);
        }

        if self.is_approve_rate_limited(channel)? {
            return Err(PairingStoreError::ApproveRateLimited);
        }

        let path = pairing_path(&self.base_dir, channel)?;
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(false)
            .open(&path)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    PairingStoreError::InvalidChannel("no pairing file".to_string())
                } else {
                    PairingStoreError::Io(e)
                }
            })?;

        file.lock_exclusive()?;

        let content = fs::read_to_string(&path).unwrap_or_default();
        let mut store: PairingStoreFile =
            serde_json::from_str(&content).unwrap_or(PairingStoreFile {
                version: 1,
                requests: Vec::new(),
            });

        let now_secs = now_secs();
        store.requests.retain(|r| !is_expired(r, now_secs));

        let idx = store
            .requests
            .iter()
            .position(|r| r.code.to_uppercase() == code);

        let entry = match idx {
            Some(i) => store.requests.remove(i),
            None => {
                fs4::FileExt::unlock(&file)?;
                self.record_failed_approve(channel)?;
                return Ok(None);
            }
        };

        self.write_pairing_file_locked(&mut file, channel, &store.requests)?;
        fs4::FileExt::unlock(&file)?;

        self.add_allow_from(channel, &entry.id)?;

        Ok(Some(entry))
    }

    /// Read the allowFrom list for a channel.
    pub fn read_allow_from(&self, channel: &str) -> Result<Vec<String>, PairingStoreError> {
        let path = allow_from_path(&self.base_dir, channel)?;
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(e) => return Err(e.into()),
        };

        let file: AllowFromStoreFile =
            serde_json::from_str(&content).unwrap_or(AllowFromStoreFile {
                version: 1,
                allow_from: Vec::new(),
            });

        Ok(file.allow_from)
    }

    /// Check if a sender is allowed (by id or username).
    pub fn is_sender_allowed(
        &self,
        channel: &str,
        id: &str,
        username: Option<&str>,
    ) -> Result<bool, PairingStoreError> {
        let allow = self.read_allow_from(channel)?;
        let id = id.trim();
        let id_ok = allow.iter().any(|e| e.trim() == id);
        if id_ok {
            return Ok(true);
        }
        if let Some(u) = username {
            let u = u.trim().to_lowercase();
            let u_norm = u.strip_prefix('@').unwrap_or(&u);
            if allow.iter().any(|e| {
                e.trim().to_lowercase() == u || e.trim().to_lowercase() == format!("@{}", u_norm)
            }) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn add_allow_from(&self, channel: &str, entry: &str) -> Result<(), PairingStoreError> {
        let entry = entry.trim().to_string();
        if entry.is_empty() {
            return Ok(());
        }

        let path = allow_from_path(&self.base_dir, channel)?;
        fs::create_dir_all(path.parent().unwrap())?;

        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;

        file.lock_exclusive()?;

        let content = fs::read_to_string(&path).unwrap_or_default();
        let mut store: AllowFromStoreFile =
            serde_json::from_str(&content).unwrap_or(AllowFromStoreFile {
                version: 1,
                allow_from: Vec::new(),
            });

        let normalized = entry.to_lowercase();
        if store
            .allow_from
            .iter()
            .any(|e| e.to_lowercase() == normalized)
        {
            fs4::FileExt::unlock(&file)?;
            return Ok(());
        }

        store.allow_from.push(entry);
        let json = serde_json::to_string_pretty(&store)?;
        fs::write(&path, json)?;

        fs4::FileExt::unlock(&file)?;
        Ok(())
    }

    fn write_pairing_file(
        &self,
        channel: &str,
        requests: &[PairingRequest],
    ) -> Result<(), PairingStoreError> {
        let path = pairing_path(&self.base_dir, channel)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        file.lock_exclusive()?;
        self.write_pairing_file_locked(&mut file, channel, requests)?;
        fs4::FileExt::unlock(&file)?;
        Ok(())
    }

    fn write_pairing_file_locked(
        &self,
        file: &mut fs::File,
        _channel: &str,
        requests: &[PairingRequest],
    ) -> Result<(), PairingStoreError> {
        let store = PairingStoreFile {
            version: 1,
            requests: requests.to_vec(),
        };
        let json = serde_json::to_string_pretty(&store)?;
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }
}

impl Default for PairingStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_safe_channel_key() {
        assert_eq!(safe_channel_key("telegram").unwrap(), "telegram");
        assert_eq!(safe_channel_key("Telegram").unwrap(), "telegram");
        safe_channel_key("").unwrap_err();
    }

    #[test]
    fn test_random_code() {
        let c = random_code();
        assert_eq!(c.len(), PAIRING_CODE_LENGTH);
        assert!(c.chars().all(|c| PAIRING_ALPHABET.contains(&(c as u8))));
    }

    fn test_store() -> (PairingStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = PairingStore::with_base_dir(dir.path().to_path_buf());
        (store, dir)
    }

    #[test]
    fn test_list_pending_empty() {
        let (store, _) = test_store();
        let requests = store.list_pending("telegram").unwrap();
        assert!(requests.is_empty());
    }

    #[test]
    fn test_upsert_request_creates_new() {
        let (store, _) = test_store();
        let result = store
            .upsert_request(
                "telegram",
                "user123",
                Some(serde_json::json!({"chat_id": 456})),
            )
            .unwrap();
        assert!(result.created);
        assert_eq!(result.code.len(), PAIRING_CODE_LENGTH);
        assert!(
            result
                .code
                .chars()
                .all(|c| PAIRING_ALPHABET.contains(&(c as u8)))
        );
    }

    #[test]
    fn test_upsert_request_updates_existing() {
        let (store, _) = test_store();
        let r1 = store.upsert_request("telegram", "user123", None).unwrap();
        assert!(r1.created);
        let r2 = store
            .upsert_request("telegram", "user123", Some(serde_json::json!({"x": 1})))
            .unwrap();
        assert!(!r2.created);
        assert_eq!(r1.code, r2.code);

        let pending = store.list_pending("telegram").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "user123");
        assert_eq!(pending[0].meta, Some(serde_json::json!({"x": 1})));
    }

    #[test]
    fn test_approve_adds_to_allow_from() {
        let (store, _) = test_store();
        let r = store.upsert_request("telegram", "user456", None).unwrap();
        assert!(r.created);

        let approved = store.approve("telegram", &r.code).unwrap();
        assert!(approved.is_some());
        assert_eq!(approved.unwrap().id, "user456");

        let allow = store.read_allow_from("telegram").unwrap();
        assert_eq!(allow, vec!["user456"]);
    }

    #[test]
    fn test_approve_case_insensitive_code() {
        let (store, _) = test_store();
        let r = store.upsert_request("telegram", "user789", None).unwrap();
        let code_lower = r.code.to_lowercase();
        let approved = store.approve("telegram", &code_lower).unwrap();
        assert!(approved.is_some());
    }

    #[test]
    fn test_approve_invalid_code_returns_none() {
        let (store, _) = test_store();
        store.upsert_request("telegram", "user123", None).unwrap();
        let approved = store.approve("telegram", "BADCODE1").unwrap();
        assert!(approved.is_none());
    }

    #[test]
    fn test_approve_rate_limited_after_many_failures() {
        let (store, _) = test_store();
        store.upsert_request("telegram", "user123", None).unwrap();
        for _ in 0..PAIRING_APPROVE_RATE_LIMIT {
            let _ = store.approve("telegram", "WRONG01");
        }
        let err = store.approve("telegram", "WRONG02").unwrap_err();
        assert!(matches!(err, PairingStoreError::ApproveRateLimited));
    }

    #[test]
    fn test_is_sender_allowed_by_id() {
        let (store, _) = test_store();
        let r = store.upsert_request("telegram", "user999", None).unwrap();
        store.approve("telegram", &r.code).unwrap();

        assert!(
            store
                .is_sender_allowed("telegram", "user999", None)
                .unwrap()
        );
        assert!(!store.is_sender_allowed("telegram", "other", None).unwrap());
    }

    #[test]
    fn test_is_sender_allowed_by_username() {
        let (store, _) = test_store();
        store
            .upsert_request(
                "telegram",
                "alice",
                Some(serde_json::json!({"username": "alice"})),
            )
            .unwrap();
        let pending = store.list_pending("telegram").unwrap();
        store.approve("telegram", &pending[0].code).unwrap();

        // approve adds id to allow_from. For username we need to add it manually.
        // Actually approve adds entry.id which is "alice". So is_sender_allowed("telegram", "alice", None) would work.
        assert!(store.is_sender_allowed("telegram", "alice", None).unwrap());
        assert!(
            store
                .is_sender_allowed("telegram", "alice", Some("alice"))
                .unwrap()
        );
    }

    #[test]
    fn test_channel_normalization() {
        let (store, _) = test_store();
        store.upsert_request("Telegram", "u1", None).unwrap();
        let pending = store.list_pending("telegram").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "u1");
    }

    #[test]
    fn test_invalid_channel_rejected() {
        let (store, _) = test_store();
        store.upsert_request("telegram", "u1", None).unwrap();
        store.list_pending("").unwrap_err();
        store.upsert_request("", "u1", None).unwrap_err();
    }
}
