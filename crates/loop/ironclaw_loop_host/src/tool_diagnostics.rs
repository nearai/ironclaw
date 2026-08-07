use std::sync::Arc;

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
}

/// A bounded result prepared before the full serialized payload moves into
/// durable persistence.
pub struct PreparedToolDiagnosticResult(Option<String>);

impl HostManagedToolDiagnosticEmitter {
    pub fn new(sink: Option<Arc<dyn HostManagedPromptDiagnosticSink>>) -> Self {
        Self { sink }
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
        sink.record_tool_result(HostManagedToolResultDiagnosticCapture {
            context: context.clone(),
            activity_id: activity_id.as_uuid(),
            capability_name: capability_id.as_str().to_string(),
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
        sink.record_tool_result(HostManagedToolResultDiagnosticCapture {
            context: context.clone(),
            activity_id: activity_id.as_uuid(),
            capability_name: capability_id.as_str().to_string(),
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
    use super::bounded_utf8_prefix;

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
}
