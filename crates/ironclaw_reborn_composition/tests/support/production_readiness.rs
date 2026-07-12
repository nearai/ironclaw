use ironclaw_host_api::runtime::RuntimeKind;
use ironclaw_host_runtime::{ProductionWiringConfig, ProductionWiringReport};
use ironclaw_reborn_composition::{
    RebornCompositionProfile, RebornReadinessDiagnostic, RebornReadinessDiagnosticComponent,
    RebornReadinessDiagnosticReason, RebornReadinessDiagnosticStatus,
};
use serde_json::json;

pub(crate) fn required_backend_parity_config() -> ProductionWiringConfig {
    ProductionWiringConfig::new([
        RuntimeKind::Script,
        RuntimeKind::Mcp,
        RuntimeKind::Wasm,
        RuntimeKind::System,
    ])
    .require_runtime_http_egress()
    .require_credential_broker()
    .require_wasm_credentials()
}

pub(crate) fn assert_required_backend_readiness_diagnostics(report: &ProductionWiringReport) {
    let diagnostics = RebornReadinessDiagnostic::from_production_wiring_report(
        RebornCompositionProfile::Production,
        report,
    );

    assert_eq!(
        sorted_diagnostic_keys(&diagnostics),
        sorted_diagnostic_keys([
            production_blocker_value(
                RebornReadinessDiagnosticComponent::RuntimeBackend,
                RebornReadinessDiagnosticReason::Unsupported,
            ),
            production_blocker_value(
                RebornReadinessDiagnosticComponent::ScriptRuntime,
                RebornReadinessDiagnosticReason::Missing,
            ),
            production_blocker_value(
                RebornReadinessDiagnosticComponent::McpRuntime,
                RebornReadinessDiagnosticReason::Missing,
            ),
            production_blocker_value(
                RebornReadinessDiagnosticComponent::WasmRuntime,
                RebornReadinessDiagnosticReason::Missing,
            ),
            production_blocker_value(
                RebornReadinessDiagnosticComponent::WasmCredentialProvider,
                RebornReadinessDiagnosticReason::Missing,
            ),
        ]),
        "required backend gaps must map to the same stable readiness vocabulary: {report:?}"
    );

    let encoded = serde_json::to_string(&diagnostics).expect("diagnostics serialize");
    assert!(!encoded.contains("postgres://"));
    assert!(!encoded.contains("libsql://"));
    assert!(!encoded.contains("01234567890123456789012345678901"));
}

fn production_blocker_value(
    component: RebornReadinessDiagnosticComponent,
    reason: RebornReadinessDiagnosticReason,
) -> serde_json::Value {
    json!({
        "profile": RebornCompositionProfile::Production,
        "component": component,
        "reason": reason,
        "status": RebornReadinessDiagnosticStatus::Blocking,
        "blocks_production": true,
    })
}

fn sorted_diagnostic_keys(values: impl IntoIterator<Item = impl serde::Serialize>) -> Vec<String> {
    let mut values = values.into_iter().map(diagnostic_key).collect::<Vec<_>>();
    values.sort();
    values
}

fn diagnostic_key(value: impl serde::Serialize) -> String {
    let value = serde_json::to_value(value).expect("diagnostic serializes");
    format!(
        "profile={}|component={}|reason={}|status={}|blocks_production={}",
        value["profile"].as_str().expect("profile string"),
        value["component"].as_str().expect("component string"),
        value["reason"].as_str().expect("reason string"),
        value["status"].as_str().expect("status string"),
        value["blocks_production"]
            .as_bool()
            .expect("blocks_production bool"),
    )
}
