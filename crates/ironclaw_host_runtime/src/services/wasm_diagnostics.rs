use ironclaw_host_api::ids::CapabilityId;
use ironclaw_wasm::{WasmError, WasmLogLevel, WasmLogRecord};

pub(super) fn log_wasm_runtime_error(capability_id: &CapabilityId, error: &WasmError) {
    if let WasmError::ExecutionFailed { message, logs, .. } = error {
        log_wasm_guest_logs(capability_id, logs);
        tracing::debug!(
            capability_id = %capability_id,
            wasm_error = %message,
            "WASM runtime execution failed with raw guest error"
        );
        return;
    }

    tracing::debug!(
        capability_id = %capability_id,
        wasm_error = %error,
        "WASM runtime execution failed"
    );
}

pub(super) fn log_wasm_guest_error(
    capability_id: &CapabilityId,
    logs: &[WasmLogRecord],
    error: &str,
) {
    log_wasm_guest_logs(capability_id, logs);
    tracing::debug!(
        capability_id = %capability_id,
        wasm_error = %error,
        "WASM guest returned raw capability error"
    );
}

fn log_wasm_guest_logs(capability_id: &CapabilityId, logs: &[WasmLogRecord]) {
    for log in logs {
        match log.level {
            WasmLogLevel::Trace => tracing::trace!(
                capability_id = %capability_id,
                wasm_log = %log.message,
                "WASM guest log"
            ),
            WasmLogLevel::Debug => tracing::debug!(
                capability_id = %capability_id,
                wasm_log = %log.message,
                "WASM guest log"
            ),
            WasmLogLevel::Info => tracing::info!(
                capability_id = %capability_id,
                wasm_log = %log.message,
                "WASM guest log"
            ),
            WasmLogLevel::Warn => tracing::warn!(
                capability_id = %capability_id,
                wasm_log = %log.message,
                "WASM guest log"
            ),
            WasmLogLevel::Error => tracing::error!(
                capability_id = %capability_id,
                wasm_log = %log.message,
                "WASM guest log"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use ironclaw_host_api::resource::ResourceUsage;
    use tracing::field::{Field, Visit};
    use tracing::{Event, Level, Subscriber};
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::registry::LookupSpan;
    use tracing_subscriber::{Layer, Registry};

    use super::*;

    const DETECTABLE_SECRET: &str = "AKIAIOSFODNN7EXAMPLE";
    const DIAGNOSTIC_REDACTION_MARKER: &str = "[WASM_DIAGNOSTIC_REDACTED]";
    const DIAGNOSTIC_MAX_BYTES: usize = 4096;
    const DIAGNOSTIC_TARGET: &str = "ironclaw_host_runtime::services::wasm_diagnostics";

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct CapturedEvent {
        level: Level,
        target: String,
        fields: BTreeMap<String, String>,
    }

    #[derive(Clone, Default)]
    struct CapturingLayer {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    impl<S> Layer<S> for CapturingLayer
    where
        S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    {
        fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
            let mut visitor = FieldVisitor::default();
            event.record(&mut visitor);
            self.events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(CapturedEvent {
                    level: *event.metadata().level(),
                    target: event.metadata().target().to_string(),
                    fields: visitor.fields,
                });
        }
    }

    #[derive(Default)]
    struct FieldVisitor {
        fields: BTreeMap<String, String>,
    }

    impl Visit for FieldVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.fields
                .insert(field.name().to_string(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }
    }

    fn capability_id() -> CapabilityId {
        CapabilityId::new("test.wasm-diagnostics").expect("valid test capability id")
    }

    fn capture_events(action: impl FnOnce()) -> Vec<CapturedEvent> {
        let layer = CapturingLayer::default();
        let events = Arc::clone(&layer.events);
        let subscriber = Registry::default().with(layer);
        tracing::subscriber::with_default(subscriber, action);
        Arc::try_unwrap(events)
            .expect("capture is no longer shared")
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn field<'a>(event: &'a CapturedEvent, name: &str) -> &'a str {
        event
            .fields
            .get(name)
            .map(String::as_str)
            .unwrap_or_else(|| panic!("event is missing {name}: {event:?}"))
    }

    fn diagnostic_fields(events: &[CapturedEvent]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|event| {
                event
                    .fields
                    .get("wasm_log")
                    .or_else(|| event.fields.get("wasm_error"))
                    .map(String::as_str)
            })
            .collect()
    }

    #[test]
    fn host_tracing_sanitizes_every_raw_wasm_diagnostic_source() {
        let capability_id = capability_id();
        let levels = [
            WasmLogLevel::Trace,
            WasmLogLevel::Debug,
            WasmLogLevel::Info,
            WasmLogLevel::Warn,
            WasmLogLevel::Error,
        ];
        let guest_logs = levels
            .into_iter()
            .map(|level| WasmLogRecord {
                level,
                message: format!("guest supplied {DETECTABLE_SECRET}"),
            })
            .collect::<Vec<_>>();
        let execution_error = WasmError::ExecutionFailed {
            message: format!("trap contained {DETECTABLE_SECRET}"),
            usage: ResourceUsage::default(),
            logs: guest_logs,
        };
        let legacy_error =
            WasmError::CompilationFailed(format!("legacy diagnostic {DETECTABLE_SECRET}"));

        let events = capture_events(|| {
            log_wasm_runtime_error(&capability_id, &execution_error);
            log_wasm_guest_error(
                &capability_id,
                &[],
                &format!("guest response {DETECTABLE_SECRET}"),
            );
            log_wasm_runtime_error(&capability_id, &legacy_error);
        });

        assert_eq!(events.len(), 8, "every diagnostic must retain its route");
        assert!(
            events.iter().all(|event| event.target == DIAGNOSTIC_TARGET),
            "sanitization must not reroute WASM diagnostics: {events:?}"
        );
        assert!(
            events
                .iter()
                .all(|event| field(event, "capability_id") == capability_id.as_str()),
            "sanitization must retain capability routing: {events:?}"
        );
        assert_eq!(
            events
                .iter()
                .take(5)
                .map(|event| event.level)
                .collect::<Vec<_>>(),
            vec![
                Level::TRACE,
                Level::DEBUG,
                Level::INFO,
                Level::WARN,
                Level::ERROR,
            ],
            "guest log levels must survive sanitization"
        );

        let diagnostics = diagnostic_fields(&events);
        assert_eq!(diagnostics.len(), events.len());
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.contains(DETECTABLE_SECRET)),
            "a detectable credential reached host tracing: {events:?}"
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.contains(DIAGNOSTIC_REDACTION_MARKER)),
            "each unsafe cause must be replaced with the stable marker: {events:?}"
        );
    }

    #[test]
    fn host_tracing_preserves_benign_wasm_diagnostics_and_safe_event_wording() {
        let capability_id = capability_id();
        let execution_error = WasmError::ExecutionFailed {
            message: "benign execution detail".to_string(),
            usage: ResourceUsage::default(),
            logs: vec![WasmLogRecord {
                level: WasmLogLevel::Info,
                message: "benign guest log".to_string(),
            }],
        };
        let legacy_error = WasmError::CompilationFailed("benign compiler detail".to_string());

        let events = capture_events(|| {
            log_wasm_runtime_error(&capability_id, &execution_error);
            log_wasm_guest_error(&capability_id, &[], "benign guest response");
            log_wasm_runtime_error(&capability_id, &legacy_error);
        });

        assert_eq!(events.len(), 4);
        assert_eq!(field(&events[0], "wasm_log"), "benign guest log");
        assert_eq!(field(&events[1], "wasm_error"), "benign execution detail");
        assert_eq!(field(&events[2], "wasm_error"), "benign guest response");
        assert_eq!(
            field(&events[3], "wasm_error"),
            "failed to compile WIT component: benign compiler detail"
        );
        assert!(
            events.iter().all(|event| {
                let message = field(event, "message");
                !message.contains("raw guest error") && !message.contains("raw capability error")
            }),
            "event wording must not promise that guest-controlled text is raw: {events:?}"
        );
    }

    #[test]
    fn host_tracing_redaction_marker_is_idempotent_at_the_sink() {
        let capability_id = capability_id();
        let events = capture_events(|| {
            log_wasm_guest_error(&capability_id, &[], DIAGNOSTIC_REDACTION_MARKER);
        });

        assert_eq!(events.len(), 1);
        assert_eq!(
            field(&events[0], "wasm_error"),
            DIAGNOSTIC_REDACTION_MARKER,
            "re-scanning an already sanitized diagnostic must not rewrite or nest its marker"
        );
    }

    #[test]
    fn host_tracing_accepts_the_byte_boundary_and_wholly_redacts_oversize_inputs() {
        let capability_id = capability_id();
        let at_limit = "é".repeat(DIAGNOSTIC_MAX_BYTES / "é".len());
        let over_limit = format!("{at_limit}x");
        assert_eq!(at_limit.len(), DIAGNOSTIC_MAX_BYTES);
        assert_eq!(over_limit.len(), DIAGNOSTIC_MAX_BYTES + 1);

        let execution_error = WasmError::ExecutionFailed {
            message: over_limit.clone(),
            usage: ResourceUsage::default(),
            logs: vec![WasmLogRecord {
                level: WasmLogLevel::Info,
                message: at_limit.clone(),
            }],
        };
        let legacy_error = WasmError::CompilationFailed(over_limit.clone());
        let events = capture_events(|| {
            log_wasm_runtime_error(&capability_id, &execution_error);
            log_wasm_guest_error(&capability_id, &[], &over_limit);
            log_wasm_runtime_error(&capability_id, &legacy_error);
        });

        assert_eq!(events.len(), 4);
        assert_eq!(
            field(&events[0], "wasm_log"),
            at_limit,
            "a complete UTF-8 diagnostic at the byte limit remains available"
        );
        for event in &events[1..] {
            let diagnostic = field(event, "wasm_error");
            assert_eq!(
                diagnostic, DIAGNOSTIC_REDACTION_MARKER,
                "an oversize diagnostic must be replaced wholesale: {event:?}"
            );
            assert!(diagnostic.len() <= DIAGNOSTIC_MAX_BYTES);
            assert!(
                !diagnostic.contains(&over_limit),
                "no oversize cause fragment may survive: {event:?}"
            );
        }
    }
}
