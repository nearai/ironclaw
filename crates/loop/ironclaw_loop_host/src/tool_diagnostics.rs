use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Instant,
};

use ironclaw_host_api::ids::{CapabilityId, InvocationId};
use ironclaw_loop_contracts::{CapabilityInputRef, LoopRunContext};

use crate::{
    HostManagedPromptDiagnosticSink, HostManagedToolFailureCategory,
    HostManagedToolInputDiagnosticCapture, HostManagedToolResultDiagnosticCapture,
    HostManagedToolResultDiagnosticStatus, HostManagedToolStartedDiagnosticCapture,
};

/// Emits best-effort tool diagnostics without making an app assembly owner
/// construct or interpret host diagnostic records.
#[derive(Clone, Default)]
pub struct HostManagedToolDiagnosticEmitter {
    sink: Option<Arc<dyn HostManagedPromptDiagnosticSink>>,
    timings: Arc<ToolInvocationTimings>,
}

// Timing is best-effort diagnostic state, not execution authority. Keep a
// per-emitter ceiling so abandoned or dropped terminal captures cannot grow
// the correlation map without bound.
const MAX_TRACKED_TOOL_INVOCATION_TIMINGS: usize = 1_024;

#[derive(Default)]
struct ToolInvocationTimingState {
    started_at: HashMap<InvocationId, Instant>,
    order: VecDeque<InvocationId>,
}

#[derive(Default)]
struct ToolInvocationTimings {
    state: Mutex<ToolInvocationTimingState>,
}

impl ToolInvocationTimings {
    fn record_started_at(&self, activity_id: InvocationId, started_at: Instant) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if !state.started_at.contains_key(&activity_id) {
            while state.started_at.len() >= MAX_TRACKED_TOOL_INVOCATION_TIMINGS {
                let Some(evicted) = state.order.pop_front() else {
                    return;
                };
                state.started_at.remove(&evicted);
            }
        }
        if let Some(index) = state.order.iter().position(|entry| entry == &activity_id) {
            state.order.remove(index);
        }
        state.order.push_back(activity_id);
        state.started_at.insert(activity_id, started_at);
    }

    fn take_duration_ms_at(&self, activity_id: InvocationId, completed_at: Instant) -> Option<u64> {
        let mut state = self.state.lock().ok()?;
        if let Some(index) = state.order.iter().position(|entry| entry == &activity_id) {
            state.order.remove(index);
        }
        let started_at = state.started_at.remove(&activity_id)?;
        completed_at
            .checked_duration_since(started_at)
            .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
    }
}

/// A bounded result prepared before the full serialized payload moves into
/// durable persistence.
pub struct PreparedToolDiagnosticResult(Option<String>);

impl HostManagedToolDiagnosticEmitter {
    pub fn new(sink: Option<Arc<dyn HostManagedPromptDiagnosticSink>>) -> Self {
        Self {
            sink,
            timings: Arc::default(),
        }
    }

    pub fn record_input(
        &self,
        context: &LoopRunContext,
        input_ref: &CapabilityInputRef,
        capability_name: &str,
        arguments: &serde_json::Value,
    ) {
        let Some(sink) = &self.sink else {
            return;
        };
        sink.record_tool_input(HostManagedToolInputDiagnosticCapture {
            context: context.clone(),
            input_ref: input_ref.as_str().to_string(),
            capability_name: capability_name.to_string(),
            arguments: arguments.clone(),
        });
    }

    pub fn record_started(
        &self,
        context: &LoopRunContext,
        activity_id: InvocationId,
        input_ref: &CapabilityInputRef,
    ) {
        let Some(sink) = &self.sink else {
            return;
        };
        self.timings.record_started_at(activity_id, Instant::now());
        sink.record_tool_started(HostManagedToolStartedDiagnosticCapture {
            context: context.clone(),
            activity_id: activity_id.as_uuid(),
            input_ref: input_ref.as_str().to_string(),
        });
    }

    pub fn prepare_result(
        &self,
        serialized: &[u8],
        max_capture_bytes: usize,
    ) -> Option<PreparedToolDiagnosticResult> {
        self.sink.as_ref()?;
        Some(PreparedToolDiagnosticResult(bounded_utf8_prefix(
            serialized,
            max_capture_bytes,
        )))
    }

    pub fn record_succeeded(
        &self,
        context: &LoopRunContext,
        activity_id: InvocationId,
        capability_id: &CapabilityId,
        prepared: Option<PreparedToolDiagnosticResult>,
        original_bytes: u64,
    ) {
        let (Some(sink), Some(PreparedToolDiagnosticResult(result))) = (&self.sink, prepared)
        else {
            return;
        };
        let duration_ms = self
            .timings
            .take_duration_ms_at(activity_id, Instant::now());
        sink.record_tool_result(HostManagedToolResultDiagnosticCapture {
            context: context.clone(),
            activity_id: activity_id.as_uuid(),
            capability_name: capability_id.as_str().to_string(),
            duration_ms,
            result,
            result_original_bytes: Some(original_bytes),
            status: HostManagedToolResultDiagnosticStatus::Succeeded,
            failure_category: None,
            failure_summary: None,
        });
    }

    pub fn record_failed(
        &self,
        context: &LoopRunContext,
        activity_id: InvocationId,
        capability_id: &CapabilityId,
        summary: &str,
    ) {
        let Some(sink) = &self.sink else {
            return;
        };
        let duration_ms = self
            .timings
            .take_duration_ms_at(activity_id, Instant::now());
        sink.record_tool_result(HostManagedToolResultDiagnosticCapture {
            context: context.clone(),
            activity_id: activity_id.as_uuid(),
            capability_name: capability_id.as_str().to_string(),
            duration_ms,
            result: None,
            result_original_bytes: None,
            status: HostManagedToolResultDiagnosticStatus::Failed,
            failure_category: Some(HostManagedToolFailureCategory::CapabilityFailed),
            failure_summary: Some(summary.to_string()),
        });
    }
}

fn bounded_utf8_prefix(serialized: &[u8], max_bytes: usize) -> Option<String> {
    let candidate = &serialized[..serialized.len().min(max_bytes)];
    match std::str::from_utf8(candidate) {
        Ok(text) => Some(text.to_owned()),
        Err(error) if error.error_len().is_none() => {
            let valid_prefix = &candidate[..error.valid_up_to()];
            std::str::from_utf8(valid_prefix).ok().map(str::to_owned)
        }
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use ironclaw_host_api::ids::InvocationId;

    use super::{ToolInvocationTimings, bounded_utf8_prefix};

    #[test]
    fn bounded_result_capture_limits_copied_bytes() {
        let payload = "x".repeat(256);
        let captured = bounded_utf8_prefix(payload.as_bytes(), 64).expect("valid UTF-8 prefix");
        assert_eq!(captured.len(), 64);
    }

    #[test]
    fn bounded_result_capture_drops_a_split_utf8_suffix() {
        let mut payload = "a".repeat(63);
        payload.push('€');
        payload.push_str("tail");

        let captured = bounded_utf8_prefix(payload.as_bytes(), 64).expect("valid UTF-8 prefix");
        assert_eq!(captured.len(), 63);
        assert!(captured.is_char_boundary(captured.len()));
    }

    #[test]
    fn tool_invocation_timings_measure_once_with_a_monotonic_clock() {
        let timings = ToolInvocationTimings::default();
        let activity_id = InvocationId::new();
        let started_at = Instant::now();
        timings.record_started_at(activity_id, started_at);

        assert_eq!(
            timings.take_duration_ms_at(activity_id, started_at + Duration::from_millis(42)),
            Some(42)
        );
        assert_eq!(
            timings.take_duration_ms_at(activity_id, started_at + Duration::from_millis(84)),
            None,
            "terminal capture must consume the timing entry"
        );
    }
}
