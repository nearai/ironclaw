use std::collections::HashMap;
use std::sync::{Arc, Weak};

use crate::db::SettingsStore;
use crate::error::DatabaseError;
use crate::secrets::{CredentialLocation, SecretBindingApproval};

const SECRET_BINDING_APPROVALS_KEY: &str = "auth.secret_binding_approvals_v1";
pub const SECRET_BINDING_APPROVAL_GATE_NAME: &str = "secret_binding_approval";
pub const SECRET_BINDING_APPROVAL_ERROR: &str = "secret_binding_approval_required";

async fn approval_lock(user_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: std::sync::OnceLock<
        tokio::sync::Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
    > = std::sync::OnceLock::new();
    let registry = LOCKS.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()));
    let mut locks = registry.lock().await;
    if let Some(lock) = locks.get(user_id).and_then(Weak::upgrade) {
        return lock;
    }
    locks.retain(|_, lock| lock.strong_count() > 0);
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    locks.insert(user_id.to_string(), Arc::downgrade(&lock));
    lock
}

fn dedup_approvals(approvals: Vec<SecretBindingApproval>) -> Vec<SecretBindingApproval> {
    let mut by_id = HashMap::new();
    for approval in approvals {
        by_id.insert(approval.approval_id(), approval);
    }
    let mut approvals: Vec<_> = by_id.into_values().collect();
    approvals.sort_by(|a, b| a.approved_at.cmp(&b.approved_at));
    approvals
}

async fn load_approvals_inner(
    store: &dyn SettingsStore,
    user_id: &str,
) -> Result<Vec<SecretBindingApproval>, DatabaseError> {
    let value = match store
        .get_setting(user_id, SECRET_BINDING_APPROVALS_KEY)
        .await?
    {
        Some(value) => value,
        None => return Ok(Vec::new()),
    };

    let approvals: Vec<SecretBindingApproval> =
        serde_json::from_value(value).map_err(|error| DatabaseError::Query(error.to_string()))?;
    Ok(dedup_approvals(approvals))
}

async fn save_approvals_inner(
    store: &dyn SettingsStore,
    user_id: &str,
    approvals: &[SecretBindingApproval],
) -> Result<(), DatabaseError> {
    if approvals.is_empty() {
        let _ = store
            .delete_setting(user_id, SECRET_BINDING_APPROVALS_KEY)
            .await?;
        return Ok(());
    }

    let value =
        serde_json::to_value(approvals).map_err(|error| DatabaseError::Query(error.to_string()))?;
    store
        .set_setting(user_id, SECRET_BINDING_APPROVALS_KEY, &value)
        .await
}

pub async fn list_binding_approvals(
    store: Option<&(dyn SettingsStore + Send + Sync)>,
    user_id: &str,
) -> Result<Vec<SecretBindingApproval>, DatabaseError> {
    let Some(store) = store else {
        return Ok(Vec::new());
    };
    load_approvals_inner(store, user_id).await
}

pub async fn binding_approval_exists(
    store: Option<&(dyn SettingsStore + Send + Sync)>,
    user_id: &str,
    approval: &SecretBindingApproval,
) -> Result<bool, DatabaseError> {
    let approvals = list_binding_approvals(store, user_id).await?;
    let approval_id = approval.approval_id();
    Ok(approvals
        .iter()
        .any(|item| item.approval_id() == approval_id))
}

pub async fn grant_binding_approval(
    store: Option<&(dyn SettingsStore + Send + Sync)>,
    user_id: &str,
    approval: SecretBindingApproval,
) -> Result<(), DatabaseError> {
    let Some(store) = store else {
        return Ok(());
    };

    let lock = approval_lock(user_id).await;
    let _guard = lock.lock().await;

    let mut approvals = load_approvals_inner(store, user_id).await?;
    let approval_id = approval.approval_id();
    approvals.retain(|item| item.approval_id() != approval_id);
    approvals.push(approval);
    let approvals = dedup_approvals(approvals);
    save_approvals_inner(store, user_id, &approvals).await
}

pub async fn revoke_binding_approval(
    store: Option<&(dyn SettingsStore + Send + Sync)>,
    user_id: &str,
    approval_id: &str,
) -> Result<bool, DatabaseError> {
    let Some(store) = store else {
        return Ok(false);
    };

    let lock = approval_lock(user_id).await;
    let _guard = lock.lock().await;

    let mut approvals = load_approvals_inner(store, user_id).await?;
    let before = approvals.len();
    approvals.retain(|item| item.approval_id() != approval_id);
    if approvals.len() == before {
        return Ok(false);
    }
    save_approvals_inner(store, user_id, &approvals).await?;
    Ok(true)
}

pub async fn revoke_secret_binding_approvals(
    store: Option<&(dyn SettingsStore + Send + Sync)>,
    user_id: &str,
    secret_name: &str,
) -> Result<usize, DatabaseError> {
    let Some(store) = store else {
        return Ok(0);
    };

    let lock = approval_lock(user_id).await;
    let _guard = lock.lock().await;

    let mut approvals = load_approvals_inner(store, user_id).await?;
    let before = approvals.len();
    approvals.retain(|item| item.secret_name != secret_name);
    if approvals.len() == before {
        return Ok(0);
    }
    save_approvals_inner(store, user_id, &approvals).await?;
    Ok(before - approvals.len())
}

pub fn location_label(location: &CredentialLocation) -> String {
    match location {
        CredentialLocation::AuthorizationBearer => "bearer".to_string(),
        CredentialLocation::AuthorizationBasic { .. } => "basic_auth".to_string(),
        CredentialLocation::Header { name, .. } => format!("header:{name}"),
        CredentialLocation::QueryParam { name } => format!("query_param:{name}"),
        CredentialLocation::UrlPath { placeholder } => format!("url_path:{placeholder}"),
    }
}

pub fn location_risk(location: &CredentialLocation) -> &'static str {
    match location {
        CredentialLocation::QueryParam { .. } => "high",
        _ => "normal",
    }
}

pub fn approval_prompt_message(approval: &SecretBindingApproval) -> String {
    format!(
        "Approve {} '{}' to use secret '{}' for host '{}' via {}.",
        approval.artifact_kind.as_str(),
        approval.artifact_name,
        approval.secret_name,
        approval.host,
        location_label(&approval.location),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use async_trait::async_trait;
    use chrono::Utc;
    use tokio::sync::RwLock;

    use super::*;
    use crate::db::SettingsStore;
    use crate::history::SettingRow;
    use crate::secrets::{CredentialArtifactKind, CredentialLocation};

    struct MemorySettingsStore {
        values: RwLock<HashMap<(String, String), serde_json::Value>>,
    }

    impl MemorySettingsStore {
        fn new() -> Self {
            Self {
                values: RwLock::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl SettingsStore for MemorySettingsStore {
        async fn get_setting(
            &self,
            user_id: &str,
            key: &str,
        ) -> Result<Option<serde_json::Value>, DatabaseError> {
            Ok(self
                .values
                .read()
                .await
                .get(&(user_id.to_string(), key.to_string()))
                .cloned())
        }

        async fn get_setting_full(
            &self,
            _user_id: &str,
            _key: &str,
        ) -> Result<Option<SettingRow>, DatabaseError> {
            Ok(None)
        }

        async fn set_setting(
            &self,
            user_id: &str,
            key: &str,
            value: &serde_json::Value,
        ) -> Result<(), DatabaseError> {
            self.values
                .write()
                .await
                .insert((user_id.to_string(), key.to_string()), value.clone());
            Ok(())
        }

        async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError> {
            Ok(self
                .values
                .write()
                .await
                .remove(&(user_id.to_string(), key.to_string()))
                .is_some())
        }

        async fn list_settings(&self, _user_id: &str) -> Result<Vec<SettingRow>, DatabaseError> {
            Ok(Vec::new())
        }

        async fn get_all_settings(
            &self,
            _user_id: &str,
        ) -> Result<HashMap<String, serde_json::Value>, DatabaseError> {
            Ok(HashMap::new())
        }

        async fn set_all_settings(
            &self,
            _user_id: &str,
            _settings: &HashMap<String, serde_json::Value>,
        ) -> Result<(), DatabaseError> {
            Ok(())
        }

        async fn has_settings(&self, _user_id: &str) -> Result<bool, DatabaseError> {
            Ok(false)
        }
    }

    fn sample_approval(secret_name: &str, host: &str) -> SecretBindingApproval {
        SecretBindingApproval {
            secret_name: secret_name.to_string(),
            artifact_kind: CredentialArtifactKind::Skill,
            artifact_name: "github-workflow".to_string(),
            artifact_fingerprint: "hash123".to_string(),
            host: host.to_string(),
            location: CredentialLocation::AuthorizationBearer,
            approved_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn grant_and_revoke_binding_approval_round_trip() {
        let store: Arc<dyn SettingsStore + Send + Sync> = Arc::new(MemorySettingsStore::new());
        let approval = sample_approval("github_token", "api.github.com");

        grant_binding_approval(Some(store.as_ref()), "alice", approval.clone())
            .await
            .expect("grant approval");
        assert!(
            binding_approval_exists(Some(store.as_ref()), "alice", &approval)
                .await
                .expect("approval exists")
        );

        let approvals = list_binding_approvals(Some(store.as_ref()), "alice")
            .await
            .expect("list approvals");
        assert_eq!(approvals.len(), 1);
        assert_eq!(approvals[0].host, "api.github.com");

        let revoked =
            revoke_binding_approval(Some(store.as_ref()), "alice", &approval.approval_id())
                .await
                .expect("revoke approval");
        assert!(revoked);
        assert!(
            list_binding_approvals(Some(store.as_ref()), "alice")
                .await
                .expect("list approvals after revoke")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn revoke_secret_binding_approvals_only_removes_matching_secret() {
        let store: Arc<dyn SettingsStore + Send + Sync> = Arc::new(MemorySettingsStore::new());
        grant_binding_approval(
            Some(store.as_ref()),
            "alice",
            sample_approval("github_token", "api.github.com"),
        )
        .await
        .expect("grant github approval");
        grant_binding_approval(
            Some(store.as_ref()),
            "alice",
            sample_approval("linear_api_key", "api.linear.app"),
        )
        .await
        .expect("grant linear approval");

        let removed =
            revoke_secret_binding_approvals(Some(store.as_ref()), "alice", "github_token")
                .await
                .expect("revoke secret approvals");
        assert_eq!(removed, 1);

        let approvals = list_binding_approvals(Some(store.as_ref()), "alice")
            .await
            .expect("list remaining approvals");
        assert_eq!(approvals.len(), 1);
        assert_eq!(approvals[0].secret_name, "linear_api_key");
    }
}
