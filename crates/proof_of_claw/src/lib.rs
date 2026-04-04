//! Proof of Claw — provable execution, ZK proofs, and hardware approval
//! for IronClaw agents.
//!
//! This crate adds the following on top of a base IronClaw runtime:
//!
//! - **Provable execution** — RISC Zero zkVM proofs of policy compliance
//! - **Private inference** — 0G Compute for decentralized LLM reasoning
//! - **Decentralized storage** — 0G Storage for execution trace persistence
//! - **Encrypted messaging** — DM3 protocol for inter-agent communication
//! - **Hardware approval** — Ledger DMK/DSK for high-value action gating
//! - **On-chain identity** — ERC-7857 iNFT + EIP-8004 registries
//!
//! ## Integration
//!
//! Register the POC hooks with an ironclaw `HookRegistry`:
//!
//! ```ignore
//! use proof_of_claw::{IronClawAdapter, InjectionDetectionHook, PolicyEnforcementHook, ProofGenerationHook};
//! use std::sync::Arc;
//!
//! let adapter = Arc::new(IronClawAdapter::new(config).await?);
//! registry.register(Arc::new(InjectionDetectionHook::new(adapter.injection_detector.clone()))).await;
//! registry.register(Arc::new(PolicyEnforcementHook::new(adapter.policy_engine.clone()))).await;
//! registry.register(Arc::new(ProofGenerationHook::new(adapter.clone()))).await;
//! ```

#![allow(missing_docs)]

pub mod config;
pub mod eip8004;
pub mod ens_dm3;
pub mod injection_detector;
pub mod inft;
pub mod ironclaw_adapter;
pub mod job_scheduler;
pub mod ledger;
pub mod policy_engine;
pub mod proof_generator;
pub mod registry;
pub mod types;
pub mod zero_g;

// Re-exports for convenience
pub use config::{AgentConfig, PolicyConfig};
pub use injection_detector::InjectionDetector;
pub use ironclaw_adapter::IronClawAdapter;
pub use policy_engine::PolicyEngine;
pub use proof_generator::ProofGenerator;
pub use registry::{compute_capability_hash, Tool, ToolRegistry};
pub use zero_g::{ZeroGCompute, ZeroGStorage};
pub use types::{
    AgentMessage, AgentPolicy, ExecutionTrace, InferenceRequest, InferenceResponse,
    MessagePayload, MessageType, PolicyResult, PolicySeverity, ProofReceipt, ToolInvocation,
    VerifiedOutput,
};

// Ironclaw hook implementations — only available when ironclaw feature is enabled
#[cfg(feature = "ironclaw")]
pub use ironclaw_adapter::{
    InjectionDetectionHook, PolicyEnforcementHook, ProofGenerationHook,
};
