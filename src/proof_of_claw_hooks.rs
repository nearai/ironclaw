//! Register Proof of Claw hooks (0G-backed trace storage, policy, injection) with IronClaw.
//!
//! Enable at runtime with `PROOF_OF_CLAW_ENABLED=1`. Requires the same env vars as
//! [`proof_of_claw::AgentConfig::from_env`] (`ZERO_G_COMPUTE_ENDPOINT`, `ZERO_G_INDEXER_RPC`, etc.).

use std::sync::Arc;

use crate::hooks::HookRegistry;
use proof_of_claw::{
    AgentConfig, InjectionDetectionHook, InjectionDetector, IronClawAdapter,
    PolicyEnforcementHook, PolicyEngine, ProofGenerationHook,
};

/// Register Proof of Claw lifecycle hooks on the shared [`HookRegistry`].
pub async fn register(registry: &Arc<HookRegistry>) -> anyhow::Result<()> {
    let config = AgentConfig::from_env()?;
    let adapter = Arc::new(IronClawAdapter::new(config.clone()).await?);

    registry
        .register_with_priority(
            Arc::new(InjectionDetectionHook::new(InjectionDetector::new())),
            15,
        )
        .await;
    registry
        .register_with_priority(
            Arc::new(PolicyEnforcementHook::new(PolicyEngine::new(
                config.policy.clone(),
            ))),
            20,
        )
        .await;
    registry
        .register(Arc::new(ProofGenerationHook::new(adapter)))
        .await;

    tracing::info!("proof_of_claw: registered injection, policy, and session-end (0G) hooks");
    Ok(())
}
