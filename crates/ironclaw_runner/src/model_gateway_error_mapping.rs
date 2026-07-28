use ironclaw_turns::run_profile::{AgentLoopHostError, LoopModelGatewayError, LoopSafeSummary};

/// Canonical conversion at the host-model gateway boundary.
///
/// Both the root-provider and thread-resolving gateways use this path so
/// fallback summaries, diagnostic metadata, and fail-closed detail scrubbing
/// cannot drift between driver families.
pub(crate) fn host_error_to_model_gateway_error(
    error: AgentLoopHostError,
) -> LoopModelGatewayError {
    let diagnostic_ref = error.diagnostic_ref;
    let reason_kind = error.reason_kind;
    let gate_ref = error.gate_ref;
    let existing_detail = error
        .detail
        .map(ironclaw_loop_host::scrub_model_visible_detail);
    let raw_summary = error.safe_summary;
    let (mut converted, rejected_summary_detail) =
        match LoopModelGatewayError::new(error.kind, raw_summary.clone()) {
            Ok(error) => (error, None),
            Err(validation_error) => {
                tracing::debug!(
                    validation_error = %validation_error,
                    "model gateway summary rejected; using fallback"
                );
                (
                    LoopModelGatewayError {
                        kind: error.kind,
                        safe_summary: LoopSafeSummary::model_gateway_failed(),
                        reason_kind: None,
                        gate_ref: None,
                        diagnostic_ref: None,
                        detail: None,
                    },
                    Some(ironclaw_loop_host::scrub_model_visible_detail(raw_summary)),
                )
            }
        };
    if let Some(detail) = existing_detail.or(rejected_summary_detail) {
        converted = converted.with_detail(detail);
    }
    if let Some(reason_kind) = reason_kind {
        converted = converted.with_reason_kind(reason_kind);
    }
    if let Some(gate_ref) = gate_ref {
        converted = converted.with_gate_ref(gate_ref);
    }
    if let Some(diagnostic_ref) = diagnostic_ref {
        converted = converted.with_diagnostic_ref(diagnostic_ref);
    }
    converted
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_turns::run_profile::AgentLoopHostErrorKind;

    #[test]
    fn existing_detail_is_rescrubbed_and_preserved() {
        let error = AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "model service is unavailable",
        )
        .with_detail(concat!(
            "provider 500 at /host/route body ghp",
            "_012345678901234567890123456789012345",
        ));

        let converted = host_error_to_model_gateway_error(error);
        let detail = converted.detail.expect("detail carried onto gateway error");

        assert!(!detail.contains(concat!("ghp", "_012345678901234567890123456789012345")));
        assert!(detail.contains("/host/route"));
        assert!(!detail.contains("EXTERNAL, UNTRUSTED source"));
    }

    #[test]
    fn rejected_summary_becomes_scrubbed_fallback_detail() {
        let raw = "provider failed at /tmp/{response} using api_key=secret-value";
        let converted = host_error_to_model_gateway_error(AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            raw,
        ));

        assert_eq!(converted.safe_summary.as_str(), "model gateway failed");
        let detail = converted
            .detail
            .expect("rejected summary preserved as detail");
        assert!(detail.contains("/tmp/{response}"));
        assert!(!detail.contains("secret-value"));
    }
}
