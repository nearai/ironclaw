//! IronClaw Engine — unified thread-capability-CodeAct execution model.
//!
//! This crate provides the core execution engine for IronClaw, unifying
//! ~10 separate abstractions (Session, Job, Routine, Channel, Tool, Skill,
//! Hook, Observer, Extension, LoopDelegate) around 5 primitives:
//!
//! - **Thread** — unit of work (replaces Session + Job + Routine + Sub-agent)
//! - **Step** — unit of execution (replaces agentic loop iteration + tool calls)
//! - **Capability** — unit of effect (replaces Tool + Skill + Hook + Extension)
//! - **MemoryDoc** — unit of durable knowledge (replaces workspace memory blobs)
//! - **Project** — unit of context (replaces flat workspace namespace)
//!
//! The engine defines traits for external dependencies ([`LlmBackend`],
//! [`Store`], [`EffectExecutor`]) that the host crate implements via bridge
//! adapters over existing infrastructure.

pub mod capability;
pub mod executor;
pub mod memory;
pub mod reflection;
pub mod runtime;
pub mod traits;
pub mod types;

// ── Re-exports: types ───────────────────────────────────────

pub use types::capability::{
    ActionDef, Capability, CapabilityLease, EffectType, LeaseId, PolicyCondition, PolicyEffect,
    PolicyRule,
};
pub use types::error::{CapabilityError, EngineError, StepError, ThreadError};
pub use types::event::{EventId, EventKind, ThreadEvent};
pub use types::memory::{DocId, DocType, MemoryDoc};
pub use types::message::{MessageRole, ThreadMessage};
pub use types::project::{Project, ProjectId};
pub use types::provenance::Provenance;
pub use types::step::{
    ActionCall, ActionResult, ExecutionTier, LlmResponse, Step, StepId, StepStatus, TokenUsage,
};
pub use types::thread::{Thread, ThreadConfig, ThreadId, ThreadState, ThreadType};

// ── Re-exports: traits ──────────────────────────────────────

pub use traits::effect::{EffectExecutor, ThreadExecutionContext};
pub use traits::llm::{LlmBackend, LlmCallConfig, LlmOutput};
pub use traits::store::Store;

// ── Re-exports: capability ────────────────────────────────────

pub use capability::registry::CapabilityRegistry;
pub use capability::lease::LeaseManager;
pub use capability::policy::{PolicyDecision, PolicyEngine};

// ── Re-exports: runtime ───────────────────────────────────────

pub use runtime::conversation::ConversationManager;
pub use runtime::manager::ThreadManager;
pub use runtime::messaging::ThreadOutcome;
pub use runtime::tree::ThreadTree;

pub use types::conversation::{
    ConversationEntry, ConversationId, ConversationSurface, EntrySender,
};

// ── Re-exports: executor ──────────────────────────────────────

pub use executor::ExecutionLoop;

// ── Re-exports: memory ────────────────────────────────────────

pub use memory::MemoryStore;
pub use memory::RetrievalEngine;

// ── Re-exports: reflection ────────────────────────────────────

pub use reflection::ReflectionResult;
