use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    process::Command,
};

use serde_json::Value;

#[test]
fn reborn_boundary_rules_active_crates_are_workspace_members() {
    // Regression for PR #3212 review: a boundary rule whose crate has a
    // `Cargo.toml` on disk but is missing from `cargo metadata` would
    // previously fail open in `assert_no_normal_workspace_deps`, masking
    // forbidden edges in the unregistered crate. Each active rule must
    // either name a crate that has no directory yet (future-only,
    // tolerated) or a crate that is in the workspace metadata.
    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");
    let registered = packages
        .iter()
        .filter_map(|package| package["name"].as_str().map(ToString::to_string))
        .collect::<std::collections::HashSet<_>>();

    let root = workspace_root();
    for rule in boundary_rules() {
        let crate_dir = root.join("crates").join(rule.crate_name);
        let manifest = crate_dir.join("Cargo.toml");
        if !manifest.exists() {
            continue;
        }
        assert!(
            registered.contains(rule.crate_name),
            "{} has a Cargo.toml at {} but is not registered as a workspace member; \
             add it to the root `Cargo.toml` `workspace.members` so its boundary rule \
             is actually checked",
            rule.crate_name,
            manifest.display()
        );
    }
}

#[test]
fn reborn_virtual_roots_match_storage_placement_contract() {
    let root = workspace_root();
    let path_source = std::fs::read_to_string(root.join("crates/ironclaw_host_api/src/path.rs"))
        .expect("host API path source must be readable");
    let storage_contract =
        std::fs::read_to_string(root.join("docs/reborn/contracts/storage-placement.md"))
            .expect("storage placement contract must be readable");
    let filesystem_contract =
        std::fs::read_to_string(root.join("docs/reborn/contracts/filesystem.md"))
            .expect("filesystem contract must be readable");

    let implemented = extract_virtual_roots_const(&path_source);
    let storage = extract_storage_placement_roots(&storage_contract);
    let filesystem = extract_filesystem_namespace_roots(&filesystem_contract);

    assert_eq!(
        implemented, storage,
        "ironclaw_host_api VIRTUAL_ROOTS must match storage-placement.md canonical roots"
    );
    assert_eq!(
        filesystem, storage,
        "filesystem.md namespace roots must match storage-placement.md canonical roots"
    );
}

#[test]
fn reborn_crate_dependency_boundaries_hold() {
    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");
    let dependencies = packages
        .iter()
        .filter_map(package_dependencies)
        .collect::<HashMap<_, _>>();

    assert_no_normal_workspace_deps(
        &dependencies,
        "ironclaw_host_api",
        workspace_ironclaw_crates(&dependencies)
            .into_iter()
            .filter(|name| *name != "ironclaw_host_api")
            .collect::<Vec<_>>(),
    );

    for rule in boundary_rules() {
        assert_no_normal_workspace_deps(&dependencies, rule.crate_name, rule.forbidden);
    }
}

#[test]
fn reborn_cli_binary_crate_stays_separate_from_v1_root() {
    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");
    let dependencies = packages
        .iter()
        .filter_map(package_dependencies)
        .collect::<HashMap<_, _>>();
    let dependencies_all_kinds = packages
        .iter()
        .filter_map(package_dependencies_all_kinds)
        .collect::<HashMap<_, _>>();

    let root = workspace_root();
    let manifest_path = root.join("crates/ironclaw_reborn_cli/Cargo.toml");
    assert!(
        manifest_path.exists(),
        "Reborn should ship as a separate binary crate at {}",
        manifest_path.display()
    );

    let manifest =
        std::fs::read_to_string(&manifest_path).expect("Reborn CLI manifest must be readable");
    assert!(
        manifest.contains("name = \"ironclaw_reborn_cli\""),
        "Reborn CLI crate package name should be ironclaw_reborn_cli"
    );
    assert!(
        manifest.contains("[[bin]]") && manifest.contains("name = \"ironclaw-reborn\""),
        "Reborn CLI crate must declare the ironclaw-reborn binary explicitly"
    );

    let command_module_paths = [
        "crates/ironclaw_reborn_cli/AGENTS.md",
        "crates/ironclaw_reborn_cli/src/commands/mod.rs",
        "crates/ironclaw_reborn_cli/src/commands/completion.rs",
        "crates/ironclaw_reborn_cli/src/commands/doctor.rs",
        "crates/ironclaw_reborn_cli/src/commands/run.rs",
        "crates/ironclaw_reborn_cli/src/context.rs",
    ];
    for path in command_module_paths {
        assert!(
            root.join(path).exists(),
            "Reborn CLI commands should use an agent-friendly one-command-per-file layout; missing {path}"
        );
    }

    let agent_contract = std::fs::read_to_string(root.join("crates/ironclaw_reborn_cli/AGENTS.md"))
        .expect("Reborn CLI crate-local AGENTS.md must be readable");
    for required_phrase in [
        "one command per file",
        "RebornCliContext",
        "no v1 runtime imports",
    ] {
        assert!(
            agent_contract.contains(required_phrase),
            "Reborn CLI AGENTS.md should document `{required_phrase}` for future command agents"
        );
    }

    assert_workspace_deps_exactly(
        &dependencies,
        "ironclaw_reborn_cli",
        ["ironclaw_reborn", "ironclaw_reborn_config"],
        "ironclaw_reborn_cli should enter Reborn through ironclaw_reborn and ironclaw_reborn_config only; add explicit architectural justification before depending on other workspace crates",
    );
    assert_workspace_deps_exactly(
        &dependencies_all_kinds,
        "ironclaw_reborn_config",
        [],
        "ironclaw_reborn_config must remain a standalone boot contract crate with no IronClaw workspace dependencies of any dependency kind",
    );
}

#[test]
fn reborn_host_runtime_services_do_not_expose_lower_substrate_handles() {
    let root = workspace_root();
    let lib = std::fs::read_to_string(root.join("crates/ironclaw_host_runtime/src/lib.rs"))
        .expect("host runtime lib.rs must be readable");
    let services =
        std::fs::read_to_string(root.join("crates/ironclaw_host_runtime/src/services.rs"))
            .expect("host runtime services.rs must be readable");

    let forbidden_lib_exports = [
        "RuntimeDispatchProcessExecutor",
        "ScriptRuntimeAdapter",
        "McpRuntimeAdapter",
        "WasmRuntimeAdapter",
    ];
    for export in forbidden_lib_exports {
        assert!(
            !lib.contains(export),
            "ironclaw_host_runtime must not re-export lower substrate handle `{export}`; upper Reborn code should enter through HostRuntimeServices::host_runtime / Arc<dyn HostRuntime>"
        );
    }

    let forbidden_public_services = [
        "pub fn registry(",
        "pub fn filesystem(",
        "pub fn governor(",
        "pub fn authorizer(",
        "pub fn process_services(",
        "pub fn process_host(",
        "pub fn with_wasm_runtime(",
        "pub fn runtime_dispatcher(",
        "pub fn runtime_dispatcher_arc(",
        "pub fn capability_host",
        "pub struct RuntimeDispatchProcessExecutor",
        "pub struct ScriptRuntimeAdapter",
        "pub struct McpRuntimeAdapter",
        "pub struct WasmRuntimeAdapter",
    ];
    for pattern in forbidden_public_services {
        assert!(
            !services.contains(pattern),
            "HostRuntimeServices must not expose lower substrate escape hatch `{pattern}`; keep dispatcher/capability/process handles private to the host-runtime crate"
        );
    }
}

#[test]
fn reborn_turns_public_surface_keeps_runner_api_explicit() {
    let root = workspace_root();
    let lib = std::fs::read_to_string(root.join("crates/ironclaw_turns/src/lib.rs"))
        .expect("turns lib.rs must be readable");

    let forbidden_public_exports = [
        "pub use runner::",
        "pub use crate::runner::",
        "pub use self::runner::",
    ];
    for pattern in forbidden_public_exports {
        assert!(
            !lib.contains(pattern),
            "ironclaw_turns public prelude must not re-export trusted runner transition API `{pattern}`; adapters must import ironclaw_turns::runner explicitly"
        );
    }
}

#[test]
fn reborn_loop_support_llm_wiring_stays_out_of_root_src() {
    let root = workspace_root();
    let root_lib =
        std::fs::read_to_string(root.join("src/lib.rs")).expect("root src/lib.rs must be readable");
    assert!(
        !root_lib.contains("pub mod reborn_loop_support;"),
        "Reborn loop LLM wiring must live under crates/ironclaw_reborn, not root src/lib.rs"
    );
    assert!(
        !root.join("src/reborn_loop_support.rs").exists(),
        "Reborn loop LLM wiring must not live under root src/"
    );

    let reborn_gateway = root.join("crates/ironclaw_reborn/src/model_gateway.rs");
    assert!(
        reborn_gateway.exists(),
        "expected Reborn LLM gateway wiring at {}",
        reborn_gateway.display()
    );
    let reborn_gateway_source = std::fs::read_to_string(&reborn_gateway)
        .expect("Reborn model gateway source must be readable");
    assert!(
        reborn_gateway_source.contains("LlmProviderModelGateway"),
        "Reborn LLM gateway wiring should expose LlmProviderModelGateway from crates/ironclaw_reborn"
    );

    let reborn_manifest = std::fs::read_to_string(root.join("crates/ironclaw_reborn/Cargo.toml"))
        .expect("Reborn manifest must be readable");
    assert!(
        reborn_manifest.contains("optional = true")
            && reborn_manifest.contains("default-features = false")
            && reborn_manifest.contains("root-llm-provider"),
        "ironclaw_reborn may reuse root LLM code only behind an explicit feature, without enabling the root app's default postgres/libsql/tui feature set"
    );
}

#[test]
fn reborn_turns_public_surface_uses_turn_ids_not_runtime_or_process_ids() {
    let root = workspace_root();
    let turns_src = root.join("crates/ironclaw_turns/src");
    let mut violations = Vec::new();
    collect_forbidden_turns_identifier_uses(&turns_src, &root, &mut violations);

    assert!(
        violations.is_empty(),
        "ironclaw_turns public API must use TurnId/TurnRunId instead of lower runtime/process identifiers:\n{}",
        violations.join("\n")
    );
}

#[test]
fn wasm_product_adapter_crate_has_local_guardrails() {
    let guardrails = workspace_root().join("crates/ironclaw_wasm_product_adapters/CLAUDE.md");
    assert!(
        guardrails.exists(),
        "ironclaw_wasm_product_adapters needs local CLAUDE.md guardrails before becoming a Reborn boundary crate"
    );
}

#[test]
fn wasm_product_adapter_crate_keeps_minimal_host_glue_dependencies() {
    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");
    let package = packages
        .iter()
        .find(|package| package["name"] == "ironclaw_wasm_product_adapters")
        .expect("ironclaw_wasm_product_adapters must be a workspace package");
    let mut deps = package["dependencies"]
        .as_array()
        .expect("dependencies")
        .iter()
        .filter(|dependency| is_normal_dependency(dependency))
        .filter_map(|dependency| dependency["name"].as_str())
        .collect::<Vec<_>>();
    deps.sort_unstable();

    // Deliberate additions beyond the original auth/egress primitives:
    //   * async-trait, tokio  — required by the native ProductAdapter runner
    //     (async traits, semaphore-based admission control, timeout).
    //   * chrono              — receive-timestamp for TrustedInboundContext.
    //   * hex                 — HMAC signature encoding in the auth verifier.
    //   * tracing             — structured logging for hardened error paths
    //                           added in the zmanian review.
    // Every addition is justified by a concrete call site in src/. Adding a
    // dep here without a matching call site is a contract violation — and
    // adding workflow/runtime crates beyond this list still requires
    // updating both the wasm crate's CLAUDE.md and this expected set.
    let expected = vec![
        "async-trait",
        "chrono",
        "hex",
        "hmac",
        "http",
        "ironclaw_product_adapters",
        "sha2",
        "subtle",
        "thiserror",
        "tokio",
        "tracing",
    ];
    assert_eq!(
        deps, expected,
        "ironclaw_wasm_product_adapters should stay thin host glue; add runtime/workflow dependencies only when a call-site proves they are required"
    );
}

#[test]
fn wasm_product_adapter_wit_preserves_product_adapter_trust_boundary() {
    let wit = std::fs::read_to_string(
        workspace_root().join("crates/ironclaw_wasm_product_adapters/wit/product_adapter.wit"),
    )
    .expect("product adapter WIT must be readable");

    assert!(
        wit.contains("record parsed-inbound"),
        "WIT should name adapter output as ParsedProductInbound, not a trusted envelope"
    );
    assert!(
        wit.contains("result<parsed-inbound, string>"),
        "parse-inbound should return a parsed inbound payload; host glue stamps TrustedInboundContext and builds ProductInboundEnvelope"
    );
    for forbidden in [
        "result<option<parsed-envelope>",
        "record parsed-envelope",
        "envelope-json",
        "Returns `none`",
        "ProductInboundEnvelope",
    ] {
        assert!(
            !wit.contains(forbidden),
            "WIT must not use `{forbidden}`; no-op events are ProductInboundPayload::NoOp and envelopes are host-stamped"
        );
    }

    let response_record = wit
        .split("record egress-response {")
        .nth(1)
        .and_then(|rest| rest.split('}').next())
        .expect("egress-response record must exist");
    assert!(
        !response_record.contains("headers"),
        "WASM egress responses must not expose raw response headers to adapters"
    );
}

#[test]
fn wasm_product_adapter_wit_declares_egress_targets_as_paired_records() {
    // Henry's review (PR #3352, 2026-05-12T05:04:30Z) flagged that the
    // WIT manifest previously exposed `declared-egress-hosts: list<string>`
    // and `declared-credential-handles: list<string>` as independent
    // lists, which contradicted the Rust `EgressPolicy` that now
    // requires exact `(host, Option<credential_handle>)` pairs.
    // Independent lists could not express "Slack token only for Slack",
    // forcing the future host glue to either reintroduce the cross-pair
    // leak the Rust policy closes or invent pair metadata the manifest
    // did not carry.
    //
    // The WIT now declares a `declared-egress-target` record and the
    // manifest carries `declared-egress-targets: list<declared-egress-
    // target>`. This boundary test pins the new shape — a regression
    // that splits the pair back into independent lists fails here.
    let wit = std::fs::read_to_string(
        workspace_root().join("crates/ironclaw_wasm_product_adapters/wit/product_adapter.wit"),
    )
    .expect("product adapter WIT must be readable");

    assert!(
        wit.contains("record declared-egress-target"),
        "WIT must declare a paired `declared-egress-target` record so the manifest can express the (host, optional credential_handle) contract the Rust EgressPolicy enforces"
    );
    assert!(
        wit.contains("declared-egress-targets: list<declared-egress-target>"),
        "adapter-manifest must carry `declared-egress-targets: list<declared-egress-target>` (paired) instead of independent host/handle lists"
    );

    // Egress-request must reference the paired target by a single
    // index, not split into separate host/handle indexes — the WIT
    // shape mirrors the Rust pair-contract.
    assert!(
        wit.contains("egress-target-index: u32"),
        "egress-request must reference a single paired target via `egress-target-index`"
    );

    // Forbidden: the prior independent-list shape and split indexes
    // that allowed cross-pair leak by construction.
    for forbidden in [
        "declared-egress-hosts: list<string>",
        "declared-credential-handles: list<string>",
        "host-index: u32",
        "credential-handle-index: option<u32>",
    ] {
        assert!(
            !wit.contains(forbidden),
            "WIT must not carry the prior independent-list / split-index shape `{forbidden}` — it could not express the paired Rust contract and would reintroduce the cross-pair credential leak"
        );
    }
}

#[test]
fn wasm_product_adapter_wit_pins_json_shim_shape() {
    // Henry's review on PR #3352 flagged that the WIT carries adapter
    // payloads as JSON strings (`parsed-json`, `evidence-json`,
    // `outbound-json`, `egress-request-json`, `capabilities-json`),
    // which weakens the typed component-model boundary. The host
    // re-validates every JSON crossing on the Rust side via serde, so
    // the seal contract still holds — but the typed redesign is a
    // followup. This test pins the current shim shape so a future
    // change must EITHER:
    //   (a) update this test alongside the corresponding typed record
    //       (deliberate redesign), OR
    //   (b) fail boundary checks (accidental shape drift).
    let wit = std::fs::read_to_string(
        workspace_root().join("crates/ironclaw_wasm_product_adapters/wit/product_adapter.wit"),
    )
    .expect("product adapter WIT must be readable");

    // Top-level documentation MUST call out the shim explicitly so a
    // reviewer doesn't have to infer the intent from the field names.
    for required_doc in ["TEMPORARY", "JSON-string payload shim", "Follow-up"] {
        assert!(
            wit.contains(required_doc),
            "WIT must document the JSON-shim status (`{required_doc}` missing); \
             see top-of-file comment block before `package`"
        );
    }

    // The five known JSON-shim fields. Each is the temporary surface
    // covering a typed Rust DTO in `ironclaw_product_adapters`.
    let shim_fields = [
        ("parsed-inbound", "parsed-json: string"),
        ("auth-evidence", "evidence-json: string"),
        ("outbound-envelope", "outbound-json: string"),
        ("outbound-render", "egress-request-json: string"),
        ("adapter-manifest", "capabilities-json: string"),
    ];
    for (record, field) in shim_fields {
        assert!(
            wit.contains(field),
            "WIT JSON-shim field `{field}` in record `{record}` is missing. \
             If you removed it as part of a typed redesign, update this test \
             to assert the new typed shape instead — otherwise the boundary \
             is silently drifting"
        );
    }
}

#[test]
fn reborn_runtime_http_egress_has_single_network_boundary() {
    let forbidden = [
        ForbiddenRuntimeNetworkUse {
            pattern: "reqwest::Client",
            reason: "runtime crates must use ironclaw_network for outbound HTTP transport",
        },
        ForbiddenRuntimeNetworkUse {
            pattern: "reqwest::blocking::Client",
            reason: "runtime crates must use ironclaw_network for outbound HTTP transport",
        },
        ForbiddenRuntimeNetworkUse {
            pattern: "reqwest::ClientBuilder",
            reason: "runtime crates must use ironclaw_network for outbound HTTP transport",
        },
        ForbiddenRuntimeNetworkUse {
            pattern: "ToSocketAddrs",
            reason: "runtime crates must not perform ad-hoc DNS resolution",
        },
        ForbiddenRuntimeNetworkUse {
            pattern: ".to_socket_addrs(",
            reason: "runtime crates must not perform ad-hoc DNS resolution",
        },
        ForbiddenRuntimeNetworkUse {
            pattern: "ssrf_safe_client_builder",
            reason: "runtime crates must not reuse V1 WASM SSRF helpers",
        },
        ForbiddenRuntimeNetworkUse {
            pattern: "validate_and_resolve_http_target",
            reason: "runtime crates must not reuse V1 WASM SSRF helpers",
        },
        ForbiddenRuntimeNetworkUse {
            pattern: "reject_private_ip",
            reason: "runtime crates must not perform ad-hoc SSRF checks",
        },
        ForbiddenRuntimeNetworkUse {
            pattern: "is_private_or_loopback_ip",
            reason: "runtime crates must not perform ad-hoc private-IP checks",
        },
    ];

    let root = workspace_root();
    let runtime_src_roots = [
        "crates/ironclaw_wasm/src",
        "crates/ironclaw_scripts/src",
        "crates/ironclaw_mcp/src",
        "crates/ironclaw_host_runtime/src",
    ];

    let mut violations = Vec::new();
    for relative_root in runtime_src_roots {
        let dir = root.join(relative_root);
        if !dir.exists() {
            continue;
        }
        collect_forbidden_runtime_network_uses(&dir, &root, &forbidden, &mut violations);
    }

    assert!(
        violations.is_empty(),
        "Reborn runtime HTTP must use the shared host egress service and ironclaw_network only:\n{}",
        violations.join("\n")
    );
}

struct ForbiddenRuntimeNetworkUse {
    pattern: &'static str,
    reason: &'static str,
}

fn collect_forbidden_turns_identifier_uses(
    dir: &std::path::Path,
    root: &std::path::Path,
    violations: &mut Vec<String>,
) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}"));
        let path = entry.path();
        if path.is_dir() {
            collect_forbidden_turns_identifier_uses(&path, root, violations);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        for pattern in ["InvocationId", "ProcessId"] {
            if contents.contains(pattern) {
                violations.push(format!(
                    "{} contains forbidden lower identifier `{pattern}`",
                    path.strip_prefix(root).unwrap_or(&path).display()
                ));
            }
        }
    }
}

struct BoundaryRule {
    crate_name: &'static str,
    forbidden: Vec<&'static str>,
}

fn boundary_rules() -> Vec<BoundaryRule> {
    vec![
        BoundaryRule {
            crate_name: "ironclaw_product_workflow",
            forbidden: vec![
                "ironclaw_dispatcher",
                "ironclaw_extensions",
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_wasm",
                "ironclaw_scripts",
                "ironclaw_network",
                "ironclaw_engine",
                "ironclaw_gateway",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_storage",
            forbidden: vec![
                "ironclaw",
                "ironclaw_approvals",
                "ironclaw_architecture",
                "ironclaw_authorization",
                "ironclaw_capabilities",
                "ironclaw_common",
                "ironclaw_conversations",
                "ironclaw_dispatcher",
                "ironclaw_engine",
                "ironclaw_event_projections",
                "ironclaw_events",
                "ironclaw_extensions",
                "ironclaw_filesystem",
                "ironclaw_gateway",
                "ironclaw_host_api",
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_loop_support",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_product_adapters",
                "ironclaw_reborn",
                "ironclaw_reborn_cli",
                "ironclaw_reborn_config",
                "ironclaw_reborn_event_store",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_scripts",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_turns",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_reborn_config",
            forbidden: vec![
                "ironclaw",
                "ironclaw_approvals",
                "ironclaw_authorization",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_dispatcher",
                "ironclaw_engine",
                "ironclaw_events",
                "ironclaw_extensions",
                "ironclaw_filesystem",
                "ironclaw_gateway",
                "ironclaw_host_api",
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_loop_support",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_product_adapters",
                "ironclaw_reborn",
                "ironclaw_reborn_event_store",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_scripts",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_turns",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_reborn_cli",
            forbidden: vec![
                "ironclaw",
                "ironclaw_engine",
                "ironclaw_gateway",
                "ironclaw_skills",
                "ironclaw_tui",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_filesystem",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_dispatcher",
                "ironclaw_events",
                "ironclaw_extensions",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_scripts",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_memory",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_dispatcher",
                "ironclaw_events",
                "ironclaw_extensions",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_scripts",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_resources",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_dispatcher",
                "ironclaw_events",
                "ironclaw_extensions",
                "ironclaw_filesystem",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_run_state",
                "ironclaw_scripts",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_trust",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_dispatcher",
                "ironclaw_events",
                "ironclaw_extensions",
                "ironclaw_filesystem",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_scripts",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_extensions",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_dispatcher",
                "ironclaw_events",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_scripts",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_events",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_dispatcher",
                "ironclaw_extensions",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_scripts",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_event_projections",
            forbidden: vec![
                "ironclaw",
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_dispatcher",
                "ironclaw_extensions",
                "ironclaw_filesystem",
                "ironclaw_host_runtime",
                "ironclaw_reborn_event_store",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_scripts",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_outbound",
            forbidden: vec![
                "ironclaw",
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_dispatcher",
                "ironclaw_extensions",
                "ironclaw_filesystem",
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_processes",
                "ironclaw_reborn_event_store",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_safety",
                "ironclaw_scripts",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_tui",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_wasm_product_adapters",
            forbidden: vec![
                "ironclaw",
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_dispatcher",
                "ironclaw_engine",
                "ironclaw_events",
                "ironclaw_extensions",
                "ironclaw_filesystem",
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_reborn_event_store",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_scripts",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_threads",
                "ironclaw_tui",
                "ironclaw_turns",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_reborn_event_store",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_dispatcher",
                "ironclaw_extensions",
                "ironclaw_filesystem",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_scripts",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_secrets",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_dispatcher",
                "ironclaw_events",
                "ironclaw_extensions",
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_scripts",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_network",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_dispatcher",
                "ironclaw_events",
                "ironclaw_extensions",
                "ironclaw_filesystem",
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_scripts",
                "ironclaw_secrets",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_authorization",
            forbidden: vec![
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_dispatcher",
                "ironclaw_extensions",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_scripts",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_run_state",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_dispatcher",
                "ironclaw_events",
                "ironclaw_extensions",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_scripts",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_threads",
            forbidden: vec![
                "ironclaw",
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_dispatcher",
                "ironclaw_engine",
                "ironclaw_events",
                "ironclaw_extensions",
                "ironclaw_filesystem",
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_safety",
                "ironclaw_scripts",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_tui",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_approvals",
            forbidden: vec![
                "ironclaw_capabilities",
                "ironclaw_dispatcher",
                "ironclaw_extensions",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_scripts",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_processes",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_dispatcher",
                "ironclaw_extensions",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_run_state",
                "ironclaw_scripts",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_turns",
            forbidden: vec![
                "ironclaw_approvals",
                "ironclaw_authorization",
                "ironclaw_capabilities",
                "ironclaw_dispatcher",
                "ironclaw_extensions",
                "ironclaw_filesystem",
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_processes",
                "ironclaw_run_state",
                "ironclaw_scripts",
                "ironclaw_secrets",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_capabilities",
            forbidden: vec![
                "ironclaw_dispatcher",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_scripts",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_dispatcher",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_run_state",
                "ironclaw_scripts",
                "ironclaw_wasm",
            ],
        },
    ]
}

fn cargo_metadata() -> Value {
    let manifest_path = workspace_root().join("Cargo.toml");
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(&manifest_path)
        .output()
        .unwrap_or_else(|error| panic!("failed to run cargo metadata: {error}"));

    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata output must be JSON")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("architecture crate must live under crates/ironclaw_architecture")
        .to_path_buf()
}

fn extract_virtual_roots_const(source: &str) -> BTreeSet<String> {
    let const_body = source
        .split("const VIRTUAL_ROOTS: &[&str] = &[")
        .nth(1)
        .and_then(|tail| tail.split("];").next())
        .expect("VIRTUAL_ROOTS const array must be present");
    extract_quoted_absolute_paths(const_body)
}

fn extract_storage_placement_roots(contract: &str) -> BTreeSet<String> {
    contract
        .lines()
        .filter_map(|line| {
            let root = line
                .strip_prefix("| `")?
                .split('`')
                .next()
                .expect("table cell must close code span");
            let root = if root.starts_with("/engine/") {
                "/engine"
            } else {
                root
            };
            Some(root.to_string())
        })
        .filter(|root| is_canonical_virtual_root(root))
        .collect()
}

fn extract_filesystem_namespace_roots(contract: &str) -> BTreeSet<String> {
    let roots_block = contract
        .split("Frozen V1 canonical virtual roots")
        .nth(1)
        .and_then(|tail| tail.split("Recommended meaning:").next())
        .expect("filesystem.md must list frozen V1 canonical virtual roots");
    roots_block
        .lines()
        .map(str::trim)
        .filter(|line| is_canonical_virtual_root(line))
        .map(ToString::to_string)
        .collect()
}

fn extract_quoted_absolute_paths(source: &str) -> BTreeSet<String> {
    source
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix('"')?.split('"').next())
        .filter(|root| is_canonical_virtual_root(root))
        .map(ToString::to_string)
        .collect()
}

fn is_canonical_virtual_root(value: &str) -> bool {
    matches!(
        value,
        "/engine"
            | "/system/settings"
            | "/system/extensions"
            | "/system/skills"
            | "/users"
            | "/projects"
            | "/memory"
            | "/artifacts"
            | "/tmp"
            | "/secrets"
            | "/events"
    )
}

fn package_dependencies(package: &Value) -> Option<(String, Vec<String>)> {
    let name = package["name"].as_str()?.to_string();
    let dependencies = workspace_dependency_names(package)
        .filter(|dependency| is_normal_dependency(dependency))
        .filter_map(|dependency| dependency["name"].as_str())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    Some((name, dependencies))
}

fn package_dependencies_all_kinds(package: &Value) -> Option<(String, Vec<String>)> {
    let name = package["name"].as_str()?.to_string();
    let dependencies = workspace_dependency_names(package)
        .filter_map(|dependency| dependency["name"].as_str())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    Some((name, dependencies))
}

fn workspace_dependency_names(package: &Value) -> impl Iterator<Item = &Value> {
    package["dependencies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|dependency| {
            dependency["name"]
                .as_str()
                .is_some_and(|name| name == "ironclaw" || name.starts_with("ironclaw_"))
        })
}

fn is_normal_dependency(dependency: &Value) -> bool {
    dependency
        .get("kind")
        .and_then(Value::as_str)
        .is_none_or(|kind| kind == "normal")
}

fn workspace_ironclaw_crates(dependencies: &HashMap<String, Vec<String>>) -> Vec<&str> {
    dependencies
        .keys()
        .filter_map(|name| {
            (name == "ironclaw" || name.starts_with("ironclaw_")).then_some(name.as_str())
        })
        .collect()
}

fn assert_workspace_deps_exactly<'a>(
    dependencies: &HashMap<String, Vec<String>>,
    crate_name: &str,
    expected: impl IntoIterator<Item = &'a str>,
    message: &str,
) {
    let actual = dependencies
        .get(crate_name)
        .unwrap_or_else(|| panic!("{crate_name} must be in cargo metadata"))
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let expected = expected
        .into_iter()
        .map(ToString::to_string)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(actual, expected, "{message}");
}

fn assert_no_normal_workspace_deps<'a>(
    dependencies: &HashMap<String, Vec<String>>,
    crate_name: &str,
    forbidden: impl IntoIterator<Item = &'a str>,
) {
    let Some(actual) = dependencies.get(crate_name) else {
        // The landing plan introduces Reborn crates in grouped PRs. Boundary
        // rules become active as soon as their crate is present in the
        // workspace, while absent future crates are ignored in earlier slices.
        //
        // Fail closed when the crate directory is on disk but missing from
        // `cargo metadata` — that combination means the crate exists but
        // was never registered as a workspace member, so its forbidden
        // edges would otherwise silently pass without ever being checked.
        let crate_manifest = workspace_root()
            .join("crates")
            .join(crate_name)
            .join("Cargo.toml");
        assert!(
            !crate_manifest.exists(),
            "{crate_name} has a Cargo.toml at {} but is not in `cargo metadata` output; \
             add it to the root `Cargo.toml` `workspace.members` so the boundary rule \
             actually runs against its dependencies",
            crate_manifest.display()
        );
        return;
    };
    for forbidden in forbidden {
        assert!(
            !actual.iter().any(|dependency| dependency == forbidden),
            "{crate_name} must not have a normal dependency on {forbidden}; actual normal ironclaw deps: {actual:?}"
        );
    }
}

fn collect_forbidden_runtime_network_uses(
    dir: &std::path::Path,
    root: &std::path::Path,
    forbidden: &[ForbiddenRuntimeNetworkUse],
    violations: &mut Vec<String>,
) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| panic!("failed to read dir entry: {error}"));
        let path = entry.path();
        if path.is_dir() {
            collect_forbidden_runtime_network_uses(&path, root, forbidden, violations);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        for (line_number, line) in contents.lines().enumerate() {
            for rule in forbidden {
                if line.contains(rule.pattern) {
                    let relative = path.strip_prefix(root).unwrap_or(&path);
                    violations.push(format!(
                        "{}:{} contains `{}` ({})",
                        relative.display(),
                        line_number + 1,
                        rule.pattern,
                        rule.reason
                    ));
                }
            }
        }
    }
}
