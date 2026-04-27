use std::sync::Arc;
use std::time::Duration;

use ironclaw_host_api::*;
use ironclaw_wasm::*;

#[test]
fn memory_growth_beyond_configured_limit_fails_closed() {
    let runtime = WasmRuntime::new(WasmRuntimeConfig {
        fuel: 100_000,
        max_output_bytes: 1_024,
        max_memory_bytes: 64 * 1024,
        timeout: Duration::from_secs(5),
        cache_compiled_modules: false,
        cache_dir: None,
        epoch_tick_interval: Duration::from_millis(10),
    })
    .unwrap();
    let module = runtime.prepare(memory_grow_spec()).unwrap();
    let descriptor = make_descriptor("engine", "engine.grow", RuntimeKind::Wasm);
    let reservation = sample_reservation();

    let err = runtime
        .invoke_i32(&module, &descriptor, Some(&reservation), 1)
        .unwrap_err();

    assert!(matches!(
        err,
        WasmError::MemoryExceeded { limit, .. } if limit == 64 * 1024
    ));
}

#[test]
fn wall_clock_timeout_interrupts_runaway_module_even_with_large_fuel() {
    let runtime = WasmRuntime::new(WasmRuntimeConfig {
        fuel: u64::MAX / 4,
        max_output_bytes: 1_024,
        max_memory_bytes: 1024 * 1024,
        timeout: Duration::from_millis(5),
        cache_compiled_modules: false,
        cache_dir: None,
        epoch_tick_interval: Duration::from_millis(5),
    })
    .unwrap();
    let module = runtime.prepare(spin_spec()).unwrap();
    let descriptor = make_descriptor("engine", "engine.spin", RuntimeKind::Wasm);
    let reservation = sample_reservation();

    let err = runtime
        .invoke_i32(&module, &descriptor, Some(&reservation), 0)
        .unwrap_err();

    assert!(matches!(err, WasmError::Timeout { .. }));
}

#[test]
fn prepared_module_cache_reuses_same_bytes_and_splits_changed_content() {
    let runtime = WasmRuntime::new(WasmRuntimeConfig {
        cache_compiled_modules: true,
        ..WasmRuntimeConfig::for_testing()
    })
    .unwrap();

    let first = runtime.prepare_cached(echo_spec(1)).unwrap();
    let second = runtime.prepare_cached(echo_spec(1)).unwrap();

    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(runtime.prepared_module_count(), 1);

    let changed = runtime.prepare_cached(echo_spec(2)).unwrap();

    assert!(!Arc::ptr_eq(&first, &changed));
    assert_eq!(runtime.prepared_module_count(), 2);
}

#[test]
fn prepared_module_cache_can_be_disabled() {
    let runtime = WasmRuntime::new(WasmRuntimeConfig {
        cache_compiled_modules: false,
        ..WasmRuntimeConfig::for_testing()
    })
    .unwrap();

    let first = runtime.prepare_cached(echo_spec(1)).unwrap();
    let second = runtime.prepare_cached(echo_spec(1)).unwrap();

    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(runtime.prepared_module_count(), 0);
}

#[test]
fn cached_modules_still_instantiate_fresh_per_invocation() {
    let runtime = WasmRuntime::new(WasmRuntimeConfig {
        cache_compiled_modules: true,
        ..WasmRuntimeConfig::for_testing()
    })
    .unwrap();
    let module = runtime.prepare_cached(counter_spec()).unwrap();
    let descriptor = make_descriptor("engine", "engine.counter", RuntimeKind::Wasm);
    let reservation = sample_reservation();

    let first = runtime
        .invoke_i32(module.as_ref(), &descriptor, Some(&reservation), 0)
        .unwrap();
    let second = runtime
        .invoke_i32(module.as_ref(), &descriptor, Some(&reservation), 0)
        .unwrap();

    assert_eq!(first.value, 1);
    assert_eq!(second.value, 1);
}

fn memory_grow_spec() -> WasmModuleSpec {
    WasmModuleSpec {
        provider: ExtensionId::new("engine").unwrap(),
        capability: CapabilityId::new("engine.grow").unwrap(),
        export: "grow".to_string(),
        bytes: wat::parse_str(
            r#"(module
                (memory 1)
                (func (export "grow") (param i32) (result i32)
                  local.get 0
                  memory.grow
                  drop
                  i32.const 1))"#,
        )
        .unwrap(),
    }
}

fn spin_spec() -> WasmModuleSpec {
    WasmModuleSpec {
        provider: ExtensionId::new("engine").unwrap(),
        capability: CapabilityId::new("engine.spin").unwrap(),
        export: "spin".to_string(),
        bytes: wat::parse_str(
            r#"(module
                (func (export "spin") (param i32) (result i32)
                  (loop br 0)
                  i32.const 0))"#,
        )
        .unwrap(),
    }
}

fn echo_spec(addend: i32) -> WasmModuleSpec {
    WasmModuleSpec {
        provider: ExtensionId::new("engine").unwrap(),
        capability: CapabilityId::new("engine.echo").unwrap(),
        export: "echo".to_string(),
        bytes: wat::parse_str(format!(
            r#"(module
                (func (export "echo") (param i32) (result i32)
                  local.get 0
                  i32.const {addend}
                  i32.add))"#,
        ))
        .unwrap(),
    }
}

fn counter_spec() -> WasmModuleSpec {
    WasmModuleSpec {
        provider: ExtensionId::new("engine").unwrap(),
        capability: CapabilityId::new("engine.counter").unwrap(),
        export: "counter".to_string(),
        bytes: wat::parse_str(
            r#"(module
                (global $counter (mut i32) (i32.const 0))
                (func (export "counter") (param i32) (result i32)
                  global.get $counter
                  i32.const 1
                  i32.add
                  global.set $counter
                  global.get $counter))"#,
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
