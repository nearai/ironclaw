use ironclaw_product_adapters::mark_bearer_token_verified;
use ironclaw_wasm_product_adapters::{
    ProductAdapterComponentRuntime, ProductAdapterComponentRuntimeConfig, RuntimeError,
};
use wit_component::{ComponentEncoder, StringEncoding, embed_component_metadata};
use wit_parser::Resolve;

const FIXTURE_ADAPTER_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 8192))
  (data (i32.const 1024) "fixture_adapter")
  (data (i32.const 1056) "install_1")
  (data (i32.const 1088) "{\22flags\22:[]}")
  (data (i32.const 2048) "{\22external_event_id\22:\22evt1\22,\22external_actor_ref\22:{\22kind\22:\22user\22,\22id\22:\22u1\22,\22display_name\22:null},\22external_conversation_ref\22:{\22space_id\22:null,\22conversation_id\22:\22c1\22,\22topic_id\22:null,\22reply_target_message_id\22:null},\22payload\22:\22no_op\22}")
  (data (i32.const 3072) "{\22egress_target_index\22:0,\22method\22:\22POST\22,\22path\22:\22/send\22,\22headers\22:[],\22body\22:[]}")
  (data (i32.const 4096) "api.example.com")

  (func $manifest (result i32)
    ;; adapter-id
    i32.const 16
    i32.const 1024
    i32.store
    i32.const 20
    i32.const 15
    i32.store
    ;; installation-id
    i32.const 24
    i32.const 1056
    i32.store
    i32.const 28
    i32.const 9
    i32.store
    ;; capabilities-json
    i32.const 32
    i32.const 1088
    i32.store
    i32.const 36
    i32.const 12
    i32.store
    ;; declared-egress-targets: [ { host: "api.example.com", credential-handle: none } ]
    i32.const 5120
    i32.const 4096
    i32.store
    i32.const 5124
    i32.const 15
    i32.store
    i32.const 5128
    i32.const 0
    i32.store
    i32.const 5132
    i32.const 0
    i32.store
    i32.const 5136
    i32.const 0
    i32.store
    i32.const 40
    i32.const 5120
    i32.store
    i32.const 44
    i32.const 1
    i32.store
    ;; declared-auth-requirements: empty list
    i32.const 48
    i32.const 0
    i32.store
    i32.const 52
    i32.const 0
    i32.store
    i32.const 16)

  (func $parse_inbound (param $raw_ptr i32) (param $raw_len i32) (param $evidence_ptr i32) (param $evidence_len i32) (result i32)
    ;; ok(parsed-inbound { parsed-json })
    i32.const 128
    i32.const 0
    i32.store
    i32.const 132
    i32.const 2048
    i32.store
    i32.const 136
    i32.const 229
    i32.store
    i32.const 128)

  (func $render_outbound (param $outbound_ptr i32) (param $outbound_len i32) (result i32)
    ;; ok(outbound-render { egress-request-json })
    i32.const 144
    i32.const 0
    i32.store
    i32.const 148
    i32.const 3072
    i32.store
    i32.const 152
    i32.const 79
    i32.store
    i32.const 144)

  (func $post (param i32))
  (func $realloc (param $old i32) (param $old_align i32) (param $new_size i32) (param $new_align i32) (result i32)
    (local $ret i32)
    global.get $heap
    local.set $ret
    global.get $heap
    local.get $new_size
    i32.add
    global.set $heap
    local.get $ret)
  (func $_initialize)

  (export "near:product-adapter/product-adapter@0.1.0#manifest" (func $manifest))
  (export "cabi_post_near:product-adapter/product-adapter@0.1.0#manifest" (func $post))
  (export "near:product-adapter/product-adapter@0.1.0#parse-inbound" (func $parse_inbound))
  (export "cabi_post_near:product-adapter/product-adapter@0.1.0#parse-inbound" (func $post))
  (export "near:product-adapter/product-adapter@0.1.0#render-outbound" (func $render_outbound))
  (export "cabi_post_near:product-adapter/product-adapter@0.1.0#render-outbound" (func $post))
  (export "cabi_realloc" (func $realloc))
  (export "_initialize" (func $_initialize))
)
"#;

fn product_adapter_component(wat_src: &str) -> Vec<u8> {
    let mut module = wat::parse_str(wat_src).expect("fixture WAT must parse");
    let mut resolve = Resolve::default();
    let package = resolve
        .push_str(
            "product_adapter.wit",
            include_str!("../wit/product_adapter.wit"),
        )
        .expect("product adapter WIT must parse");
    let world = resolve
        .select_world(&[package], Some("product-adapter-component"))
        .expect("product adapter world must exist");

    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8)
        .expect("component metadata must embed");

    ComponentEncoder::default()
        .module(&module)
        .expect("fixture module must decode")
        .validate(true)
        .encode()
        .expect("component must encode")
}

#[test]
fn prepares_manifest_from_product_adapter_component() {
    let runtime =
        ProductAdapterComponentRuntime::new(ProductAdapterComponentRuntimeConfig::for_testing())
            .expect("runtime");
    let prepared = runtime
        .prepare("fixture", &product_adapter_component(FIXTURE_ADAPTER_WAT))
        .expect("prepare");

    assert_eq!(prepared.name(), "fixture");
    assert_eq!(prepared.manifest().adapter_id.as_str(), "fixture_adapter");
    assert_eq!(prepared.manifest().installation_id.as_str(), "install_1");
    assert_eq!(prepared.manifest().capabilities_json, r#"{"flags":[]}"#);
    assert_eq!(prepared.manifest().declared_egress_targets.len(), 1);
    assert_eq!(
        prepared.manifest().declared_egress_targets[0].host.as_str(),
        "api.example.com"
    );
    assert_eq!(prepared.egress_policy().declared_hosts().count(), 1);
}

#[test]
fn calls_parse_and_render_exports() {
    let runtime =
        ProductAdapterComponentRuntime::new(ProductAdapterComponentRuntimeConfig::for_testing())
            .expect("runtime");
    let prepared = runtime
        .prepare("fixture", &product_adapter_component(FIXTURE_ADAPTER_WAT))
        .expect("prepare");

    let evidence = mark_bearer_token_verified("alice");
    let parsed = runtime
        .parse_inbound(&prepared, br#"{"hello":true}"#, &evidence)
        .expect("parse");
    assert!(parsed.parsed_json.contains(r#""external_event_id":"evt1""#));

    let rendered = runtime
        .render_outbound(&prepared, r#"{"payload":"out"}"#)
        .expect("render");
    let request = &rendered.egress_request;
    assert_eq!(request.host().as_str(), "api.example.com");
    assert_eq!(request.method().as_str(), "POST");
    assert_eq!(request.path().as_str(), "/send");
    assert!(request.headers().is_empty());
    assert!(request.body().is_empty());
    assert!(request.credential_handle().is_none());
}

#[test]
fn prepared_component_is_safe_to_share_across_host_calls() {
    fn assert_send_sync<T: Send + Sync>() {}

    assert_send_sync::<ironclaw_wasm_product_adapters::PreparedProductAdapterComponent>();
}

#[test]
fn parse_caps_component_logs_at_runtime_boundary() {
    let runtime =
        ProductAdapterComponentRuntime::new(ProductAdapterComponentRuntimeConfig::for_testing())
            .expect("runtime");
    let log_spam_wat = FIXTURE_ADAPTER_WAT
        .replacen(
            "(module\n",
            "(module\n  (import \"near:product-adapter/product-adapter-host@0.1.0\" \"log\" (func $host_log (param i32 i32 i32)))\n",
            1,
        )
        .replace(
            "  (func $parse_inbound (param $raw_ptr i32) (param $raw_len i32) (param $evidence_ptr i32) (param $evidence_len i32) (result i32)\n    ;; ok(parsed-inbound { parsed-json })\n    i32.const 128",
            "  (func $parse_inbound (param $raw_ptr i32) (param $raw_len i32) (param $evidence_ptr i32) (param $evidence_len i32) (result i32)\n    (local $i i32)\n    (loop $emit\n      i32.const 2\n      i32.const 1024\n      i32.const 15\n      call $host_log\n      local.get $i\n      i32.const 1\n      i32.add\n      local.tee $i\n      i32.const 1001\n      i32.lt_u\n      br_if $emit)\n    ;; ok(parsed-inbound { parsed-json })\n    i32.const 128",
        );
    let prepared = runtime
        .prepare("fixture", &product_adapter_component(&log_spam_wat))
        .expect("prepare");
    let evidence = mark_bearer_token_verified("alice");

    let parsed = runtime
        .parse_inbound(&prepared, br#"{"hello":true}"#, &evidence)
        .expect("parse");

    assert_eq!(parsed.logs.len(), 1_000);
    assert_eq!(
        parsed.logs.last().expect("last retained log").message,
        "fixture_adapter"
    );
}

#[test]
fn parse_rejects_json_that_is_not_parsed_product_inbound() {
    let runtime =
        ProductAdapterComponentRuntime::new(ProductAdapterComponentRuntimeConfig::for_testing())
            .expect("runtime");
    let invalid_parse_wat = FIXTURE_ADAPTER_WAT
        .replace(
            "{\\22external_event_id\\22:\\22evt1\\22,\\22external_actor_ref\\22:{\\22kind\\22:\\22user\\22,\\22id\\22:\\22u1\\22,\\22display_name\\22:null},\\22external_conversation_ref\\22:{\\22space_id\\22:null,\\22conversation_id\\22:\\22c1\\22,\\22topic_id\\22:null,\\22reply_target_message_id\\22:null},\\22payload\\22:\\22no_op\\22}",
            "{\\22payload\\22:\\22parsed\\22}",
        )
        .replace(
            "    i32.const 136\n    i32.const 229\n    i32.store\n",
            "    i32.const 136\n    i32.const 20\n    i32.store\n",
        );
    let prepared = runtime
        .prepare("fixture", &product_adapter_component(&invalid_parse_wat))
        .expect("prepare");
    let evidence = mark_bearer_token_verified("alice");

    let err = runtime
        .parse_inbound(&prepared, br#"{\"hello\":true}"#, &evidence)
        .expect_err("invalid parsed inbound DTO");

    assert!(
        matches!(err, RuntimeError::InvalidJson { field, .. }
            if field == "parsed-inbound.parsed-json"),
        "{err:?}"
    );
}

#[test]
fn render_rejects_fuel_exhaustion_with_usable_error() {
    let runtime =
        ProductAdapterComponentRuntime::new(ProductAdapterComponentRuntimeConfig::for_testing())
            .expect("runtime");
    let looping_render_wat = FIXTURE_ADAPTER_WAT.replace(
        "  (func $render_outbound (param $outbound_ptr i32) (param $outbound_len i32) (result i32)\n    ;; ok(outbound-render { egress-request-json })\n    i32.const 144\n    i32.const 0\n    i32.store\n    i32.const 148\n    i32.const 3072\n    i32.store\n    i32.const 152\n    i32.const 79\n    i32.store\n    i32.const 144)",
        "  (func $render_outbound (param $outbound_ptr i32) (param $outbound_len i32) (result i32)\n    (loop $spin br $spin)\n    unreachable)",
    );
    let prepared = runtime
        .prepare("fixture", &product_adapter_component(&looping_render_wat))
        .expect("prepare");

    let err = runtime
        .render_outbound(&prepared, r#"{"payload":"out"}"#)
        .expect_err("fuel exhaustion must fail render");

    assert!(
        matches!(err, RuntimeError::ExecutionFailed { ref message, .. }
            if message.to_ascii_lowercase().contains("fuel")),
        "{err:?}"
    );
}

#[test]
fn render_rejects_missing_typed_egress_fields() {
    let runtime =
        ProductAdapterComponentRuntime::new(ProductAdapterComponentRuntimeConfig::for_testing())
            .expect("runtime");
    let missing_fields_wat = FIXTURE_ADAPTER_WAT
        .replace(
            "{\\22egress_target_index\\22:0,\\22method\\22:\\22POST\\22,\\22path\\22:\\22/send\\22,\\22headers\\22:[],\\22body\\22:[]}",
            "{\\22egress_target_index\\22:0}",
        )
        .replace(
            "    i32.const 152\n    i32.const 79\n    i32.store\n",
            "    i32.const 152\n    i32.const 25\n    i32.store\n",
        );
    let prepared = runtime
        .prepare("fixture", &product_adapter_component(&missing_fields_wat))
        .expect("prepare");

    let err = runtime
        .render_outbound(&prepared, r#"{"payload":"out"}"#)
        .expect_err("missing typed egress fields");

    assert!(
        matches!(err, RuntimeError::InvalidJson { field, ref message }
            if field == "outbound-render.egress-request-json"
                && message.contains("method")),
        "{err:?}"
    );
}

#[test]
fn render_rejects_kebab_case_egress_target_index_json() {
    let runtime =
        ProductAdapterComponentRuntime::new(ProductAdapterComponentRuntimeConfig::for_testing())
            .expect("runtime");
    let kebab_case_index_wat = FIXTURE_ADAPTER_WAT.replace(
        "{\\22egress_target_index\\22:0,\\22method\\22:\\22POST\\22,\\22path\\22:\\22/send\\22,\\22headers\\22:[],\\22body\\22:[]}",
        "{\\22egress-target-index\\22:0,\\22method\\22:\\22POST\\22,\\22path\\22:\\22/send\\22,\\22headers\\22:[],\\22body\\22:[]}",
    );
    let prepared = runtime
        .prepare("fixture", &product_adapter_component(&kebab_case_index_wat))
        .expect("prepare");

    let err = runtime
        .render_outbound(&prepared, r#"{"payload":"out"}"#)
        .expect_err("kebab-case JSON shim field must be rejected");

    assert!(
        matches!(err, RuntimeError::InvalidJson { field, ref message }
            if field == "outbound-render.egress-request-json"
                && message.contains("egress_target_index")),
        "{err:?}"
    );
}

#[test]
fn render_rejects_host_managed_headers() {
    let runtime =
        ProductAdapterComponentRuntime::new(ProductAdapterComponentRuntimeConfig::for_testing())
            .expect("runtime");
    let forbidden_header_wat = FIXTURE_ADAPTER_WAT
        .replace(
            "{\\22egress_target_index\\22:0,\\22method\\22:\\22POST\\22,\\22path\\22:\\22/send\\22,\\22headers\\22:[],\\22body\\22:[]}",
            "{\\22egress_target_index\\22:0,\\22method\\22:\\22POST\\22,\\22path\\22:\\22/send\\22,\\22headers\\22:[{\\22name\\22:\\22Authorization\\22,\\22value\\22:\\22secret\\22}],\\22body\\22:[]}",
        )
        .replace(
            "    i32.const 152\n    i32.const 79\n    i32.store\n",
            "    i32.const 152\n    i32.const 120\n    i32.store\n",
        );
    let prepared = runtime
        .prepare("fixture", &product_adapter_component(&forbidden_header_wat))
        .expect("prepare");

    let err = runtime
        .render_outbound(&prepared, r#"{"payload":"out"}"#)
        .expect_err("host managed header");

    assert!(
        matches!(err, RuntimeError::InvalidJson { field, ref message }
            if field == "outbound-render.egress-request-json"
                && message.contains("header is managed by the host")),
        "{err:?}"
    );
}

#[test]
fn render_rejects_egress_target_not_declared_by_manifest() {
    let runtime =
        ProductAdapterComponentRuntime::new(ProductAdapterComponentRuntimeConfig::for_testing())
            .expect("runtime");
    let undeclared_target_wat = FIXTURE_ADAPTER_WAT.replace(
        "    i32.const 44\n    i32.const 1\n    i32.store\n",
        "    i32.const 44\n    i32.const 0\n    i32.store\n",
    );
    let prepared = runtime
        .prepare(
            "fixture",
            &product_adapter_component(&undeclared_target_wat),
        )
        .expect("prepare");

    let err = runtime
        .render_outbound(&prepared, r#"{"payload":"out"}"#)
        .expect_err("undeclared render target");
    assert!(
        matches!(err, RuntimeError::InvalidJson { field, ref message }
            if field == "outbound-render.egress-request-json"
                && message.contains("egress_target_index 0 is not declared")),
        "{err:?}"
    );
}

#[test]
fn malformed_component_bytes_are_rejected() {
    let runtime =
        ProductAdapterComponentRuntime::new(ProductAdapterComponentRuntimeConfig::for_testing())
            .expect("runtime");
    let err = runtime
        .prepare("bad", b"not wasm")
        .expect_err("bad component");
    assert!(matches!(err, RuntimeError::CompilationFailed(_)), "{err:?}");
}

#[test]
fn oversized_component_bytes_are_rejected_before_compilation() {
    let config = ProductAdapterComponentRuntimeConfig {
        max_component_bytes: 4,
        ..ProductAdapterComponentRuntimeConfig::for_testing()
    };
    let runtime = ProductAdapterComponentRuntime::new(config).expect("runtime");
    let err = runtime
        .prepare("oversized", b"not wasm but too large")
        .expect_err("oversized component must be rejected before compile");

    assert!(
        matches!(err, RuntimeError::ComponentTooLarge { actual, limit }
            if actual == b"not wasm but too large".len() && limit == 4),
        "{err:?}"
    );
}

#[test]
fn component_without_product_adapter_exports_is_rejected() {
    let runtime =
        ProductAdapterComponentRuntime::new(ProductAdapterComponentRuntimeConfig::for_testing())
            .expect("runtime");
    let err = runtime
        .prepare(
            "empty",
            &wat::parse_str("(component)").expect("component wat"),
        )
        .expect_err("missing exports");
    assert!(
        matches!(err, RuntimeError::InstantiationFailed(_)),
        "{err:?}"
    );
}

/// JSON above the host's own size ceiling is rejected before any serde
/// allocation. The WASM memory cap bounds component-returned strings; this
/// host-side ceiling protects the host from operators who raise that cap.
///
/// We exercise the same `ensure_json_within_host_budget` helper through the
/// `render_outbound` host-input path (the outbound envelope flows through
/// `ensure_json` before the component is invoked).
#[test]
fn render_rejects_oversized_host_envelope_before_serde() {
    let runtime =
        ProductAdapterComponentRuntime::new(ProductAdapterComponentRuntimeConfig::for_testing())
            .expect("runtime");
    let prepared = runtime
        .prepare("fixture", &product_adapter_component(FIXTURE_ADAPTER_WAT))
        .expect("prepare");

    // 2 MiB envelope — well above MAX_COMPONENT_JSON_BYTES (1 MiB). Valid
    // JSON shape so the rejection has to come from the size check, not from
    // serde structural validation.
    let filler = "a".repeat(2 * 1024 * 1024);
    let oversized = format!(r#"{{"payload":"{filler}"}}"#);

    let err = runtime
        .render_outbound(&prepared, &oversized)
        .expect_err("oversized envelope must be rejected");
    assert!(
        matches!(err, RuntimeError::InvalidJson { field, ref message }
            if field == "outbound-envelope.outbound-json"
                && message.contains("host limit")),
        "{err:?}"
    );
}
