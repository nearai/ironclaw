use ironclaw_host_api::*;
use ironclaw_wasm::*;
use serde_json::json;

#[test]
fn json_abi_round_trips_input_and_records_usage() {
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime.prepare(json_echo_spec()).unwrap();
    let descriptor = descriptor_with_input_schema();
    let reservation = sample_reservation();

    let result = runtime
        .invoke_json(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation {
                input: json!({"message": "hello"}),
            },
        )
        .unwrap();

    assert_eq!(result.output, json!({"message": "hello"}));
    assert_eq!(result.reservation_id, reservation.id);
    assert!(result.usage.wall_clock_ms > 0);
    assert!(result.usage.process_count >= 1);
    assert!(result.fuel_consumed > 0);
    assert!(result.output_bytes > 0);
}

#[test]
fn json_abi_validates_input_schema_before_guest_execution() {
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime.prepare(json_echo_spec()).unwrap();
    let descriptor = descriptor_with_input_schema();
    let reservation = sample_reservation();

    let err = runtime
        .invoke_json(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation { input: json!({}) },
        )
        .unwrap_err();

    assert!(matches!(
        err,
        WasmError::InvalidInvocation { reason } if reason.contains("message")
    ));
}

#[test]
fn json_abi_enforces_full_json_schema_constraints_before_guest_execution() {
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime.prepare(json_echo_spec()).unwrap();
    let descriptor = make_descriptor(
        "json",
        "json.echo",
        RuntimeKind::Wasm,
        json!({
            "type": "object",
            "required": ["message"],
            "additionalProperties": false,
            "properties": {
                "message": {"type": "string", "enum": ["allowed"]}
            }
        }),
    );
    let reservation = sample_reservation();

    let err = runtime
        .invoke_json(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation {
                input: json!({"message": "blocked", "extra": true}),
            },
        )
        .unwrap_err();

    assert!(matches!(err, WasmError::InvalidInvocation { reason } if reason.contains("schema")));
}

#[test]
fn json_abi_reports_guest_error_status_with_sanitized_message() {
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime.prepare(guest_error_spec()).unwrap();
    let descriptor = make_descriptor(
        "json",
        "json.fail",
        RuntimeKind::Wasm,
        json!({"type":"object"}),
    );
    let reservation = sample_reservation();

    let err = runtime
        .invoke_json(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation {
                input: json!({"message": "hello"}),
            },
        )
        .unwrap_err();

    assert!(matches!(
        err,
        WasmError::GuestError { status: 7, message } if message == "bad input"
    ));
}

#[test]
fn json_abi_rejects_invalid_guest_json_output() {
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime.prepare(invalid_json_output_spec()).unwrap();
    let descriptor = make_descriptor(
        "json",
        "json.invalid",
        RuntimeKind::Wasm,
        json!({"type":"object"}),
    );
    let reservation = sample_reservation();

    let err = runtime
        .invoke_json(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation {
                input: json!({"message": "hello"}),
            },
        )
        .unwrap_err();

    assert!(matches!(err, WasmError::InvalidGuestOutput { .. }));
}

#[test]
fn json_abi_enforces_output_byte_limit() {
    let runtime = WasmRuntime::new(WasmRuntimeConfig {
        fuel: 100_000,
        max_output_bytes: 8,
        ..WasmRuntimeConfig::for_testing()
    })
    .unwrap();
    let module = runtime.prepare(json_echo_spec()).unwrap();
    let descriptor = descriptor_with_input_schema();
    let reservation = sample_reservation();

    let err = runtime
        .invoke_json(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation {
                input: json!({"message": "hello"}),
            },
        )
        .unwrap_err();

    assert!(matches!(
        err,
        WasmError::OutputLimitExceeded { limit: 8, actual } if actual > 8
    ));
}

#[test]
fn json_abi_requires_memory_allocator_and_output_accessors() {
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime
        .prepare(WasmModuleSpec {
            provider: ExtensionId::new("json").unwrap(),
            capability: CapabilityId::new("json.no_abi").unwrap(),
            export: "run".to_string(),
            bytes: wat::parse_str(
                r#"(module (func (export "run") (param i32 i32) (result i32) i32.const 0))"#,
            )
            .unwrap(),
        })
        .unwrap();
    let descriptor = make_descriptor(
        "json",
        "json.no_abi",
        RuntimeKind::Wasm,
        json!({"type":"object"}),
    );
    let reservation = sample_reservation();

    let err = runtime
        .invoke_json(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation { input: json!({}) },
        )
        .unwrap_err();

    assert!(matches!(err, WasmError::MissingMemory));
}

fn json_echo_spec() -> WasmModuleSpec {
    WasmModuleSpec {
        provider: ExtensionId::new("json").unwrap(),
        capability: CapabilityId::new("json.echo").unwrap(),
        export: "run".to_string(),
        bytes: wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (global $heap (mut i32) (i32.const 1024))
                (global $out_ptr (mut i32) (i32.const 0))
                (global $out_len (mut i32) (i32.const 0))
                (func (export "alloc") (param $len i32) (result i32)
                  (local $ptr i32)
                  global.get $heap
                  local.set $ptr
                  global.get $heap
                  local.get $len
                  i32.add
                  global.set $heap
                  local.get $ptr)
                (func (export "run") (param $ptr i32) (param $len i32) (result i32)
                  local.get $ptr
                  global.set $out_ptr
                  local.get $len
                  global.set $out_len
                  i32.const 0)
                (func (export "output_ptr") (result i32)
                  global.get $out_ptr)
                (func (export "output_len") (result i32)
                  global.get $out_len))"#,
        )
        .unwrap(),
    }
}

fn guest_error_spec() -> WasmModuleSpec {
    WasmModuleSpec {
        provider: ExtensionId::new("json").unwrap(),
        capability: CapabilityId::new("json.fail").unwrap(),
        export: "run".to_string(),
        bytes: wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (data (i32.const 64) "{\"code\":\"bad_input\",\"message\":\"bad input\"}")
                (global $heap (mut i32) (i32.const 1024))
                (func (export "alloc") (param $len i32) (result i32)
                  (local $ptr i32)
                  global.get $heap
                  local.set $ptr
                  global.get $heap
                  local.get $len
                  i32.add
                  global.set $heap
                  local.get $ptr)
                (func (export "run") (param i32 i32) (result i32)
                  i32.const 7)
                (func (export "output_ptr") (result i32)
                  i32.const 64)
                (func (export "output_len") (result i32)
                  i32.const 42))"#,
        )
        .unwrap(),
    }
}

fn invalid_json_output_spec() -> WasmModuleSpec {
    WasmModuleSpec {
        provider: ExtensionId::new("json").unwrap(),
        capability: CapabilityId::new("json.invalid").unwrap(),
        export: "run".to_string(),
        bytes: wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (data (i32.const 64) "not-json")
                (global $heap (mut i32) (i32.const 1024))
                (func (export "alloc") (param $len i32) (result i32)
                  (local $ptr i32)
                  global.get $heap
                  local.set $ptr
                  global.get $heap
                  local.get $len
                  i32.add
                  global.set $heap
                  local.get $ptr)
                (func (export "run") (param i32 i32) (result i32)
                  i32.const 0)
                (func (export "output_ptr") (result i32)
                  i32.const 64)
                (func (export "output_len") (result i32)
                  i32.const 8))"#,
        )
        .unwrap(),
    }
}

fn descriptor_with_input_schema() -> CapabilityDescriptor {
    make_descriptor(
        "json",
        "json.echo",
        RuntimeKind::Wasm,
        json!({
            "type": "object",
            "required": ["message"],
            "properties": {
                "message": {"type": "string"}
            }
        }),
    )
}

fn make_descriptor(
    provider: &str,
    capability: &str,
    runtime: RuntimeKind,
    parameters_schema: serde_json::Value,
) -> CapabilityDescriptor {
    CapabilityDescriptor {
        id: CapabilityId::new(capability).unwrap(),
        provider: ExtensionId::new(provider).unwrap(),
        runtime,
        trust_ceiling: TrustClass::Sandbox,
        description: "test capability".to_string(),
        parameters_schema,
        effects: vec![EffectKind::DispatchCapability],
        default_permission: PermissionMode::Allow,
        resource_profile: None,
    }
}

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn sample_reservation() -> ResourceReservation {
    ResourceReservation {
        id: ResourceReservationId::new(),
        scope: sample_scope(),
        estimate: ResourceEstimate::default(),
    }
}
