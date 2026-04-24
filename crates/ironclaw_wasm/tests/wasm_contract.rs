use ironclaw_host_api::*;
use ironclaw_resources::*;
use ironclaw_wasm::*;
use rust_decimal_macros::dec;

#[test]
fn valid_module_prepares_and_invalid_module_fails() {
    let runtime = WasmRuntime::for_testing().unwrap();
    let prepared = runtime.prepare(echo_spec()).unwrap();

    assert_eq!(prepared.provider(), &ExtensionId::new("echo").unwrap());
    assert_eq!(
        prepared.capability(),
        &CapabilityId::new("echo.add_one").unwrap()
    );
    assert_eq!(prepared.export(), "add_one");

    let err = runtime
        .prepare(WasmModuleSpec {
            provider: ExtensionId::new("echo").unwrap(),
            capability: CapabilityId::new("echo.bad").unwrap(),
            export: "run".to_string(),
            bytes: b"not wasm".to_vec(),
        })
        .unwrap_err();
    assert!(matches!(err, WasmError::InvalidModule { .. }));
}

#[test]
fn invocation_requires_wasm_descriptor_matching_module_and_reservation() {
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime.prepare(echo_spec()).unwrap();
    let reservation = sample_active_reservation();

    let mut descriptor = make_descriptor("echo", "echo.add_one", RuntimeKind::Script);
    let err = runtime
        .invoke_i32(&module, &descriptor, Some(&reservation), 41)
        .unwrap_err();
    assert!(matches!(
        err,
        WasmError::DescriptorMismatch { reason } if reason.contains("RuntimeKind::Wasm")
    ));

    descriptor = make_descriptor("other", "echo.add_one", RuntimeKind::Wasm);
    let err = runtime
        .invoke_i32(&module, &descriptor, Some(&reservation), 41)
        .unwrap_err();
    assert!(matches!(
        err,
        WasmError::DescriptorMismatch { reason } if reason.contains("provider")
    ));

    descriptor = make_descriptor("echo", "echo.other", RuntimeKind::Wasm);
    let err = runtime
        .invoke_i32(&module, &descriptor, Some(&reservation), 41)
        .unwrap_err();
    assert!(matches!(
        err,
        WasmError::DescriptorMismatch { reason } if reason.contains("capability")
    ));

    descriptor = make_descriptor("echo", "echo.add_one", RuntimeKind::Wasm);
    let err = runtime
        .invoke_i32(&module, &descriptor, None, 41)
        .unwrap_err();
    assert!(matches!(err, WasmError::MissingReservation));
}

#[test]
fn exported_function_is_invoked_and_usage_can_be_reconciled() {
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime.prepare(echo_spec()).unwrap();
    let descriptor = make_descriptor("echo", "echo.add_one", RuntimeKind::Wasm);

    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_usd: Some(dec!(1.00)),
            max_concurrency_slots: Some(1),
            ..ResourceLimits::default()
        },
    );
    let reservation = governor
        .reserve(
            scope,
            ResourceEstimate {
                usd: Some(dec!(0.01)),
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
        )
        .unwrap();
    let active = governor.active_reservation(reservation.id).unwrap();

    let result = runtime
        .invoke_i32(&module, &descriptor, Some(&active), 41)
        .unwrap();

    assert_eq!(result.value, 42);
    assert_eq!(result.reservation_id, reservation.id);
    assert!(result.usage.wall_clock_ms > 0);
    assert!(result.usage.process_count >= 1);
    assert!(result.fuel_consumed > 0);
    assert!(result.output_bytes > 0);

    governor.reconcile(reservation.id, result.usage).unwrap();
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert!(governor.usage_for(&account).wall_clock_ms > 0);
}

#[test]
fn output_byte_limit_is_enforced() {
    let runtime = WasmRuntime::new(WasmRuntimeConfig {
        fuel: 10_000,
        max_output_bytes: 1,
        ..WasmRuntimeConfig::for_testing()
    })
    .unwrap();
    let module = runtime.prepare(echo_spec()).unwrap();
    let descriptor = make_descriptor("echo", "echo.add_one", RuntimeKind::Wasm);
    let reservation = sample_active_reservation();

    let err = runtime
        .invoke_i32(&module, &descriptor, Some(&reservation), 41)
        .unwrap_err();

    assert!(matches!(
        err,
        WasmError::OutputLimitExceeded { limit: 1, actual } if actual > 1
    ));
}

#[test]
fn fuel_limit_stops_runaway_module() {
    let runtime = WasmRuntime::new(WasmRuntimeConfig {
        fuel: 1_000,
        max_output_bytes: 1_024,
        ..WasmRuntimeConfig::for_testing()
    })
    .unwrap();
    let module = runtime
        .prepare(WasmModuleSpec {
            provider: ExtensionId::new("loop").unwrap(),
            capability: CapabilityId::new("loop.spin").unwrap(),
            export: "spin".to_string(),
            bytes: wat::parse_str(
                r#"(module
                    (func (export "spin") (param i32) (result i32)
                      (loop br 0)
                      i32.const 0))"#,
            )
            .unwrap(),
        })
        .unwrap();
    let descriptor = make_descriptor("loop", "loop.spin", RuntimeKind::Wasm);
    let reservation = sample_active_reservation();

    let err = runtime
        .invoke_i32(&module, &descriptor, Some(&reservation), 0)
        .unwrap_err();

    assert!(matches!(err, WasmError::FuelExhausted { .. }));
}

#[test]
fn memory_limit_is_enforced() {
    let runtime = WasmRuntime::new(WasmRuntimeConfig {
        fuel: 10_000,
        max_output_bytes: 1_024,
        max_memory_bytes: 64 * 1024,
    })
    .unwrap();
    let module = runtime
        .prepare(WasmModuleSpec {
            provider: ExtensionId::new("memory-hog").unwrap(),
            capability: CapabilityId::new("memory-hog.allocate").unwrap(),
            export: "allocate".to_string(),
            bytes: wat::parse_str(
                r#"(module
                    (memory 2)
                    (func (export "allocate") (param i32) (result i32)
                      local.get 0))"#,
            )
            .unwrap(),
        })
        .unwrap();
    let descriptor = make_descriptor("memory-hog", "memory-hog.allocate", RuntimeKind::Wasm);
    let reservation = sample_active_reservation();

    let err = runtime
        .invoke_i32(&module, &descriptor, Some(&reservation), 0)
        .unwrap_err();

    assert!(matches!(err, WasmError::MemoryExceeded { .. }));
}

#[test]
fn closed_reservation_cannot_be_claimed_for_invocation() {
    let governor = InMemoryResourceGovernor::new();
    let reservation = governor
        .reserve(sample_scope(), ResourceEstimate::default())
        .unwrap();
    governor.release(reservation.id).unwrap();

    assert!(matches!(
        governor.active_reservation(reservation.id),
        Err(ResourceError::ReservationClosed { .. })
    ));
}

#[test]
fn missing_host_import_fails_validation() {
    let runtime = WasmRuntime::for_testing().unwrap();
    let err = runtime
        .prepare(WasmModuleSpec {
            provider: ExtensionId::new("needs-host").unwrap(),
            capability: CapabilityId::new("needs-host.run").unwrap(),
            export: "run".to_string(),
            bytes: wat::parse_str(
                r#"(module
                    (import "host" "read_file" (func $read_file (param i32) (result i32)))
                    (func (export "run") (param i32) (result i32)
                      local.get 0
                      call $read_file))"#,
            )
            .unwrap(),
        })
        .unwrap_err();

    assert!(matches!(err, WasmError::UnsupportedImport { .. }));
}

fn echo_spec() -> WasmModuleSpec {
    WasmModuleSpec {
        provider: ExtensionId::new("echo").unwrap(),
        capability: CapabilityId::new("echo.add_one").unwrap(),
        export: "add_one".to_string(),
        bytes: wat::parse_str(
            r#"(module
                (func (export "add_one") (param i32) (result i32)
                  local.get 0
                  i32.const 1
                  i32.add))"#,
        )
        .unwrap(),
    }
}

fn make_descriptor(provider: &str, capability: &str, runtime: RuntimeKind) -> CapabilityDescriptor {
    CapabilityDescriptor {
        id: CapabilityId::new(capability).unwrap(),
        provider: ExtensionId::new(provider).unwrap(),
        runtime,
        trust_ceiling: TrustClass::Sandbox,
        description: "test capability".to_string(),
        parameters_schema: serde_json::json!({"type":"object"}),
        effects: vec![EffectKind::DispatchCapability],
        default_permission: PermissionMode::Allow,
        resource_profile: None,
    }
}

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn sample_active_reservation() -> ActiveResourceReservation {
    let governor = InMemoryResourceGovernor::new();
    let reservation = governor
        .reserve(sample_scope(), ResourceEstimate::default())
        .unwrap();
    governor.active_reservation(reservation.id).unwrap()
}
