use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::llm::{
    LlmConfig, LlmError, LlmProvider, RecordingLlm, SessionManager, build_provider_chain,
};

/// Cloneable snapshot of the active LLM runtime state.
#[derive(Clone)]
pub struct LlmRuntimeSnapshot {
    pub backend: String,
    pub model: String,
    pub llm: Arc<dyn LlmProvider>,
    pub cheap_llm: Option<Arc<dyn LlmProvider>>,
    pub recording_handle: Option<Arc<RecordingLlm>>,
}

impl LlmRuntimeSnapshot {
    pub fn from_parts(
        backend: String,
        llm: Arc<dyn LlmProvider>,
        cheap_llm: Option<Arc<dyn LlmProvider>>,
        recording_handle: Option<Arc<RecordingLlm>>,
    ) -> Self {
        Self {
            backend,
            model: llm.active_model_name(),
            llm,
            cheap_llm,
            recording_handle,
        }
    }
}

/// Shared hot-reloadable LLM runtime.
///
/// Callers clone concrete provider snapshots from this runtime before
/// starting a new turn/request/job so in-flight work keeps using the
/// provider instance it started with.
pub struct LlmRuntime {
    session: Arc<SessionManager>,
    current: RwLock<LlmRuntimeSnapshot>,
}

impl LlmRuntime {
    pub fn new(session: Arc<SessionManager>, snapshot: LlmRuntimeSnapshot) -> Self {
        Self {
            session,
            current: RwLock::new(snapshot),
        }
    }

    pub async fn from_config(
        config: &LlmConfig,
        session: Arc<SessionManager>,
    ) -> Result<Self, LlmError> {
        let snapshot = build_runtime_snapshot(config, session.clone()).await?;
        Ok(Self::new(session, snapshot))
    }

    pub fn snapshot(&self) -> LlmRuntimeSnapshot {
        read_guard(&self.current).clone()
    }

    pub fn current_provider(&self) -> Arc<dyn LlmProvider> {
        Arc::clone(&read_guard(&self.current).llm)
    }

    pub fn current_cheap_provider(&self) -> Option<Arc<dyn LlmProvider>> {
        read_guard(&self.current).cheap_llm.as_ref().map(Arc::clone)
    }

    pub fn current_or_cheap_provider(&self) -> Arc<dyn LlmProvider> {
        let snapshot = self.snapshot();
        snapshot.cheap_llm.unwrap_or(snapshot.llm)
    }

    pub fn current_recording_handle(&self) -> Option<Arc<RecordingLlm>> {
        read_guard(&self.current)
            .recording_handle
            .as_ref()
            .map(Arc::clone)
    }

    pub async fn reload(&self, config: &LlmConfig) -> Result<LlmRuntimeSnapshot, LlmError> {
        let snapshot = build_runtime_snapshot(config, self.session.clone()).await?;
        *write_guard(&self.current) = snapshot.clone();
        Ok(snapshot)
    }
}

pub async fn build_runtime_snapshot(
    config: &LlmConfig,
    session: Arc<SessionManager>,
) -> Result<LlmRuntimeSnapshot, LlmError> {
    let (llm, cheap_llm, recording_handle) = build_provider_chain(config, session).await?;
    Ok(LlmRuntimeSnapshot::from_parts(
        config.backend.clone(),
        llm,
        cheap_llm,
        recording_handle,
    ))
}

fn read_guard<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    match lock.read() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("Recovering from poisoned LLM runtime read lock");
            poisoned.into_inner()
        }
    }
}

fn write_guard<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    match lock.write() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("Recovering from poisoned LLM runtime write lock");
            poisoned.into_inner()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmConfig, SessionConfig};
    use secrecy::SecretString;

    fn openai_config(model: &str) -> LlmConfig {
        LlmConfig {
            backend: "openai".to_string(),
            session: SessionConfig::default(),
            nearai: crate::llm::NearAiConfig {
                model: "unused".to_string(),
                cheap_model: None,
                base_url: "https://private.near.ai".to_string(),
                api_key: None,
                fallback_model: None,
                max_retries: 0,
                circuit_breaker_threshold: None,
                circuit_breaker_recovery_secs: 30,
                response_cache_enabled: false,
                response_cache_ttl_secs: 3600,
                response_cache_max_entries: 1000,
                failover_cooldown_secs: 300,
                failover_cooldown_threshold: 3,
                smart_routing_cascade: true,
            },
            provider: Some(crate::llm::RegistryProviderConfig {
                protocol: crate::llm::ProviderProtocol::OpenAiCompletions,
                provider_id: "openai".to_string(),
                api_key: Some(SecretString::from("sk-test".to_string())),
                base_url: "https://api.openai.com/v1".to_string(),
                model: model.to_string(),
                extra_headers: Vec::new(),
                oauth_token: None,
                is_codex_chatgpt: false,
                refresh_token: None,
                auth_path: None,
                cache_retention: crate::llm::CacheRetention::None,
                unsupported_params: Vec::new(),
            }),
            bedrock: None,
            gemini_oauth: None,
            openai_codex: None,
            request_timeout_secs: 30,
            cheap_model: None,
            smart_routing_cascade: true,
        }
    }

    #[tokio::test]
    async fn reload_swaps_provider_for_new_snapshots() {
        let session = Arc::new(SessionManager::new(SessionConfig::default()));
        let runtime = LlmRuntime::from_config(&openai_config("gpt-4o-mini"), session)
            .await
            .expect("runtime should build");

        let before = runtime.current_provider();
        assert_eq!(before.active_model_name(), "gpt-4o-mini");

        let snapshot = runtime
            .reload(&openai_config("gpt-4.1"))
            .await
            .expect("reload should succeed");

        assert_eq!(snapshot.model, "gpt-4.1");
        assert_eq!(runtime.current_provider().active_model_name(), "gpt-4.1");
        assert_eq!(before.active_model_name(), "gpt-4o-mini");
    }
}
