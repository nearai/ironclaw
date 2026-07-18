use ironclaw_common::hashing::sha256_hex;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};
use ironclaw_host_api::{ScopedPath, UserId};
use ironclaw_product_adapters::AdapterInstallationId;
use serde::{Deserialize, Serialize};

use crate::pairing::PairingCode;

pub const TELEGRAM_INSTALLATION_SETUP_PATH: &str =
    "/tenant-shared/telegram-setup/installation.json";
const TELEGRAM_PAIRING_CODE_ROOT: &str = "/tenant-shared/telegram-pairing/codes";
const TELEGRAM_PAIRING_USER_ROOT: &str = "/tenant-shared/telegram-pairing/users";
const TELEGRAM_PAIRING_COMPLETION_ROOT: &str =
    "/tenant-shared/telegram-pairing/pending-completions";
const TELEGRAM_BINDING_ROOT: &str = "/tenant-shared/telegram-binding/identities";
const TELEGRAM_BINDING_USER_ROOT: &str = "/tenant-shared/telegram-binding/users";
const TELEGRAM_DM_TARGET_ROOT: &str = "/tenant-shared/telegram-dm-targets";
const PATH_HASH_LEN: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct StoredTelegramBinding {
    pub(super) provider_user_id: String,
    pub(super) user_id: String,
    pub(super) epoch: String,
    #[serde(default = "binding_active_default")]
    pub(super) active: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct StoredTelegramBindingUserIndex {
    pub(super) provider_user_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct StoredPairingUserPointer {
    pub(super) code: PairingCode,
    #[serde(default = "pairing_pointer_active_default")]
    pub(super) active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct StoredPairingCompletion {
    pub(super) installation_id: AdapterInstallationId,
    pub(super) user_id: UserId,
    pub(super) chat_id: i64,
    pub(super) completed: bool,
}

fn binding_active_default() -> bool {
    true
}

fn pairing_pointer_active_default() -> bool {
    true
}

pub(super) fn setup_path() -> Result<ScopedPath, FilesystemError> {
    scoped_path(TELEGRAM_INSTALLATION_SETUP_PATH.to_string())
}

pub(super) fn pairing_code_path(code: &PairingCode) -> Result<ScopedPath, FilesystemError> {
    scoped_path(format!(
        "{TELEGRAM_PAIRING_CODE_ROOT}/{}.json",
        code.as_str()
    ))
}

pub(super) fn pairing_user_path(user_id: &UserId) -> Result<ScopedPath, FilesystemError> {
    scoped_path(format!(
        "{TELEGRAM_PAIRING_USER_ROOT}/{}.json",
        hashed_segment(user_id.as_str())
    ))
}

pub(super) fn pairing_completion_path(
    installation_id: &AdapterInstallationId,
    user_id: &UserId,
) -> Result<ScopedPath, FilesystemError> {
    scoped_path(format!(
        "{TELEGRAM_PAIRING_COMPLETION_ROOT}/{}/{}.json",
        hashed_segment(installation_id.as_str()),
        hashed_segment(user_id.as_str())
    ))
}

pub(super) fn binding_path(provider_user_id: &str) -> Result<ScopedPath, FilesystemError> {
    scoped_path(format!(
        "{TELEGRAM_BINDING_ROOT}/{}.json",
        hashed_segment(provider_user_id)
    ))
}

pub(super) fn binding_user_index_path(user_id: &UserId) -> Result<ScopedPath, FilesystemError> {
    scoped_path(format!(
        "{TELEGRAM_BINDING_USER_ROOT}/{}.json",
        hashed_segment(user_id.as_str())
    ))
}

pub(super) fn dm_target_path(
    installation_id: &AdapterInstallationId,
    user_id: &UserId,
) -> Result<ScopedPath, FilesystemError> {
    scoped_path(format!(
        "{TELEGRAM_DM_TARGET_ROOT}/{}/{}.json",
        hashed_segment(installation_id.as_str()),
        hashed_segment(user_id.as_str())
    ))
}

fn scoped_path(path: String) -> Result<ScopedPath, FilesystemError> {
    ScopedPath::new(path).map_err(|_| FilesystemError::BackendInfrastructure {
        operation: FilesystemOperation::ReadFile,
        reason: "Telegram host-state path is invalid".into(),
    })
}

fn hashed_segment(value: &str) -> String {
    let digest = sha256_hex(value.as_bytes());
    digest.chars().take(PATH_HASH_LEN).collect()
}
