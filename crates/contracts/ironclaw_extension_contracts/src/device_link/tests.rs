use super::*;

fn payload() -> DeviceLinkPayload {
    DeviceLinkPayload::new("scheme://login?token=AAAA-BBBB").expect("valid payload")
}

#[test]
fn mode_input_kind_display_kind_and_error_code_round_trip_their_wire_values() {
    for (mode, expected) in [
        (DeviceLinkMode::Default, "\"default\""),
        (DeviceLinkMode::Alternate, "\"alternate\""),
    ] {
        let encoded = serde_json::to_string(&mode).expect("serialize mode");
        assert_eq!(encoded, expected);
        assert_eq!(
            serde_json::from_str::<DeviceLinkMode>(&encoded).expect("deserialize mode"),
            mode
        );
    }

    for (kind, expected) in [
        (DeviceLinkInputKind::Identifier, "\"identifier\""),
        (DeviceLinkInputKind::Code, "\"code\""),
        (DeviceLinkInputKind::Password, "\"password\""),
    ] {
        let encoded = serde_json::to_string(&kind).expect("serialize input kind");
        assert_eq!(encoded, expected);
        assert_eq!(
            serde_json::from_str::<DeviceLinkInputKind>(&encoded).expect("deserialize"),
            kind
        );
    }

    for (kind, expected) in [
        (DeviceLinkDisplayKind::QrCode, "\"qr_code\""),
        (DeviceLinkDisplayKind::Link, "\"link\""),
    ] {
        let encoded = serde_json::to_string(&kind).expect("serialize display kind");
        assert_eq!(encoded, expected);
        assert_eq!(
            serde_json::from_str::<DeviceLinkDisplayKind>(&encoded).expect("deserialize"),
            kind
        );
    }

    for (code, expected) in [
        (DeviceLinkErrorCode::Expired, "\"expired\""),
        (DeviceLinkErrorCode::UnknownFlow, "\"unknown_flow\""),
        (DeviceLinkErrorCode::Declined, "\"declined\""),
        (DeviceLinkErrorCode::InvalidInput, "\"invalid_input\""),
        (DeviceLinkErrorCode::RateLimited, "\"rate_limited\""),
        (
            DeviceLinkErrorCode::AccountUnavailable,
            "\"account_unavailable\"",
        ),
        (
            DeviceLinkErrorCode::IdentityConflict,
            "\"identity_conflict\"",
        ),
        (
            DeviceLinkErrorCode::VendorUnavailable,
            "\"vendor_unavailable\"",
        ),
        (DeviceLinkErrorCode::CustodyFailed, "\"custody_failed\""),
        (DeviceLinkErrorCode::Internal, "\"internal\""),
    ] {
        let encoded = serde_json::to_string(&code).expect("serialize error code");
        assert_eq!(encoded, expected);
        assert_eq!(
            serde_json::from_str::<DeviceLinkErrorCode>(&encoded).expect("deserialize"),
            code
        );
    }

    for (kind, expected) in [
        (DeviceLinkStepKind::Display, "\"display\""),
        (DeviceLinkStepKind::AwaitingVendor, "\"awaiting_vendor\""),
        (DeviceLinkStepKind::InputRequired, "\"input_required\""),
        (DeviceLinkStepKind::Completed, "\"completed\""),
        (DeviceLinkStepKind::Failed, "\"failed\""),
    ] {
        let encoded = serde_json::to_string(&kind).expect("serialize step kind");
        assert_eq!(encoded, expected);
        assert_eq!(
            serde_json::from_str::<DeviceLinkStepKind>(&encoded).expect("deserialize"),
            kind
        );
    }
}

/// Every step variant survives the wire, and durations ride as whole
/// milliseconds rather than serde's `{secs, nanos}` struct.
#[test]
fn every_step_variant_round_trips_and_durations_are_milliseconds() {
    let steps = [
        DeviceLinkStep::Display {
            kind: DeviceLinkDisplayKind::QrCode,
            payload: payload(),
            expires_in: Duration::from_secs(30),
        },
        DeviceLinkStep::AwaitingVendor {
            retry_in: Duration::from_millis(2_500),
        },
        DeviceLinkStep::InputRequired {
            kind: DeviceLinkInputKind::Password,
            label: "Account password".to_string(),
            hint: Some("Set on the vendor's own settings screen.".to_string()),
        },
        DeviceLinkStep::InputRequired {
            kind: DeviceLinkInputKind::Identifier,
            label: "Account identifier".to_string(),
            hint: None,
        },
        DeviceLinkStep::Completed {
            account_label: "Personal account".to_string(),
            vendor_user_ref: "vendor-user-4711".to_string(),
        },
        DeviceLinkStep::Failed {
            code: DeviceLinkErrorCode::RateLimited,
            restartable: true,
        },
    ];

    for step in steps {
        let encoded = serde_json::to_value(&step).expect("serialize step");
        let decoded: DeviceLinkStep =
            serde_json::from_value(encoded.clone()).expect("deserialize step");
        assert_eq!(decoded, step, "{encoded}");
    }

    let display = serde_json::to_value(DeviceLinkStep::Display {
        kind: DeviceLinkDisplayKind::QrCode,
        payload: payload(),
        expires_in: Duration::from_secs(30),
    })
    .expect("serialize");
    assert_eq!(display["step"], "display");
    assert_eq!(display["expires_in"], 30_000);
    assert_eq!(display["payload"], "scheme://login?token=AAAA-BBBB");

    let awaiting = serde_json::to_value(DeviceLinkStep::AwaitingVendor {
        retry_in: Duration::from_millis(2_500),
    })
    .expect("serialize");
    assert_eq!(awaiting["retry_in"], 2_500);

    // The absent hint is omitted, not encoded as null.
    let bare = serde_json::to_value(DeviceLinkStep::InputRequired {
        kind: DeviceLinkInputKind::Code,
        label: "Login code".to_string(),
        hint: None,
    })
    .expect("serialize");
    assert!(bare.get("hint").is_none(), "{bare}");
}

#[test]
fn step_kind_and_terminality_agree_with_the_variants() {
    assert_eq!(
        DeviceLinkStep::AwaitingVendor {
            retry_in: Duration::from_secs(3)
        }
        .kind(),
        DeviceLinkStepKind::AwaitingVendor
    );
    assert!(
        !DeviceLinkStep::AwaitingVendor {
            retry_in: Duration::from_secs(3)
        }
        .is_terminal()
    );
    assert!(
        DeviceLinkStep::Completed {
            account_label: "Personal account".to_string(),
            vendor_user_ref: "vendor-user-4711".to_string(),
        }
        .is_terminal()
    );
    assert!(
        DeviceLinkStep::Failed {
            code: DeviceLinkErrorCode::Expired,
            restartable: true,
        }
        .is_terminal()
    );
}

#[test]
fn payload_is_bounded_validated_and_redacted() {
    assert!(DeviceLinkPayload::new("").is_err());
    assert!(DeviceLinkPayload::new("has space").is_err());
    assert!(DeviceLinkPayload::new("has\nnewline").is_err());
    assert!(DeviceLinkPayload::new("x".repeat(MAX_DEVICE_LINK_PAYLOAD_BYTES + 1)).is_err());

    let payload = payload();
    assert_eq!(payload.len(), "scheme://login?token=AAAA-BBBB".len());
    assert!(!payload.is_empty());
    assert_eq!(payload.expose(), "scheme://login?token=AAAA-BBBB");

    let rendered = format!("{payload:?}");
    assert!(!rendered.contains("AAAA-BBBB"), "{rendered}");
    assert!(rendered.contains("redacted"), "{rendered}");

    // The rejection message must not echo the payload either.
    let rejected = DeviceLinkPayload::new("bad payload").expect_err("whitespace is rejected");
    assert!(!rejected.to_string().contains("bad payload"), "{rejected}");

    // The wire form is the bare string, and it revalidates on the way in.
    let encoded = serde_json::to_string(&payload).expect("serialize");
    assert_eq!(encoded, "\"scheme://login?token=AAAA-BBBB\"");
    assert_eq!(
        serde_json::from_str::<DeviceLinkPayload>(&encoded).expect("deserialize"),
        payload
    );
    assert!(serde_json::from_str::<DeviceLinkPayload>("\"has space\"").is_err());
}

/// The two secret variants must be unable to leak through `Debug`, and the
/// identifier does not print either.
#[test]
fn device_link_input_debug_redacts_every_variant() {
    let inputs = [
        DeviceLinkInput::Identifier("+15550000000".to_string()),
        DeviceLinkInput::Code(SecretString::from("123456")),
        DeviceLinkInput::Password(SecretString::from("correct horse battery staple")),
    ];
    let expected_kinds = [
        DeviceLinkInputKind::Identifier,
        DeviceLinkInputKind::Code,
        DeviceLinkInputKind::Password,
    ];

    for (input, kind) in inputs.iter().zip(expected_kinds) {
        assert_eq!(input.kind(), kind);
        let rendered = format!("{input:?}");
        assert!(rendered.contains("redacted"), "{rendered}");
        assert!(!rendered.contains("15550000000"), "{rendered}");
        assert!(!rendered.contains("123456"), "{rendered}");
        assert!(!rendered.contains("correct horse"), "{rendered}");
    }
}

#[test]
fn device_link_input_validation_bounds_every_variant() {
    DeviceLinkInput::Identifier("+15550000000".to_string())
        .validate()
        .expect("a plausible identifier is accepted");
    DeviceLinkInput::Code(SecretString::from("123456"))
        .validate()
        .expect("a plausible code is accepted");

    for (input, kind) in [
        (
            DeviceLinkInput::Identifier(String::new()),
            DeviceLinkInputKind::Identifier,
        ),
        (
            DeviceLinkInput::Identifier("x".repeat(MAX_DEVICE_LINK_IDENTIFIER_BYTES + 1)),
            DeviceLinkInputKind::Identifier,
        ),
        (
            DeviceLinkInput::Identifier("+1555\u{0}0000".to_string()),
            DeviceLinkInputKind::Identifier,
        ),
        (
            DeviceLinkInput::Code(SecretString::from("")),
            DeviceLinkInputKind::Code,
        ),
        (
            DeviceLinkInput::Password(SecretString::from(
                "x".repeat(MAX_DEVICE_LINK_SECRET_BYTES + 1),
            )),
            DeviceLinkInputKind::Password,
        ),
    ] {
        let error = input.validate().expect_err("must be rejected");
        assert!(
            matches!(&error, DeviceLinkError::InvalidInput { kind: seen, .. } if *seen == kind),
            "{error:?}"
        );
        // A rejection never quotes what was submitted.
        assert!(!error.to_string().contains("xxx"), "{error}");
    }
}

#[test]
fn step_validation_bounds_the_display_strings_an_adapter_returns() {
    DeviceLinkStep::InputRequired {
        kind: DeviceLinkInputKind::Code,
        label: "Login code".to_string(),
        hint: Some("Sent to your account.".to_string()),
    }
    .validate()
    .expect("bounded text is accepted");

    let oversize = "x".repeat(MAX_DEVICE_LINK_LABEL_BYTES + 1);
    for step in [
        DeviceLinkStep::InputRequired {
            kind: DeviceLinkInputKind::Code,
            label: oversize.clone(),
            hint: None,
        },
        DeviceLinkStep::InputRequired {
            kind: DeviceLinkInputKind::Code,
            label: "Login code".to_string(),
            hint: Some(oversize.clone()),
        },
        DeviceLinkStep::InputRequired {
            kind: DeviceLinkInputKind::Code,
            label: String::new(),
            hint: None,
        },
        DeviceLinkStep::Completed {
            account_label: "Personal account".to_string(),
            vendor_user_ref: format!("vendor{}user", '\u{0}'),
        },
        DeviceLinkStep::Completed {
            account_label: oversize,
            vendor_user_ref: "vendor-user-4711".to_string(),
        },
    ] {
        assert!(
            matches!(step.validate(), Err(DeviceLinkError::InvalidStep { .. })),
            "{step:?} must be rejected"
        );
    }
}

/// Codes and restartability are derived from the error, so a card and the
/// audit trail never disagree about whether to offer "try again".
#[test]
fn errors_project_stable_codes_and_restartability() {
    let cases = [
        (
            DeviceLinkError::UnknownFlow,
            DeviceLinkErrorCode::UnknownFlow,
            true,
        ),
        (
            DeviceLinkError::InvalidInput {
                kind: DeviceLinkInputKind::Code,
                reason: "submitted value must not be empty",
            },
            DeviceLinkErrorCode::InvalidInput,
            true,
        ),
        (
            DeviceLinkError::InvalidStep {
                reason: "step display text exceeds its length bound",
            },
            DeviceLinkErrorCode::InvalidInput,
            false,
        ),
        (
            DeviceLinkError::UnsupportedMode {
                mode: DeviceLinkMode::Alternate,
            },
            DeviceLinkErrorCode::Internal,
            false,
        ),
        (
            DeviceLinkError::Vendor {
                code: DeviceLinkErrorCode::AccountUnavailable,
                restartable: false,
            },
            DeviceLinkErrorCode::AccountUnavailable,
            false,
        ),
        (
            DeviceLinkError::Vendor {
                code: DeviceLinkErrorCode::RateLimited,
                restartable: true,
            },
            DeviceLinkErrorCode::RateLimited,
            true,
        ),
        (
            DeviceLinkError::Custody(LinkedSessionError::Revoked),
            DeviceLinkErrorCode::CustodyFailed,
            false,
        ),
        (
            DeviceLinkError::Internal {
                reason: "adapter is not bound",
            },
            DeviceLinkErrorCode::Internal,
            false,
        ),
    ];

    for (error, code, restartable) in cases {
        assert_eq!(error.code(), code, "{error:?}");
        assert_eq!(error.restartable(), restartable, "{error:?}");
        // Fixed host-authored text only — nothing interpolated from a
        // submitted value or a vendor body.
        assert!(!error.to_string().is_empty());
    }
}

/// A custody failure converts, so an adapter can use `?` without inventing
/// its own mapping — and lands on the custody code, not `Internal`.
#[test]
fn custody_failures_convert_into_device_link_errors() {
    let error: DeviceLinkError = LinkedSessionError::BlobTooLarge {
        bytes: 1,
        max: MAX_DEVICE_LINK_PAYLOAD_BYTES,
    }
    .into();
    assert_eq!(error.code(), DeviceLinkErrorCode::CustodyFailed);
    assert!(!error.restartable());
}

#[test]
fn flow_ids_are_bounded_opaque_strings() {
    let flow = DeviceLinkFlowId::new("flow-a1b2").expect("valid flow id");
    assert_eq!(flow.as_str(), "flow-a1b2");
    assert_eq!(flow.to_string(), "flow-a1b2");

    assert!(DeviceLinkFlowId::new("").is_err());
    assert!(DeviceLinkFlowId::new("with space").is_err());
    assert!(DeviceLinkFlowId::new("x".repeat(129)).is_err());
}

/// The context's `Debug` must not print custody or configuration values —
/// it is the type a driver logs when a flow misbehaves.
#[test]
fn context_debug_reports_identity_only() {
    struct StubPort;

    #[async_trait]
    impl LinkedSessionPort for StubPort {
        async fn load(
            &self,
        ) -> Result<
            Option<crate::linked_session::LinkedSessionSnapshot>,
            crate::linked_session::LinkedSessionError,
        > {
            Ok(None)
        }

        async fn save(
            &self,
            _expected: crate::linked_session::LinkedSessionVersion,
            _blob: crate::linked_session::SessionBytes,
        ) -> Result<
            crate::linked_session::LinkedSessionVersion,
            crate::linked_session::LinkedSessionError,
        > {
            Err(crate::linked_session::LinkedSessionError::Unavailable {
                reason: "stub port does not store",
            })
        }
    }

    let flow_id = DeviceLinkFlowId::new("flow-a1b2").expect("flow id");
    let extension_id = ExtensionId::new("example").expect("extension id");
    let user_id = UserId::new("user1").expect("user id");
    let mut config = BTreeMap::new();
    config.insert("api_id".to_string(), "sensitive-config-value".to_string());
    let port = StubPort;
    let ctx = DeviceLinkContext {
        flow_id: &flow_id,
        extension_id: &extension_id,
        user_id: &user_id,
        config: &config,
        session: &port,
        account: None,
    };

    let rendered = format!("{ctx:?}");
    assert!(rendered.contains("flow-a1b2"), "{rendered}");
    assert!(rendered.contains("example"), "{rendered}");
    assert!(!rendered.contains("sensitive-config-value"), "{rendered}");
}
