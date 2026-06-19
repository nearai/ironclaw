//! Per-run catalog of client-supplied ("external") tools.
//!
//! Transient, run-scoped coordination state for the OpenAI-compatible Responses
//! surface — deliberately NOT part of the durable [`crate::TurnRunState`] or the
//! turn event log. It holds two things keyed by [`TurnRunId`]:
//!
//! - the caller tool *definitions* (so the loop capability host can offer them
//!   to the model), and
//! - client-submitted tool *outputs* keyed by provider call id (so a parked
//!   [`crate::TurnStatus::BlockedExternalTool`] gate can resume by feeding the
//!   output back as the tool result, without re-executing anything host-side).
//!
//! The loop capability host reads specs and takes outputs; the product/Responses
//! layer registers specs at submit and submits outputs on resume. Outputs are
//! removed once taken so a resumed run consumes each submitted output exactly
//! once. This store never persists raw output into the durable turn record — it
//! exists only to bridge a parked external-tool call to its client-supplied
//! result for the lifetime of the run.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::TurnRunId;

/// Maximum accepted external tool name length, in bytes.
const MAX_EXTERNAL_TOOL_NAME_BYTES: usize = 128;
/// Maximum accepted external tool description length, in bytes.
const MAX_EXTERNAL_TOOL_DESCRIPTION_BYTES: usize = 8 * 1024;
/// Maximum accepted serialized parameters-schema length, in bytes.
const MAX_EXTERNAL_TOOL_SCHEMA_BYTES: usize = 64 * 1024;
/// Maximum number of external tools registered for one run.
const MAX_EXTERNAL_TOOLS_PER_RUN: usize = 256;

/// Reason an [`ExternalToolSpec`] failed validation. Stable, user-safe strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalToolSpecError {
    pub reason: &'static str,
}

impl std::fmt::Display for ExternalToolSpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason)
    }
}

impl std::error::Error for ExternalToolSpecError {}

/// A client-declared tool the model may call. The host never executes it: a call
/// parks the run and returns control to the API client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalToolSpec {
    name: String,
    description: String,
    parameters_schema: serde_json::Value,
}

impl ExternalToolSpec {
    /// Validate and construct a spec. Rejects empty/oversized/control-character
    /// names, oversized descriptions, and oversized parameter schemas.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters_schema: serde_json::Value,
    ) -> Result<Self, ExternalToolSpecError> {
        let name = name.into();
        if name.is_empty() {
            return Err(ExternalToolSpecError {
                reason: "external tool name must not be empty",
            });
        }
        if name.len() > MAX_EXTERNAL_TOOL_NAME_BYTES {
            return Err(ExternalToolSpecError {
                reason: "external tool name is too long",
            });
        }
        if name.trim() != name {
            return Err(ExternalToolSpecError {
                reason: "external tool name must not have surrounding whitespace",
            });
        }
        if name.chars().any(char::is_control) {
            return Err(ExternalToolSpecError {
                reason: "external tool name must not contain control characters",
            });
        }
        let description = description.into();
        if description.len() > MAX_EXTERNAL_TOOL_DESCRIPTION_BYTES {
            return Err(ExternalToolSpecError {
                reason: "external tool description is too long",
            });
        }
        let schema_len = serde_json::to_string(&parameters_schema)
            .map(|s| s.len())
            .unwrap_or(usize::MAX);
        if schema_len > MAX_EXTERNAL_TOOL_SCHEMA_BYTES {
            return Err(ExternalToolSpecError {
                reason: "external tool parameters schema is too large",
            });
        }
        Ok(Self {
            name,
            description,
            parameters_schema,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn parameters_schema(&self) -> &serde_json::Value {
        &self.parameters_schema
    }
}

/// A model-invoked external tool call that parked the run, recorded by the loop
/// host at invocation time. The OpenAI-compatible Responses surface reads these
/// back to render a parked [`crate::TurnStatus::BlockedExternalTool`] run's
/// pending call as a `function_call` output item — the call's
/// name/arguments/`call_id` are otherwise only in the loop checkpoint, which has
/// no external read path. Cleared once the client output is consumed on resume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingExternalCall {
    call_id: String,
    name: String,
    arguments: serde_json::Value,
}

impl PendingExternalCall {
    pub fn new(
        call_id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Self {
        Self {
            call_id: call_id.into(),
            name: name.into(),
            arguments,
        }
    }

    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn arguments(&self) -> &serde_json::Value {
        &self.arguments
    }
}

/// Error surface for [`ExternalToolCatalog`] operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalToolCatalogError {
    /// The backing store is unavailable (lock poisoned, backend down).
    Unavailable,
    /// A register request exceeded the per-run tool cap or contained duplicate
    /// tool names.
    InvalidRegistration { reason: &'static str },
}

impl std::fmt::Display for ExternalToolCatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => f.write_str("external tool catalog unavailable"),
            Self::InvalidRegistration { reason } => f.write_str(reason),
        }
    }
}

impl std::error::Error for ExternalToolCatalogError {}

/// Host-side, run-scoped catalog of client-supplied tools and their submitted
/// outputs. See the module docs for the lifecycle.
#[async_trait]
pub trait ExternalToolCatalog: Send + Sync {
    /// Register (replacing any prior set) the external tools for a run. An empty
    /// list clears the run's tools.
    async fn register(
        &self,
        run_id: TurnRunId,
        specs: Vec<ExternalToolSpec>,
    ) -> Result<(), ExternalToolCatalogError>;

    /// The external tools the model may call for this run.
    async fn specs(
        &self,
        run_id: TurnRunId,
    ) -> Result<Vec<ExternalToolSpec>, ExternalToolCatalogError>;

    /// Bind a loop capability `input_ref` to the provider `call_id` the client
    /// will reference. Recorded by the loop host when the model invokes an
    /// external tool, so a submitted output (keyed by `call_id`) can later be
    /// matched to the parked invocation (keyed by `input_ref`) on re-dispatch.
    async fn bind_call(
        &self,
        run_id: TurnRunId,
        input_ref: String,
        call_id: String,
    ) -> Result<(), ExternalToolCatalogError>;

    /// The provider `call_id` previously bound to an `input_ref`, if any.
    async fn call_id_for_input_ref(
        &self,
        run_id: TurnRunId,
        input_ref: &str,
    ) -> Result<Option<String>, ExternalToolCatalogError>;

    /// Record a client-submitted output for a parked external tool call.
    async fn submit_output(
        &self,
        run_id: TurnRunId,
        call_id: String,
        output: serde_json::Value,
    ) -> Result<(), ExternalToolCatalogError>;

    /// Take (remove) a previously submitted output for a call, if present. The
    /// host calls this when re-dispatching a resumed external-tool call so each
    /// output is consumed once.
    async fn take_output(
        &self,
        run_id: TurnRunId,
        call_id: &str,
    ) -> Result<Option<serde_json::Value>, ExternalToolCatalogError>;

    /// Resolve the `call_id` bound to `input_ref`, then take its submitted
    /// output if present. Convenience for the loop host, which knows the parked
    /// invocation's `input_ref` but not the client-facing `call_id`.
    async fn take_output_for_input_ref(
        &self,
        run_id: TurnRunId,
        input_ref: &str,
    ) -> Result<Option<serde_json::Value>, ExternalToolCatalogError> {
        let Some(call_id) = self.call_id_for_input_ref(run_id, input_ref).await? else {
            return Ok(None);
        };
        self.take_output(run_id, &call_id).await
    }

    /// Record (replacing any prior record with the same `call_id`) a model-invoked
    /// external tool call that parked the run. The loop host calls this when it
    /// registers the provider tool call, so the parked invocation's
    /// name/arguments/`call_id` survive in run-scoped state the Responses surface
    /// can read — the loop checkpoint that also holds them has no external read
    /// path. Insertion order is preserved across distinct `call_id`s so a run that
    /// parked on several calls renders them deterministically.
    async fn record_pending_call(
        &self,
        run_id: TurnRunId,
        call: PendingExternalCall,
    ) -> Result<(), ExternalToolCatalogError>;

    /// The external tool calls that have parked this run and not yet been
    /// completed, in invocation order. Read by the Responses surface to render a
    /// parked `BlockedExternalTool` run's pending `function_call` items.
    async fn pending_calls(
        &self,
        run_id: TurnRunId,
    ) -> Result<Vec<PendingExternalCall>, ExternalToolCatalogError>;

    /// Drop the pending-call record for `call_id`. The loop host calls this when a
    /// resumed external-tool call completes (its client output was consumed) so a
    /// run that parks again on a later call does not re-surface the resolved one.
    async fn clear_pending_call(
        &self,
        run_id: TurnRunId,
        call_id: &str,
    ) -> Result<(), ExternalToolCatalogError>;

    /// Drop all catalog state for a run. Called when the run reaches a terminal
    /// state so abandoned runs do not leak.
    async fn clear(&self, run_id: TurnRunId) -> Result<(), ExternalToolCatalogError>;
}

#[derive(Default)]
struct RunEntry {
    specs: Vec<ExternalToolSpec>,
    /// Client-submitted outputs keyed by provider `call_id`.
    outputs: HashMap<String, serde_json::Value>,
    /// `input_ref` → `call_id` bindings for parked external-tool invocations.
    call_ids_by_input_ref: HashMap<String, String>,
    /// Parked external-tool calls (name/arguments/`call_id`), in invocation
    /// order, deduplicated by `call_id`. Surfaced to the Responses API as the
    /// pending `function_call` items of a `BlockedExternalTool` run.
    pending_calls: Vec<PendingExternalCall>,
}

/// In-memory [`ExternalToolCatalog`] for local-dev / single-process Reborn.
#[derive(Default)]
pub struct InMemoryExternalToolCatalog {
    runs: Mutex<HashMap<TurnRunId, RunEntry>>,
}

impl InMemoryExternalToolCatalog {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ExternalToolCatalog for InMemoryExternalToolCatalog {
    async fn register(
        &self,
        run_id: TurnRunId,
        specs: Vec<ExternalToolSpec>,
    ) -> Result<(), ExternalToolCatalogError> {
        if specs.len() > MAX_EXTERNAL_TOOLS_PER_RUN {
            return Err(ExternalToolCatalogError::InvalidRegistration {
                reason: "too many external tools for one run",
            });
        }
        let mut seen = std::collections::HashSet::with_capacity(specs.len());
        for spec in &specs {
            if !seen.insert(spec.name()) {
                return Err(ExternalToolCatalogError::InvalidRegistration {
                    reason: "duplicate external tool name",
                });
            }
        }
        let mut runs = self
            .runs
            .lock()
            .map_err(|_| ExternalToolCatalogError::Unavailable)?;
        if specs.is_empty() {
            runs.remove(&run_id);
            return Ok(());
        }
        let entry = runs.entry(run_id).or_default();
        entry.specs = specs;
        Ok(())
    }

    async fn specs(
        &self,
        run_id: TurnRunId,
    ) -> Result<Vec<ExternalToolSpec>, ExternalToolCatalogError> {
        let runs = self
            .runs
            .lock()
            .map_err(|_| ExternalToolCatalogError::Unavailable)?;
        Ok(runs
            .get(&run_id)
            .map(|entry| entry.specs.clone())
            .unwrap_or_default())
    }

    async fn bind_call(
        &self,
        run_id: TurnRunId,
        input_ref: String,
        call_id: String,
    ) -> Result<(), ExternalToolCatalogError> {
        let mut runs = self
            .runs
            .lock()
            .map_err(|_| ExternalToolCatalogError::Unavailable)?;
        runs.entry(run_id)
            .or_default()
            .call_ids_by_input_ref
            .insert(input_ref, call_id);
        Ok(())
    }

    async fn call_id_for_input_ref(
        &self,
        run_id: TurnRunId,
        input_ref: &str,
    ) -> Result<Option<String>, ExternalToolCatalogError> {
        let runs = self
            .runs
            .lock()
            .map_err(|_| ExternalToolCatalogError::Unavailable)?;
        Ok(runs
            .get(&run_id)
            .and_then(|entry| entry.call_ids_by_input_ref.get(input_ref).cloned()))
    }

    async fn submit_output(
        &self,
        run_id: TurnRunId,
        call_id: String,
        output: serde_json::Value,
    ) -> Result<(), ExternalToolCatalogError> {
        let mut runs = self
            .runs
            .lock()
            .map_err(|_| ExternalToolCatalogError::Unavailable)?;
        runs.entry(run_id)
            .or_default()
            .outputs
            .insert(call_id, output);
        Ok(())
    }

    async fn take_output(
        &self,
        run_id: TurnRunId,
        call_id: &str,
    ) -> Result<Option<serde_json::Value>, ExternalToolCatalogError> {
        let mut runs = self
            .runs
            .lock()
            .map_err(|_| ExternalToolCatalogError::Unavailable)?;
        Ok(runs
            .get_mut(&run_id)
            .and_then(|entry| entry.outputs.remove(call_id)))
    }

    async fn record_pending_call(
        &self,
        run_id: TurnRunId,
        call: PendingExternalCall,
    ) -> Result<(), ExternalToolCatalogError> {
        let mut runs = self
            .runs
            .lock()
            .map_err(|_| ExternalToolCatalogError::Unavailable)?;
        let pending = &mut runs.entry(run_id).or_default().pending_calls;
        // Replace in place on re-record (same call id re-dispatched) so order is
        // stable; otherwise append.
        if let Some(existing) = pending
            .iter_mut()
            .find(|existing| existing.call_id() == call.call_id())
        {
            *existing = call;
        } else {
            pending.push(call);
        }
        Ok(())
    }

    async fn pending_calls(
        &self,
        run_id: TurnRunId,
    ) -> Result<Vec<PendingExternalCall>, ExternalToolCatalogError> {
        let runs = self
            .runs
            .lock()
            .map_err(|_| ExternalToolCatalogError::Unavailable)?;
        Ok(runs
            .get(&run_id)
            .map(|entry| entry.pending_calls.clone())
            .unwrap_or_default())
    }

    async fn clear_pending_call(
        &self,
        run_id: TurnRunId,
        call_id: &str,
    ) -> Result<(), ExternalToolCatalogError> {
        let mut runs = self
            .runs
            .lock()
            .map_err(|_| ExternalToolCatalogError::Unavailable)?;
        if let Some(entry) = runs.get_mut(&run_id) {
            entry.pending_calls.retain(|call| call.call_id() != call_id);
        }
        Ok(())
    }

    async fn clear(&self, run_id: TurnRunId) -> Result<(), ExternalToolCatalogError> {
        let mut runs = self
            .runs
            .lock()
            .map_err(|_| ExternalToolCatalogError::Unavailable)?;
        runs.remove(&run_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str) -> ExternalToolSpec {
        ExternalToolSpec::new(name, "desc", serde_json::json!({"type": "object"})).expect("spec")
    }

    #[test]
    fn spec_validation_rejects_bad_names_and_oversized_fields() {
        assert!(ExternalToolSpec::new("", "d", serde_json::json!({})).is_err());
        assert!(ExternalToolSpec::new(" x", "d", serde_json::json!({})).is_err());
        assert!(ExternalToolSpec::new("x\u{0000}", "d", serde_json::json!({})).is_err());
        let long_name = "n".repeat(MAX_EXTERNAL_TOOL_NAME_BYTES + 1);
        assert!(ExternalToolSpec::new(long_name, "d", serde_json::json!({})).is_err());
        let big_desc = "d".repeat(MAX_EXTERNAL_TOOL_DESCRIPTION_BYTES + 1);
        assert!(ExternalToolSpec::new("ok", big_desc, serde_json::json!({})).is_err());
        assert!(ExternalToolSpec::new("ok", "d", serde_json::json!({"type": "object"})).is_ok());
    }

    #[tokio::test]
    async fn register_specs_and_read_back() {
        let catalog = InMemoryExternalToolCatalog::new();
        let run = TurnRunId::new();
        catalog
            .register(run, vec![spec("get_weather"), spec("search")])
            .await
            .expect("register");
        let specs = catalog.specs(run).await.expect("specs");
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].name(), "get_weather");
        // Unknown run yields no specs.
        assert!(
            catalog
                .specs(TurnRunId::new())
                .await
                .expect("empty")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn register_rejects_duplicate_names_and_overflow() {
        let catalog = InMemoryExternalToolCatalog::new();
        let run = TurnRunId::new();
        let dup = catalog
            .register(run, vec![spec("a"), spec("a")])
            .await
            .unwrap_err();
        assert!(matches!(
            dup,
            ExternalToolCatalogError::InvalidRegistration { .. }
        ));
        let too_many: Vec<_> = (0..=MAX_EXTERNAL_TOOLS_PER_RUN)
            .map(|i| spec(&format!("tool{i}")))
            .collect();
        assert!(catalog.register(run, too_many).await.is_err());
    }

    #[tokio::test]
    async fn submit_and_take_output_is_once_only() {
        let catalog = InMemoryExternalToolCatalog::new();
        let run = TurnRunId::new();
        catalog
            .submit_output(run, "call_1".to_string(), serde_json::json!("72F"))
            .await
            .expect("submit");
        let first = catalog.take_output(run, "call_1").await.expect("take");
        assert_eq!(first, Some(serde_json::json!("72F")));
        // Consumed: a second take yields nothing.
        let second = catalog.take_output(run, "call_1").await.expect("take");
        assert_eq!(second, None);
    }

    #[tokio::test]
    async fn bind_then_take_output_for_input_ref_resolves_via_call_id() {
        let catalog = InMemoryExternalToolCatalog::new();
        let run = TurnRunId::new();
        catalog
            .bind_call(run, "input-1".to_string(), "call_abc".to_string())
            .await
            .expect("bind");
        // Output submitted by the client-facing call id...
        catalog
            .submit_output(run, "call_abc".to_string(), serde_json::json!("sunny"))
            .await
            .expect("submit");
        // ...is taken by the host using the parked invocation's input_ref.
        let taken = catalog
            .take_output_for_input_ref(run, "input-1")
            .await
            .expect("take");
        assert_eq!(taken, Some(serde_json::json!("sunny")));
        // Consumed once.
        assert_eq!(
            catalog
                .take_output_for_input_ref(run, "input-1")
                .await
                .expect("take"),
            None
        );
        // Unknown input_ref resolves to nothing.
        assert_eq!(
            catalog
                .take_output_for_input_ref(run, "input-unknown")
                .await
                .expect("take"),
            None
        );
    }

    #[tokio::test]
    async fn clear_drops_run_state() {
        let catalog = InMemoryExternalToolCatalog::new();
        let run = TurnRunId::new();
        catalog
            .register(run, vec![spec("a")])
            .await
            .expect("register");
        catalog
            .submit_output(run, "call_1".to_string(), serde_json::json!(1))
            .await
            .expect("submit");
        catalog.clear(run).await.expect("clear");
        assert!(catalog.specs(run).await.expect("specs").is_empty());
        assert_eq!(
            catalog.take_output(run, "call_1").await.expect("take"),
            None
        );
    }

    #[tokio::test]
    async fn pending_calls_record_read_and_clear_preserve_order() {
        let catalog = InMemoryExternalToolCatalog::new();
        let run = TurnRunId::new();
        catalog
            .record_pending_call(
                run,
                PendingExternalCall::new(
                    "call_1",
                    "get_weather",
                    serde_json::json!({"city": "SF"}),
                ),
            )
            .await
            .expect("record 1");
        catalog
            .record_pending_call(
                run,
                PendingExternalCall::new("call_2", "search", serde_json::json!({"q": "rust"})),
            )
            .await
            .expect("record 2");

        let pending = catalog.pending_calls(run).await.expect("pending");
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].call_id(), "call_1");
        assert_eq!(pending[0].name(), "get_weather");
        assert_eq!(pending[0].arguments(), &serde_json::json!({"city": "SF"}));
        assert_eq!(pending[1].call_id(), "call_2");

        // Re-recording the same call id replaces in place without reordering.
        catalog
            .record_pending_call(
                run,
                PendingExternalCall::new(
                    "call_1",
                    "get_weather",
                    serde_json::json!({"city": "NYC"}),
                ),
            )
            .await
            .expect("re-record");
        let pending = catalog.pending_calls(run).await.expect("pending");
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].call_id(), "call_1");
        assert_eq!(pending[0].arguments(), &serde_json::json!({"city": "NYC"}));

        // Clearing one leaves the rest.
        catalog
            .clear_pending_call(run, "call_1")
            .await
            .expect("clear one");
        let pending = catalog.pending_calls(run).await.expect("pending");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].call_id(), "call_2");

        // Unknown run yields no pending calls.
        assert!(
            catalog
                .pending_calls(TurnRunId::new())
                .await
                .expect("empty")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn clear_run_drops_pending_calls() {
        let catalog = InMemoryExternalToolCatalog::new();
        let run = TurnRunId::new();
        catalog
            .record_pending_call(
                run,
                PendingExternalCall::new("call_1", "t", serde_json::json!({})),
            )
            .await
            .expect("record");
        catalog.clear(run).await.expect("clear");
        assert!(
            catalog
                .pending_calls(run)
                .await
                .expect("pending")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn empty_register_clears_specs() {
        let catalog = InMemoryExternalToolCatalog::new();
        let run = TurnRunId::new();
        catalog
            .register(run, vec![spec("a")])
            .await
            .expect("register");
        catalog
            .register(run, vec![])
            .await
            .expect("clear via empty");
        assert!(catalog.specs(run).await.expect("specs").is_empty());
    }
}
