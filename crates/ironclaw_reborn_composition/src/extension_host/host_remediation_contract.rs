//! The structural guard for host-authored remediation text.
//!
//! ## Why this module exists
//!
//! #6299 folded every capability failure through the `ironclaw_host_api`
//! charter, which squeezed the free-text diagnostic through `SafeSummary`.
//! Every host-authored remediation string fails that contract (URLs, newlines,
//! backticks, the word `secret`), so all of them silently degraded to
//! "capability summary unavailable". Nothing went red, because the only
//! full-path coverage was ONE scenario (google readiness) and the failure mode
//! is degradation, not error.
//!
//! Two lessons are encoded here:
//!
//! 1. **Test the whole path, not one validator.** A prior spec that checked
//!    only the `ironclaw_threads` validator would have stayed green while
//!    production was broken. Each case below drives the real
//!    `FirstPartyCapabilityError` producer AND the real `ironclaw_turns`
//!    host_api hop AND the real `ironclaw_threads` persistence validator.
//! 2. **Assert the placeholder is ABSENT.** "an error was surfaced" passes
//!    while the UX is dead; only "the text is not the placeholder AND contains
//!    the expected step" catches degradation.
//!
//! Coverage is compiler-nudged, not compiler-proved. The cases come from
//! [`HostRemediationText::all`]; its exhaustiveness witness match plus the
//! `[HostRemediationText; 6]` length annotation force an author who adds a
//! VARIANT to update `ALL`. A new PRODUCER that calls
//! `dispatch_with_host_remediation` WITHOUT adding a variant is NOT caught by
//! the compiler and needs a manual audit
//! (`rg -n dispatch_with_host_remediation crates`).

use ironclaw_host_api::RuntimeDispatchErrorKind;
use ironclaw_host_api::{DispatchFailureDetail, HostRemediation, Resolution, SafeSummary};
use ironclaw_host_runtime::FirstPartyCapabilityError;
use ironclaw_reborn_config::HostRemediationText;
use ironclaw_threads::{ToolResultReferenceEnvelope, ToolResultSafeSummary};
use ironclaw_turns::run_profile::{
    CapabilityFailureDetail, CapabilityFailureKind, ModelVisibleToolObservation, ObservationTrust,
    ToolObservationDetail, ToolObservationStatus, resolution,
};

/// The exact string a degraded remediation collapses to. Asserted ABSENT
/// everywhere below — this is the failure signature of the #6299 regression.
fn placeholder() -> String {
    SafeSummary::placeholder().as_str().to_string()
}

/// Drive the REAL host_api hop (`ironclaw_turns::run_profile::resolution::failed`,
/// the same constructor production uses) and return the model-visible text that
/// reaches the verdict.
fn text_through_host_api_hop(detail: CapabilityFailureDetail) -> Option<String> {
    match resolution::failed(
        CapabilityFailureKind::Backend,
        "tool failed".to_string(),
        Some(detail),
    ) {
        Resolution::Done(done) => done
            .verdict
            .diagnostic()
            .and_then(|diagnostic| diagnostic.model_visible_text().map(str::to_string)),
        other => panic!("expected Done, got {other:?}"),
    }
}

/// Drive the REAL persistence validator by building the production-shaped
/// model observation (`GenericFailure { detail }` — the slot
/// `ironclaw_agent_loop` renders remediation into) and pushing it through
/// `ToolResultReferenceEnvelope`, which is what actually runs
/// `validate_model_observation_text`.
fn persists_to_thread_history(text: &str, trust: ObservationTrust) -> Result<(), String> {
    let observation = ModelVisibleToolObservation {
        schema_version: ironclaw_turns::run_profile::MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
        status: ToolObservationStatus::Error,
        summary: "the tool call failed".to_string(),
        detail: ToolObservationDetail::GenericFailure {
            failure_kind: CapabilityFailureKind::Backend,
            detail: Some(text.to_string()),
        },
        artifacts: Vec::new(),
        recovery: None,
        trust,
    };
    let json = serde_json::to_value(&observation).map_err(|error| error.to_string())?;
    ToolResultReferenceEnvelope::with_model_observation(
        "result:01890a5dac96774bbcceb302099a8057",
        ToolResultSafeSummary::new("the tool call failed").map_err(|error| error.to_string())?,
        json,
    )
    .map(|_| ())
}

/// LAYER 1 + 3 + 5: every enumerated host-authored remediation text survives
/// the WHOLE path — trusted-newtype construction, the real producer, the real
/// host_api hop, and the real persistence validator — and never degrades to the
/// placeholder.
///
/// Adding a `HostRemediationText` VARIANT without listing it in `ALL` fails to
/// compile (`HostRemediationText::all`'s exhaustiveness witness plus the
/// `[HostRemediationText; 6]` length annotation), so an enumerated text cannot
/// ship untested. A new PRODUCER that calls
/// `dispatch_with_host_remediation` without adding a variant is NOT compiler-
/// caught — that case needs a manual audit
/// (`rg -n dispatch_with_host_remediation crates`).
#[test]
fn host_remediation_texts_survive_the_whole_path() {
    for entry in HostRemediationText::all() {
        let text = entry.text();
        assert!(
            !text.trim().is_empty(),
            "{entry:?}: enumerated remediation text must not be empty"
        );

        // (a) constructs on the trusted channel.
        let remediation = HostRemediation::new(text.clone()).unwrap_or_else(|error| {
            panic!("{entry:?}: must construct as HostRemediation, got: {error}")
        });

        // (b) the REAL producer lands it on the trusted variant — not the
        // untrusted-diagnostic fallback, which would silently degrade later.
        let error = FirstPartyCapabilityError::dispatch_with_host_remediation(
            RuntimeDispatchErrorKind::OperationFailed,
            None,
            text.clone(),
        );
        let FirstPartyCapabilityError::Dispatch { detail, .. } = &error else {
            panic!("{entry:?}: expected a Dispatch error");
        };
        let Some(detail) = detail.as_deref() else {
            panic!("{entry:?}: expected a failure detail");
        };
        match detail {
            DispatchFailureDetail::HostRemediation { text: carried } => {
                assert_eq!(
                    carried.as_str(),
                    text,
                    "{entry:?}: producer must carry the text verbatim"
                );
            }
            other => panic!(
                "{entry:?}: producer fell back to the untrusted channel ({other:?}) — \
                 the text would degrade to the placeholder downstream"
            ),
        }

        // (c) the REAL host_api hop carries it through intact.
        let through_hop = text_through_host_api_hop(CapabilityFailureDetail::HostRemediation {
            text: remediation,
        })
        .unwrap_or_else(|| panic!("{entry:?}: host_api hop dropped the remediation entirely"));
        assert_eq!(
            through_hop, text,
            "{entry:?}: host_api hop must carry host-authored remediation verbatim"
        );
        assert_ne!(
            through_hop,
            placeholder(),
            "{entry:?}: remediation degraded to the placeholder at the host_api hop \
             — this is the #6299 regression"
        );
        assert!(
            !through_hop.contains(&placeholder()),
            "{entry:?}: remediation must not contain the placeholder"
        );

        // (d) the REAL persistence validator accepts it, so it reaches history.
        // `HostAuthored` is the trust the production renderer stamps for a
        // `HostRemediation` detail — the provenance signal that replaced the
        // deleted content heuristic.
        persists_to_thread_history(&through_hop, ObservationTrust::HostAuthored).unwrap_or_else(
            |error| panic!("{entry:?}: text would be dropped at thread persistence: {error}"),
        );
    }
}

/// LAYER 1: each enumerated text names its actual operator step, so a future
/// reword cannot quietly empty the guidance while still passing the validators.
#[test]
fn host_remediation_texts_name_their_operator_step() {
    for entry in HostRemediationText::all() {
        let text = entry.text();
        let expected = match entry {
            HostRemediationText::GoogleNotConfigured => "config set google.client_id",
            HostRemediationText::GoogleBackendAuth => "config set google.client_secret",
            HostRemediationText::SlackBothGaps => "config set slack.enabled",
            HostRemediationText::SlackEnableOnly => "config set slack.enabled",
            HostRemediationText::SlackRedirectUriOnly => {
                "IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI"
            }
            HostRemediationText::ApplyStep => "ironclaw service restart",
        };
        assert!(
            text.contains(expected),
            "{entry:?}: remediation must name `{expected}`, got: {text}"
        );
    }
}

/// LAYER 4: the trusted channel must stay NARROW. Untrusted capability output
/// (a WASM tool's stderr, an MCP error body, a provider rejection) still
/// collapses to the placeholder through the same production path.
///
/// This is what stops a future maintainer from "fixing" some other dropped-text
/// bug by routing untrusted output through `HostRemediation`.
#[test]
fn untrusted_capability_output_still_collapses_to_the_placeholder() {
    for untrusted in [
        // A host path — the classic leak the SafeSummary contract exists for.
        "failed reading /etc/passwd",
        // A token-shaped string in arbitrary tool output.
        "provider rejected token sk-ant-abc123def456",
        // A raw payload dump.
        "backend returned {\"error\": \"boom\"}",
    ] {
        let through_hop = text_through_host_api_hop(CapabilityFailureDetail::Diagnostic {
            text: untrusted.to_string(),
        })
        .expect("the untrusted diagnostic arm is still present");
        assert_eq!(
            through_hop,
            placeholder(),
            "untrusted capability output must still collapse to the placeholder: {untrusted}"
        );
    }
}

/// LAYER 4 (pair): credential VALUE shapes are rejected by the trusted
/// newtype's constructor, and the producer therefore does NOT put them on the
/// trusted variant — it degrades to the untrusted channel instead of smuggling
/// a secret through the wider guard.
#[test]
fn credential_value_shapes_are_refused_by_the_trusted_channel() {
    for leaked in [
        "config set google.client_secret sk-ant-abc123def456",
        "the value is GOCSPX-abc123def456ghi",
        "use ghp_0123456789abcdefghij",
        "bot token xoxb-1234-5678-abcdefghijklmnop",
        "user token xoxp-1234-5678-abcdefghijklmnop",
    ] {
        assert!(
            HostRemediation::new(leaked).is_err(),
            "the trusted channel must refuse a credential VALUE: {leaked}"
        );

        // And the producer degrades rather than carrying it on the trusted arm.
        let error = FirstPartyCapabilityError::dispatch_with_host_remediation(
            RuntimeDispatchErrorKind::OperationFailed,
            None,
            leaked.to_string(),
        );
        let FirstPartyCapabilityError::Dispatch { detail, .. } = &error else {
            panic!("expected a Dispatch error");
        };
        assert!(
            matches!(
                detail.as_deref(),
                Some(DispatchFailureDetail::Diagnostic { .. })
            ),
            "a credential-shaped value must fall back to the untrusted channel: {leaked}"
        );
    }
}

/// PROVENANCE — not content shape — is what governs persistence.
///
/// The same bytes are accepted when the observation declares
/// `ObservationTrust::HostAuthored` and rejected when it declares
/// `UntrustedToolOutput`. Nothing about the STRING differs between the two
/// calls, so this can only pass if the validator is reading the trust signal.
///
/// This is the test that makes the deleted `is_config_set_key_reference`
/// heuristic unnecessary: it covers both the dotted-key shape that heuristic
/// tried to parse AND bare prose vocabulary it never could have.
#[test]
fn persistence_is_governed_by_provenance_not_content_shape() {
    for text in [
        // Bare prose vocabulary — no `config set`, no dotted key. The old
        // heuristic could never have exempted this.
        "the client secret was rejected by the provider",
        // The dotted-key shape the heuristic used to special-case.
        "run `ironclaw config set google.client_secret` to update it",
        // And a real production string end to end.
        &ironclaw_reborn_config::HostRemediationText::GoogleBackendAuth.text(),
    ] {
        persists_to_thread_history(text, ObservationTrust::HostAuthored).unwrap_or_else(|error| {
            panic!("host-authored provenance must persist {text:?} intact: {error}")
        });
        assert!(
            persists_to_thread_history(text, ObservationTrust::UntrustedToolOutput).is_err(),
            "the SAME bytes arriving as untrusted capability output must still be \
             rejected — only provenance may differ: {text:?}"
        );
    }
}
