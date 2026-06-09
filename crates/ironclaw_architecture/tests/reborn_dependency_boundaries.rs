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
fn conversation_trusted_trigger_submitter_stays_conversation_or_composition_owned() {
    let root = workspace_root();
    let mut uses = Vec::new();
    collect_forbidden_string_uses(
        &root.join("crates"),
        "ConversationTrustedTriggerSubmitter",
        &root,
        &mut uses,
    );
    let allowed = BTreeSet::from([
        "crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs",
        "crates/ironclaw_conversations/src/inbound.rs",
    ]);
    let violations = uses
        .into_iter()
        .filter(|path| !allowed.contains(path.as_str()))
        .collect::<Vec<_>>();

    assert!(
        violations.is_empty(),
        "Conversation trusted trigger submission must stay conversations/composition-owned; \
         product adapters and capabilities must use untrusted inbound requests. \
         Unexpected call sites:\n{}",
        violations.join("\n")
    );
}

#[test]
fn conversation_trusted_trigger_submitter_stays_out_of_root_exports() {
    let root = workspace_root();
    let lib_source = std::fs::read_to_string(root.join("crates/ironclaw_conversations/src/lib.rs"))
        .expect("conversation lib source must be readable");

    assert!(
        !lib_source.contains("ConversationTrustedTriggerSubmitter"),
        "ConversationTrustedTriggerSubmitter must not be re-exported from ironclaw_conversations; \
         composition should use the trusted_trigger_fire_submitter factory returning the trait object"
    );
}

#[test]
fn conversation_trusted_trigger_classifier_stays_out_of_root_exports() {
    let root = workspace_root();
    let lib_source = std::fs::read_to_string(root.join("crates/ironclaw_conversations/src/lib.rs"))
        .expect("conversation lib source must be readable");

    assert!(
        !lib_source.contains("classify_trusted_trigger_inbound_error"),
        "classify_trusted_trigger_inbound_error is submitter policy and must not be re-exported \
         from ironclaw_conversations; composition-owned materialization should classify its own \
         local errors"
    );
    assert!(
        !lib_source.contains("classify_inbound_error"),
        "trusted trigger inbound classification must not be re-exported from \
         ironclaw_conversations; keep it private to conversations-owned submitter policy"
    );
    assert!(
        !lib_source.contains("TrustedTriggerInboundFailureKind"),
        "trusted trigger inbound classification types must not be re-exported from \
         ironclaw_conversations; keep them private to conversations-owned submitter policy"
    );
    assert!(
        !lib_source.contains("pub mod trusted_trigger"),
        "trusted_trigger must stay a private implementation module; root exports should name only \
         the narrow symbols downstream composition needs"
    );
}

#[test]
fn trusted_trigger_submit_request_minting_stays_worker_owned() {
    let root = workspace_root();
    let mut struct_literal_uses = Vec::new();
    collect_forbidden_string_uses(
        &root.join("crates"),
        "TrustedTriggerSubmitRequest {",
        &root,
        &mut struct_literal_uses,
    );
    let allowed_struct_literals = BTreeSet::from([
        "crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs",
        "crates/ironclaw_triggers/src/worker/ports.rs",
    ]);
    let struct_literal_violations = struct_literal_uses
        .into_iter()
        .filter(|path| !allowed_struct_literals.contains(path.as_str()))
        .collect::<Vec<_>>();

    assert!(
        struct_literal_violations.is_empty(),
        "TrustedTriggerSubmitRequest fields must stay private; trusted trigger requests \
         are minted by the trigger worker, not by downstream submitter callers. \
         Unexpected struct literal use:\n{}",
        struct_literal_violations.join("\n")
    );
}

#[test]
fn retired_host_trusted_ingress_token_crate_stays_removed() {
    let root = workspace_root();
    let retired_crate_name = ["ironclaw", "trusted", "ingress"].join("_");
    assert!(
        !root
            .join("crates")
            .join(&retired_crate_name)
            .join("Cargo.toml")
            .exists(),
        "a separate trusted ingress crate must stay absent; trusted trigger \
         submission is sealed by ironclaw_triggers and privately converted inside \
         ironclaw_conversations"
    );

    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");
    let package_names = packages
        .iter()
        .filter_map(|package| package["name"].as_str())
        .collect::<BTreeSet<_>>();
    assert!(
        !package_names.contains(retired_crate_name.as_str()),
        "a separate trusted ingress crate must not be introduced as a workspace crate"
    );

    let dependencies = packages
        .iter()
        .filter_map(package_dependencies)
        .collect::<HashMap<_, _>>();
    let violations = dependencies
        .iter()
        .filter_map(|(crate_name, deps)| {
            deps.iter()
                .any(|dependency| dependency == retired_crate_name.as_str())
                .then_some(crate_name.as_str())
        })
        .collect::<Vec<_>>();

    assert!(
        violations.is_empty(),
        "a separate trusted ingress crate must not be introduced as a production dependency; \
         trusted trigger submission is now sealed by ironclaw_triggers and privately \
         converted inside ironclaw_conversations. Unexpected dependents:\n{}",
        violations.join("\n")
    );
}

#[test]
fn untrusted_ingress_paths_cannot_submit_host_trusted_inbound() {
    let root = workspace_root();
    let forbidden = [
        ForbiddenUse {
            pattern: "ConversationTrustedTriggerSubmitter",
            reason: "untrusted ingress paths must not construct conversation-owned trusted trigger submitters",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "trusted_trigger_fire_submitter",
            reason: "untrusted ingress paths must not build host-trusted trigger submitters",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "TrustedTriggerSubmitRequest",
            reason: "untrusted ingress paths must not submit host-trusted trigger fires",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "TrustedTriggerFireSubmitter",
            reason: "untrusted ingress paths must not implement host-trusted trigger submission",
            exempt: None,
        },
    ];
    let untrusted_src_roots = [
        "crates/ironclaw_capabilities/src",
        "crates/ironclaw_first_party_extension_ports/src",
        "crates/ironclaw_first_party_extensions/src",
        "crates/ironclaw_host_api/src",
        "crates/ironclaw_host_runtime/src",
        "crates/ironclaw_product_adapters/src",
        "crates/ironclaw_product_adapter_registry/src",
        "crates/ironclaw_product_workflow/src",
        "crates/ironclaw_product_workflow_storage/src",
        "crates/ironclaw_reborn_webui_ingress/src",
        "crates/ironclaw_wasm_product_adapters/src",
        "crates/ironclaw_webui_v2/src",
        "crates/ironclaw_telegram_v2_adapter/src",
        "crates/ironclaw_slack_v2_adapter/src",
    ];

    let mut violations = Vec::new();
    for relative_root in untrusted_src_roots {
        let dir = root.join(relative_root);
        if !dir.exists() {
            continue;
        }
        collect_forbidden_uses(&dir, &root, &forbidden, &mut violations);
    }

    assert!(
        violations.is_empty(),
        "Untrusted ingress, product, and capability paths must not submit or construct host-trusted synthetic inbound requests; \
         those operations belong to the conversations/composition boundary only:\n{}",
        violations.join("\n")
    );
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
        "crates/ironclaw_reborn_cli/src/commands/repl.rs",
        "crates/ironclaw_reborn_cli/src/commands/run.rs",
        "crates/ironclaw_reborn_cli/src/commands/serve.rs",
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
        [
            "ironclaw_reborn_composition",
            "ironclaw_reborn_config",
            "ironclaw_reborn_traces",
            "ironclaw_reborn_webui_ingress",
        ],
        "ironclaw_reborn_cli should enter Reborn through ironclaw_reborn_composition (assembled-runtime and provider-admin facade), ironclaw_reborn_config (boot-config contract), ironclaw_reborn_traces (contributor-side TraceCommons client extracted from the legacy monolith), and ironclaw_reborn_webui_ingress (host-owned WebUI serve lifecycle) only. Adding any other workspace crate here re-opens speculative public API access to internal Reborn types.",
    );
    assert_workspace_deps_exactly(
        &dependencies_all_kinds,
        "ironclaw_reborn_config",
        [],
        "ironclaw_reborn_config must remain a standalone boot contract crate with no IronClaw workspace dependencies of any dependency kind",
    );

    let runtime_dir = root.join("crates/ironclaw_reborn_cli/src/runtime");
    let mut cli_runtime_source = String::new();
    collect_runtime_rs(&runtime_dir, &mut cli_runtime_source);
    assert!(
        cli_runtime_source.contains("build_reborn_runtime"),
        "Reborn CLI should enter the assembled runtime through ironclaw_reborn_composition::build_reborn_runtime"
    );
    for forbidden in [
        "use ironclaw_host_runtime::",
        "use ironclaw_reborn::",
        "use ironclaw_threads::",
        "use ironclaw_turns::",
        "HostRuntimeServices",
        "build_default_planned_runtime",
    ] {
        assert!(
            !cli_runtime_source.contains(forbidden),
            "Reborn CLI runtime/ must not wire lower-level Reborn runtime pieces directly via `{forbidden}`; keep REPL as a UX shell over ironclaw_reborn_composition."
        );
    }
}

#[test]
fn reborn_host_runtime_services_do_not_expose_lower_substrate_handles() {
    let root = workspace_root();
    let lib = std::fs::read_to_string(root.join("crates/ironclaw_host_runtime/src/lib.rs"))
        .expect("host runtime lib.rs must be readable");
    let services =
        std::fs::read_to_string(root.join("crates/ironclaw_host_runtime/src/services.rs"))
            .expect("host runtime services.rs must be readable");
    let obligations =
        std::fs::read_to_string(root.join("crates/ironclaw_host_runtime/src/obligations.rs"))
            .expect("host runtime obligations.rs must be readable");
    let host_runtime_contract =
        std::fs::read_to_string(root.join("docs/reborn/contracts/host-runtime.md"))
            .expect("host runtime contract must be readable");
    let scripts = std::fs::read_to_string(root.join("crates/ironclaw_scripts/src/lib.rs"))
        .expect("script runtime lib.rs must be readable");
    let scripts_manifest = std::fs::read_to_string(root.join("crates/ironclaw_scripts/Cargo.toml"))
        .expect("script runtime Cargo.toml must be readable");
    let mcp = std::fs::read_to_string(root.join("crates/ironclaw_mcp/src/lib.rs"))
        .expect("MCP runtime lib.rs must be readable");
    let mcp_manifest = std::fs::read_to_string(root.join("crates/ironclaw_mcp/Cargo.toml"))
        .expect("MCP runtime Cargo.toml must be readable");

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

    let obligations_pub_use = extract_pub_use_block(&lib, "pub use obligations::{");
    let forbidden_obligation_exports = [
        "NetworkObligationPolicyStore",
        "RuntimeSecretInjectionStore",
        "RuntimeSecretInjectionStoreError",
    ];
    for export in forbidden_obligation_exports {
        assert!(
            !obligations_pub_use.contains(export),
            "ironclaw_host_runtime must not re-export lower substrate handoff store `{export}`; upper Reborn code should enter through HostRuntimeServices::host_runtime / Arc<dyn HostRuntime>"
        );
    }

    let forbidden_lib_accessors = [
        "pub use obligations::NetworkObligationPolicyStore",
        "pub use obligations::RuntimeSecretInjectionStore",
        "pub use obligations::RuntimeSecretInjectionStoreError",
        "pub use obligations::*",
        "pub fn with_secret_injection_store(",
        "pub fn with_network_policy_store(",
        "pub fn network(&self) -> &N",
        "pub fn secrets(&self) -> &S",
    ];
    for pattern in forbidden_lib_accessors {
        assert!(
            !lib.contains(pattern),
            "HostHttpEgressService must not expose lower substrate escape hatch `{pattern}`; keep raw network/secret/policy handoff wiring private to host-runtime composition"
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
        "pub fn secret_injection_store(",
        "pub fn network_policy_store(",
        "pub fn with_host_http_egress<N, SecretBackend>",
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

    let forbidden_obligation_accessors = [
        "pub struct RuntimeSecretInjectionStore",
        "pub enum RuntimeSecretInjectionStoreError",
        "pub struct NetworkObligationPolicyStore",
        "pub fn insert(",
        "pub fn take(",
        "pub fn discard_for_capability(",
        "pub fn with_handoff_stores(",
        "pub fn with_network_policy_store(",
        "pub fn with_secret_injection_store(",
        "pub fn network_policy_store(&self)",
        "pub fn secret_injection_store(&self)",
        "pub fn staged_network_policy_present_for_diagnostics(",
        "pub fn staged_secret_present_for_diagnostics(",
    ];
    for pattern in forbidden_obligation_accessors {
        assert!(
            !obligations.contains(pattern),
            "BuiltinObligationServices and lower handoff stores must not expose lower substrate escape hatch `{pattern}`; keep secret/network handoff stores private to host-runtime composition"
        );
    }

    for required_phrase in [
        "try_with_host_http_egress",
        "low-level host-runtime/test harness escape hatches",
        "upper Reborn crates must not use them",
    ] {
        assert!(
            host_runtime_contract.contains(required_phrase),
            "host-runtime contract should document `{required_phrase}` so raw handoff store seams are not mistaken for upper Reborn APIs"
        );
    }

    let forbidden_script_lane_surface = [
        "RuntimeAdapter",
        "pub struct ScriptRuntimeAdapter",
        "pub fn script_error_kind",
    ];
    for pattern in forbidden_script_lane_surface {
        assert!(
            !scripts.contains(pattern),
            "ironclaw_scripts must not expose host-runtime dispatcher composition surface `{pattern}`; compose script dispatch adapters inside ironclaw_host_runtime"
        );
    }

    assert!(
        !scripts_manifest.contains("ironclaw_dispatcher"),
        "ironclaw_scripts must not depend on ironclaw_dispatcher; script dispatcher adapters are host-runtime-private composition"
    );

    let forbidden_mcp_lane_surface = [
        "RuntimeAdapter",
        "pub struct McpRuntimeAdapter",
        "pub fn mcp_error_kind",
    ];
    for pattern in forbidden_mcp_lane_surface {
        assert!(
            !mcp.contains(pattern),
            "ironclaw_mcp must not expose host-runtime dispatcher composition surface `{pattern}`; compose MCP dispatch adapters inside ironclaw_host_runtime"
        );
    }
    assert!(
        !mcp_manifest.contains("ironclaw_dispatcher"),
        "ironclaw_mcp must not depend on ironclaw_dispatcher; MCP dispatcher adapters are host-runtime-private composition"
    );
}

fn extract_pub_use_block<'a>(contents: &'a str, start_marker: &str) -> &'a str {
    let Some(start) = contents.find(start_marker) else {
        return "";
    };
    let after_start = &contents[start..];
    let Some(end) = after_start.find("};") else {
        return after_start;
    };
    &after_start[..end]
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

    // The composition root — the only crate that should pull `ironclaw_reborn`
    // (and through it `ironclaw_llm`) for the assembled runtime — must mirror
    // the same feature-gated discipline. Both `ironclaw_reborn` (transitive)
    // and `ironclaw_llm` (direct) live behind a `root-llm-provider` feature
    // on the composition crate, so a default build of composition stays
    // substrate-only.
    let composition_manifest =
        std::fs::read_to_string(root.join("crates/ironclaw_reborn_composition/Cargo.toml"))
            .expect("Reborn composition manifest must be readable");
    assert!(
        composition_manifest.contains("root-llm-provider")
            && composition_manifest.contains("ironclaw_llm")
            && composition_manifest.contains("optional = true")
            && composition_manifest.contains("default-features = false"),
        "ironclaw_reborn_composition must gate `ironclaw_llm` behind the same `root-llm-provider` feature with `optional = true, default-features = false`"
    );
}

/// Lock the narrowed `ironclaw_reborn` public surface in place.
///
/// `ironclaw_reborn` previously exposed ~25 types as a wall of `pub use`
/// re-exports (capability resolvers, surface profile filters, milestone
/// scope/sink, model route policies, planned-driver factory helpers, the
/// loop-driver-host factory, etc.). Internal-trace audits found that **no
/// crate outside the reborn family ever named any of those items** and that
/// composition does not need them either — it imports via submodule paths
/// (`ironclaw_reborn::driver_registry::DriverRegistry`, etc.). The wall was
/// pure speculative public API.
///
/// This test pins the cleanup: `crates/ironclaw_reborn/src/lib.rs` must be a
/// directory of `pub mod` declarations and nothing else. A future contributor
/// who tries to re-add the convenience `pub use` block fails this test
/// alongside the boundary rule that forbids any non-composition crate from
/// taking a normal cargo dep on `ironclaw_reborn`.
#[test]
fn reborn_internal_crate_keeps_directory_of_modules_lib_rs() {
    let root = workspace_root();
    let lib = std::fs::read_to_string(root.join("crates/ironclaw_reborn/src/lib.rs"))
        .expect("ironclaw_reborn lib.rs must be readable");

    // The forbidden re-export prefixes correspond to the original noisy
    // wall. Anyone wanting these items must reach them through a `pub mod`
    // path or (preferably) consume them through `ironclaw_reborn_composition`.
    let forbidden_reexports = [
        "pub use ironclaw_loop_support::",
        "pub use loop_driver_host::",
        "pub use milestone_events::",
        "pub use model_gateway::",
        "pub use model_routes::",
        "pub use planned_driver::",
        "pub use planned_driver_factory::",
        "pub use text_loop_driver::",
        "pub use app_loop_family::",
    ];
    for forbidden in forbidden_reexports {
        assert!(
            !lib.contains(forbidden),
            "ironclaw_reborn/src/lib.rs must not re-export internal items via `{forbidden}`. \
             Reach them through the `pub mod` path or through ironclaw_reborn_composition. \
             See `reborn_internal_crate_keeps_directory_of_modules_lib_rs` for context."
        );
    }

    // The composition root is the sanctioned consumer of `ironclaw_reborn`'s
    // module paths. Confirm the run-state assembly is wired there (it would
    // otherwise have to live in the CLI or root app, which the dep rules
    // forbid).
    let composition_runtime = root.join("crates/ironclaw_reborn_composition/src/runtime.rs");
    let composition_local_dev_runtime =
        root.join("crates/ironclaw_reborn_composition/src/runtime/local_dev.rs");
    assert!(
        composition_runtime.exists(),
        "expected Reborn runtime assembly at {}",
        composition_runtime.display()
    );
    assert!(
        composition_local_dev_runtime.exists(),
        "expected local-dev runtime assembly at {}",
        composition_local_dev_runtime.display()
    );
    let composition_runtime_source = std::fs::read_to_string(&composition_runtime)
        .expect("composition runtime.rs must be readable");
    let composition_runtime_sources = format!(
        "{}\n{}",
        composition_runtime_source,
        std::fs::read_to_string(&composition_local_dev_runtime)
            .expect("composition runtime/local_dev.rs must be readable")
    );
    for required in [
        "pub async fn build_reborn_runtime",
        "pub struct RebornRuntime",
        "use ironclaw_reborn::runtime::",
        "build_default_planned_runtime",
        "DefaultPlannedRuntimeParts",
    ] {
        assert!(
            composition_runtime_source.contains(required),
            "composition runtime.rs missing `{required}` -- the runtime assembly slice \
             must live in `ironclaw_reborn_composition` so the CLI and other \
             ingress points can avoid importing `ironclaw_reborn` directly."
        );
    }
    assert!(
        composition_runtime_sources.contains("use ironclaw_loop_support::")
            && composition_runtime_sources.contains("LoopCapabilityPortFactory"),
        "composition runtime module set missing loop-support capability factory wiring -- \
         the host adapter assembly may live in a runtime submodule, but it must stay inside \
         `ironclaw_reborn_composition` rather than the CLI or other ingress points."
    );
}

/// Lock the boot-config TOML + provider-catalog layering for the
/// standalone `ironclaw-reborn` binary.
///
/// Three properties:
///
/// 1. `ironclaw_reborn_config` continues to expose the boot-time parser
///    (`RebornConfigFile`) and the file-path accessors
///    (`RebornHome::config_file_path` / `providers_file_path`). These are
///    the surface the CLI relies on to find both files without
///    hardcoding the paths itself, and they're what shell tooling /
///    operator runbooks pattern-match on.
///
/// 2. The provider catalog file lives at `<home>/providers.json` —
///    same filename as v1's `~/.ironclaw/providers.json` so operator
///    muscle memory transfers and the same JSON editor tooling
///    applies. The boot TOML lives at `<home>/config.toml`. Changing
///    either filename breaks all existing operator-side documentation.
///
/// 3. `RebornConfigFile` rejects inline secret material at parse time.
///    The unit test in `secrets_guard` covers the patterns; this
///    boundary test asserts that the rejection path is *wired through*
///    `RebornConfigFile::validate` (file-level grep). A regression
///    that bypasses the guard for the boot file fails here loudly
///    rather than silently round-tripping a secret through git.
#[test]
fn reborn_boot_config_file_layout_is_pinned() {
    let root = workspace_root();

    let config_lib = std::fs::read_to_string(root.join("crates/ironclaw_reborn_config/src/lib.rs"))
        .expect("reborn config lib.rs must be readable");
    for required_export in [
        "pub use config_file::",
        "RebornConfigFile",
        "REBORN_CONFIG_API_VERSION",
        "InlineSecretError",
    ] {
        assert!(
            config_lib.contains(required_export),
            "ironclaw_reborn_config/src/lib.rs must export `{required_export}`; \
             see reborn_boot_config_file_layout_is_pinned for context"
        );
    }

    let home_src = std::fs::read_to_string(root.join("crates/ironclaw_reborn_config/src/home.rs"))
        .expect("reborn config home.rs must be readable");
    for required_method in ["pub fn config_file_path", "pub fn providers_file_path"] {
        assert!(
            home_src.contains(required_method),
            "RebornHome must expose `{required_method}` so the CLI / composition can locate \
             the boot files without hardcoding paths; see \
             reborn_boot_config_file_layout_is_pinned"
        );
    }
    // File names — these match v1's `~/.ironclaw/providers.json` so the
    // same operator tooling / documentation applies.
    assert!(
        home_src.contains("\"config.toml\""),
        "boot config file name must be `config.toml`"
    );
    assert!(
        home_src.contains("\"providers.json\""),
        "provider catalog file name must be `providers.json` to match v1's filename for \
         operator-tooling compatibility"
    );

    // The boot TOML parser must wire the inline-secret guard. A
    // regression that bypasses it (e.g. a future contributor adds a
    // new section and forgets to call `reject_inline_secret`) would
    // silently allow pasted credentials through.
    let config_file_src =
        std::fs::read_to_string(root.join("crates/ironclaw_reborn_config/src/config_file.rs"))
            .expect("reborn config_file.rs must be readable");
    assert!(
        config_file_src.contains("reject_inline_secret"),
        "RebornConfigFile::validate must call `reject_inline_secret` on operator-pasteable \
         fields. See `docs/reborn/contracts/secrets.md` and epic #3036's `Pitfalls & \
         Landmines` section: \"Do not bake secret material into blueprints/config.\""
    );

    // Provider-catalog load-from-path must be reachable from
    // composition without forcing `ironclaw_reborn_config` to depend
    // on `ironclaw_llm` (which would violate _config's standalone
    // boundary). The composition crate is the legitimate consumer.
    let llm_catalog = root.join("crates/ironclaw_reborn_composition/src/llm_catalog.rs");
    assert!(
        llm_catalog.exists(),
        "composition must expose a catalog resolver at {} so the CLI can stitch \
         RebornConfigFile + providers.json into a RebornLlmConfig without itself \
         depending on ironclaw_llm",
        llm_catalog.display()
    );
    let llm_catalog_src = std::fs::read_to_string(&llm_catalog).expect("llm_catalog readable");
    for required in [
        "pub fn resolve_llm_selection_against_catalog",
        "pub fn resolve_against_registry",
        "ProviderRegistry::load_from_path",
    ] {
        assert!(
            llm_catalog_src.contains(required),
            "composition llm_catalog must expose `{required}` so the resolver path is \
             stable; see reborn_boot_config_file_layout_is_pinned"
        );
    }

    // `ironclaw_llm` must expose the path-overridable loader so the
    // catalog file location is selectable per-deployment (the
    // standalone Reborn binary points at $IRONCLAW_REBORN_HOME/providers.json,
    // not v1's ~/.ironclaw/providers.json).
    let llm_registry = std::fs::read_to_string(root.join("crates/ironclaw_llm/src/registry.rs"))
        .expect("ironclaw_llm registry.rs must be readable");
    assert!(
        llm_registry.contains("pub fn load_from_path"),
        "ironclaw_llm::ProviderRegistry must expose `load_from_path` so callers can \
         override the user-overlay catalog path; v1 hardcoded ~/.ironclaw/providers.json \
         and the Reborn standalone needs its own home."
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
fn wasm_sandbox_core_is_standalone_v1_parity_kernel() {
    let root = workspace_root().join("crates/ironclaw_wasm_sandbox_core");
    assert!(
        root.join("Cargo.toml").exists(),
        "shared WASM sandbox core should exist before ProductAdapters duplicate v1 sandbox setup"
    );
    assert!(
        root.join("CLAUDE.md").exists(),
        "shared WASM sandbox core needs local guardrails"
    );

    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");
    let package = packages
        .iter()
        .find(|package| package["name"] == "ironclaw_wasm_sandbox_core")
        .expect("ironclaw_wasm_sandbox_core must be a workspace package");
    let workspace_deps = workspace_dependency_names(package)
        .filter_map(|dependency| dependency["name"].as_str())
        .collect::<Vec<_>>();
    assert!(
        workspace_deps.is_empty(),
        "WASM sandbox core should stay independent of IronClaw domain crates; got {workspace_deps:?}"
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
    //   * serde_json          — validates temporary JSON-shim WIT payloads.
    //   * ironclaw_wasm_sandbox_core — shared v1-style minimal WASI sandbox kernel.
    //   * wasmtime            — component type and generated binding instantiation.
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
        "ironclaw_wasm_sandbox_core",
        "serde_json",
        "sha2",
        "subtle",
        "thiserror",
        "tokio",
        "tracing",
        "wasmtime",
    ];
    assert_eq!(
        deps, expected,
        "ironclaw_wasm_product_adapters should stay thin host glue; add runtime/workflow dependencies only when a call-site proves they are required"
    );
}

#[test]
fn wasm_product_adapter_runtime_uses_v1_style_minimal_wasi() {
    let root = workspace_root();
    let core = std::fs::read_to_string(root.join("crates/ironclaw_wasm_sandbox_core/src/lib.rs"))
        .expect("WASM sandbox core must be readable");
    let adapter_store =
        std::fs::read_to_string(root.join("crates/ironclaw_wasm_product_adapters/src/store.rs"))
            .expect("ProductAdapter WASM store must be readable");
    let adapter_runtime = std::fs::read_to_string(
        root.join("crates/ironclaw_wasm_product_adapters/src/component_runtime.rs"),
    )
    .expect("ProductAdapter WASM runtime must be readable");

    assert!(
        adapter_store.contains("SandboxStoreCore")
            && adapter_runtime.contains("add_minimal_wasi_to_linker"),
        "ProductAdapter components should use the shared v1-style WASM sandbox core instead of duplicating WASI setup"
    );
    assert!(
        core.contains("wasmtime_wasi::p2::add_to_linker_sync"),
        "shared sandbox core should register WASI p2 like v1 tools/channels"
    );
    assert!(
        core.contains("WasiCtxBuilder::new().build()"),
        "shared sandbox core should use the v1 minimal default: no env, args, preopens, or inherited network"
    );
    for forbidden in [
        "inherit_env",
        "inherit_stdio",
        "preopened_dir",
        "inherit_network",
        "allow_ip_name_lookup(true)",
        "socket_addr_check(|_, _| Box::pin(async { true }))",
    ] {
        assert!(
            !core.contains(forbidden)
                && !adapter_store.contains(forbidden)
                && !adapter_runtime.contains(forbidden),
            "ProductAdapter minimal WASI must not enable `{forbidden}`; HTTP egress stays host-mediated"
        );
    }
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

#[test]
fn reborn_product_api_crates_do_not_bind_http_ingress() {
    let forbidden = [
        ForbiddenUse {
            pattern: "tokio::net::TcpListener::bind",
            reason: "Reborn product/API crates must expose route descriptors, not bind listeners",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "std::net::TcpListener::bind",
            reason: "Reborn product/API crates must expose route descriptors, not bind listeners",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "TcpListener::bind",
            reason: "Reborn product/API crates must expose route descriptors, not bind listeners",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "axum::serve",
            reason: "Reborn product/API crates must not own server lifecycle",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "hyper::Server",
            reason: "Reborn product/API crates must not own server lifecycle",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "Server::bind",
            reason: "Reborn product/API crates must not own server lifecycle",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "axum_server::bind",
            reason: "Reborn product/API crates must not own server lifecycle",
            exempt: None,
        },
    ];

    let root = workspace_root();
    let reborn_product_api_src_roots = [
        "crates/ironclaw_reborn/src",
        "crates/ironclaw_reborn_cli/src",
        "crates/ironclaw_reborn_composition/src",
        "crates/ironclaw_reborn_config/src",
        "crates/ironclaw_reborn_event_store/src",
        "crates/ironclaw_reborn_api/src",
        "crates/ironclaw_reborn_openai_compat/src",
        "crates/ironclaw_product_adapters/src",
        "crates/ironclaw_product_adapter_registry/src",
        "crates/ironclaw_product_workflow/src",
        "crates/ironclaw_wasm_product_adapters/src",
        "crates/ironclaw_telegram_v2_adapter/src",
        "crates/ironclaw_slack_v2_adapter/src",
        "crates/ironclaw_outbound/src",
        "crates/ironclaw_conversations/src",
        "crates/ironclaw_turns/src",
        "crates/ironclaw_threads/src",
        "crates/ironclaw_loop_support/src",
        // WebChat v2 route surface: a Product/API crate that exposes
        // axum handler functions and `IngressRouteDescriptor`s but must
        // never bind sockets or call `axum::serve` itself — that is
        // host composition's job. Without this entry the contract fails
        // open for the new route crate.
        "crates/ironclaw_webui_v2/src",
    ];

    let mut violations = Vec::new();
    for relative_root in reborn_product_api_src_roots {
        let dir = root.join(relative_root);
        if !dir.exists() {
            continue;
        }
        collect_forbidden_uses(&dir, &root, &forbidden, &mut violations);
    }

    assert!(
        violations.is_empty(),
        "Reborn HTTP ingress must be host-owned; product/API crates may expose descriptors or route fragments but must not bind/serve listeners:\n{}",
        violations.join("\n")
    );
}

#[test]
fn reborn_product_auth_contract_stays_reborn_native() {
    let forbidden = [
        ForbiddenUse {
            pattern: "ironclaw::",
            reason: "Reborn product auth must not depend on the v1 root crate",
            exempt: Some(is_reborn_tracing_target_line),
        },
        ForbiddenUse {
            pattern: "src/extensions",
            reason: "v1 extension paths are inventory only, not Reborn auth implementation",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "src/channels/web",
            reason: "v1 web routes are inventory only, not Reborn auth implementation",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "ExtensionManager",
            reason: "Reborn product auth must not call through the v1 extension manager",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "PendingOAuth",
            reason: "Reborn product auth must not reuse v1 pending OAuth maps",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "PendingGate",
            reason: "Reborn product auth must not reuse v1 pending gate maps",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "SecretsStore",
            reason: "Reborn product auth must use opaque handles, not raw v1 secrets storage",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "get_decrypted",
            reason: "Reborn product auth must not retrieve raw secret material directly",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "auth-token",
            reason: "Reborn manual-token setup must not fall back to v1 chat token route names",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "auth_token",
            reason: "Reborn manual-token setup must not fall back to v1 chat token command paths",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "IncomingMessage",
            reason: "Reborn product auth must not capture manual tokens through chat transcripts",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "ChatMessage",
            reason: "Reborn product auth must not capture manual tokens through chat transcripts",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "secret_name",
            reason: "Reborn product auth must use scoped credential accounts and opaque handles, not raw v1 secret names",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "SecretName",
            reason: "Reborn product auth must use scoped credential accounts and opaque handles, not raw v1 secret names",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "reqwest",
            reason: "Reborn product auth must not own outbound HTTP transport",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "authorization_code: String",
            reason: "raw OAuth codes must be one-shot non-serializable provider inputs",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "pkce_verifier: String",
            reason: "raw PKCE verifiers must be one-shot non-serializable provider inputs",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "access_token: String",
            reason: "raw provider tokens must not enter product auth contract records",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "refresh_token: String",
            reason: "raw provider tokens must not enter product auth contract records",
            exempt: None,
        },
    ];

    let root = workspace_root();
    let manifest = std::fs::read_to_string(root.join("crates/ironclaw_auth/Cargo.toml"))
        .expect("ironclaw_auth manifest must be readable");
    assert!(
        !manifest.contains("reqwest"),
        "ironclaw_auth must not depend on reqwest directly; provider transport belongs behind Reborn-native composition"
    );

    let auth_src = root.join("crates/ironclaw_auth/src");
    assert!(
        auth_src.exists(),
        "Reborn product auth contract crate must have a src directory at {}",
        auth_src.display()
    );

    let mut violations = Vec::new();
    collect_forbidden_uses(&auth_src, &root, &forbidden, &mut violations);
    collect_forbidden_reborn_auth_file_uses(
        &root.join("crates/ironclaw_reborn_composition/src/auth.rs"),
        &root,
        &forbidden,
        &mut violations,
    );
    collect_forbidden_reborn_auth_path_uses(
        &root.join("crates/ironclaw_reborn_composition/src/product_auth_serve"),
        &root.join("crates/ironclaw_reborn_composition/src/product_auth_serve.rs"),
        &root,
        &forbidden,
        &mut violations,
    );

    assert!(
        violations.is_empty(),
        "Reborn product auth can be behavior-compatible with v1, but implementation and composition code paths must not mingle with v1 routes, v1 extension/secrets managers, raw provider transport, or raw secret records:\n{}",
        violations.join("\n")
    );
}

struct ForbiddenRuntimeNetworkUse {
    pattern: &'static str,
    reason: &'static str,
}

struct ForbiddenUse {
    pattern: &'static str,
    reason: &'static str,
    exempt: Option<fn(&str) -> bool>,
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

fn collect_forbidden_string_uses(
    dir: &std::path::Path,
    needle: &str,
    root: &std::path::Path,
    matches: &mut Vec<String>,
) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}"));
        let path = entry.path();
        if path.is_dir() {
            collect_forbidden_string_uses(&path, needle, root, matches);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        if contents.contains(needle) {
            matches.push(
                path.strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string(),
            );
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
            // Product auth is a Reborn contract/facade vocabulary. It may
            // describe behavior-compatible v1 inventory, but implementation
            // code must not reach into v1 routes, extension managers, secret
            // stores, runtimes, or channel-specific stacks.
            crate_name: "ironclaw_auth",
            forbidden: vec![
                "ironclaw",
                "ironclaw_approvals",
                "ironclaw_authorization",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_dispatcher",
                "ironclaw_engine",
                "ironclaw_event_projections",
                "ironclaw_events",
                "ironclaw_extensions",
                "ironclaw_filesystem",
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_loop_support",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_product_adapters",
                "ironclaw_product_adapter_registry",
                "ironclaw_product_workflow",
                "ironclaw_reborn",
                "ironclaw_reborn_cli",
                "ironclaw_reborn_composition",
                "ironclaw_reborn_config",
                "ironclaw_reborn_event_store",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_scripts",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_storage",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_turns",
                "ironclaw_wasm",
                "ironclaw_wasm_product_adapters",
            ],
        },
        BoundaryRule {
            // WebChat v2 route surface must only reach into Reborn through
            // the host-facing facade and the ingress vocabulary; anything
            // that lets a handler touch the dispatcher, runtime lane, run
            // state, or a storage backend directly would defeat the
            // single-facade discipline that this crate exists to enforce.
            crate_name: "ironclaw_webui_v2",
            forbidden: vec![
                "ironclaw",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_dispatcher",
                "ironclaw_engine",
                "ironclaw_event_projections",
                "ironclaw_events",
                "ironclaw_extensions",
                "ironclaw_filesystem",
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_loop_support",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                // Single-facade boundary: route handlers consume only the
                // `ironclaw_product_workflow` facade plus the ingress + error
                // vocabulary. Projection types are re-exported through the
                // facade crate so handlers never reach into the adapter
                // surface directly.
                "ironclaw_product_adapters",
                "ironclaw_reborn",
                "ironclaw_reborn_cli",
                "ironclaw_reborn_composition",
                "ironclaw_reborn_config",
                "ironclaw_reborn_event_store",
                "ironclaw_first_party_extensions",
                "ironclaw_first_party_extension_ports",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_scripts",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_storage",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_turns",
                "ironclaw_wasm",
                "ironclaw_wasm_product_adapters",
            ],
        },
        BoundaryRule {
            // OpenAI-compatible route surface is a Reborn product/API facade.
            // It may depend on host ingress vocabulary and ProductWorkflow
            // adapter contracts, but it must not revive v1 gateway/LLM proxy
            // paths or reach into runtime/composition services directly.
            crate_name: "ironclaw_reborn_openai_compat",
            forbidden: vec![
                "ironclaw",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_dispatcher",
                "ironclaw_engine",
                "ironclaw_event_projections",
                "ironclaw_event_streams",
                "ironclaw_events",
                "ironclaw_extensions",
                "ironclaw_filesystem",
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_loop_support",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_product_workflow",
                "ironclaw_reborn",
                "ironclaw_reborn_cli",
                "ironclaw_reborn_composition",
                "ironclaw_reborn_config",
                "ironclaw_reborn_event_store",
                "ironclaw_first_party_extensions",
                "ironclaw_first_party_extension_ports",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_scripts",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_storage",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_turns",
                "ironclaw_wasm",
                "ironclaw_wasm_product_adapters",
                "ironclaw_webui_v2",
            ],
        },
        BoundaryRule {
            // Durable storage for OpenAI-compatible public refs sits behind
            // the OpenAiCompatRefStore port. It may use the universal
            // filesystem backend and the OpenAI-compatible contract crate, but
            // must not grow route handling, ProductWorkflow orchestration, or
            // runtime/composition reach-through.
            crate_name: "ironclaw_reborn_openai_compat_storage",
            forbidden: vec![
                "ironclaw",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_dispatcher",
                "ironclaw_engine",
                "ironclaw_event_projections",
                "ironclaw_event_streams",
                "ironclaw_events",
                "ironclaw_extensions",
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_loop_support",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_product_workflow",
                "ironclaw_reborn",
                "ironclaw_reborn_cli",
                "ironclaw_reborn_composition",
                "ironclaw_reborn_config",
                "ironclaw_reborn_event_store",
                "ironclaw_first_party_extensions",
                "ironclaw_first_party_extension_ports",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_scripts",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_storage",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_turns",
                "ironclaw_wasm",
                "ironclaw_wasm_product_adapters",
                "ironclaw_webui_v2",
            ],
        },
        BoundaryRule {
            // Registry projects ProductAdapter host-api sections from the single
            // Extension Manifest v2 over extension-owned installation and activation
            // state. Runtime/dispatcher/engine crates would invert ownership, secrets
            // crates could expose raw material instead of opaque handles, and v1
            // WASM/channel crates would bypass the Reborn registry boundary.
            crate_name: "ironclaw_product_adapter_registry",
            forbidden: vec![
                "ironclaw",
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_dispatcher",
                "ironclaw_engine",
                "ironclaw_events",
                "ironclaw_filesystem",
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_product_workflow",
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
                "ironclaw_wasm",
                "ironclaw_wasm_product_adapters",
            ],
        },
        BoundaryRule {
            // First-party extensions are userland implementation packages.
            // They may consume scoped storage and pure safety helpers, but
            // must not receive ambient runtime authority or loop-facing
            // runtime handles.
            crate_name: "ironclaw_first_party_extensions",
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
                "ironclaw_first_party_extension_ports",
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_loop_support",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_product_adapters",
                "ironclaw_product_workflow",
                "ironclaw_product_adapter_registry",
                "ironclaw_reborn",
                "ironclaw_reborn_composition",
                "ironclaw_reborn_config",
                "ironclaw_reborn_event_store",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_runtime_policy",
                "ironclaw_scripts",
                "ironclaw_secrets",
                "ironclaw_threads",
                "ironclaw_tui",
                "ironclaw_wasm",
                "ironclaw_wasm_product_adapters",
            ],
        },
        BoundaryRule {
            // First-party extension ports are adapter glue above concrete
            // userland implementations. They may depend on loop/turn-facing
            // contracts, but must not reach into host runtime authority or
            // product composition.
            crate_name: "ironclaw_first_party_extension_ports",
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
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_product_adapters",
                "ironclaw_product_workflow",
                "ironclaw_product_adapter_registry",
                "ironclaw_reborn",
                "ironclaw_reborn_composition",
                "ironclaw_reborn_config",
                "ironclaw_reborn_event_store",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_scripts",
                "ironclaw_secrets",
                "ironclaw_tui",
                "ironclaw_wasm",
                "ironclaw_wasm_product_adapters",
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
            // The standalone CLI reaches runtime and provider/admin UX through
            // `ironclaw_reborn_composition` facades. Adding any of the
            // forbidden deps here re-opens "speculative public API" access to
            // internal Reborn types (turn coordinator, session thread service,
            // loop drivers, LLM registry/auth internals, etc.) and
            // re-introduces the narrow-surface regression this rule exists to
            // prevent.
            crate_name: "ironclaw_reborn_cli",
            forbidden: vec![
                "ironclaw",
                "ironclaw_engine",
                "ironclaw_gateway",
                "ironclaw_llm",
                "ironclaw_loop_support",
                "ironclaw_reborn",
                "ironclaw_skills",
                "ironclaw_threads",
                "ironclaw_tui",
                "ironclaw_turns",
            ],
        },
        BoundaryRule {
            // Host-owned WebUI ingress: binds the TCP listener and runs
            // the axum serve loop for the composed v2 Router. Deliberately
            // narrow: it must not pull product/API internals, lower
            // substrate handles, or v1 surface code into the binary path.
            // Reaches Reborn through ironclaw_reborn_composition's facade
            // only (Router + WebuiAuthenticator trait + WebuiServeConfig).
            crate_name: "ironclaw_reborn_webui_ingress",
            forbidden: vec![
                "ironclaw",
                "ironclaw_authorization",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_dispatcher",
                "ironclaw_engine",
                "ironclaw_events",
                "ironclaw_extensions",
                "ironclaw_filesystem",
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_loop_support",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_product_adapters",
                "ironclaw_product_adapter_registry",
                "ironclaw_product_workflow",
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
                "ironclaw_wasm_product_adapters",
                "ironclaw_webui_v2",
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
                // ironclaw_filesystem is permitted: FilesystemResourceGovernorStore
                // routes the resource-governor snapshot through ScopedFilesystem
                // under the universal-fs-dispatch rework (plan
                // 2026-05-14-universal-fs-dispatch).
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
                "ironclaw_first_party_extensions",
                "ironclaw_first_party_extension_ports",
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
            // Product-facing projection reducers consume typed domain events.
            // `ironclaw_turns` is intentionally allowed here for
            // `TurnLifecycleEvent`-derived read models such as pending gates;
            // projection crates must still stay below product/runtime
            // composition and must not import root `src/` or legacy engine
            // pending-gate types.
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
            crate_name: "ironclaw_event_streams",
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
                "ironclaw_processes",
                "ironclaw_product_adapter_registry",
                "ironclaw_product_adapters",
                "ironclaw_product_workflow",
                "ironclaw_product_workflow_storage",
                "ironclaw_reborn_event_store",
                "ironclaw_reborn",
                "ironclaw_reborn_cli",
                "ironclaw_reborn_composition",
                "ironclaw_reborn_config",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_scripts",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_telegram_v2_adapter",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_wasm",
                "ironclaw_wasm_product_adapters",
                "ironclaw_webui_v2",
            ],
        },
        BoundaryRule {
            // Concrete Slack protocol adapter owns only Slack payload
            // normalization/rendering over the ProductAdapter DTO surface.
            // Host auth verification, credential resolution, delivery fanout,
            // workflow admission, and runtime/network authority stay outside
            // the adapter crate.
            crate_name: "ironclaw_slack_v2_adapter",
            forbidden: vec![
                "ironclaw",
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_auth",
                "ironclaw_capabilities",
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
                "ironclaw_product_adapter_registry",
                "ironclaw_product_workflow",
                "ironclaw_product_workflow_storage",
                "ironclaw_reborn",
                "ironclaw_reborn_cli",
                "ironclaw_reborn_composition",
                "ironclaw_reborn_config",
                "ironclaw_reborn_event_store",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_scripts",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_telegram_v2_adapter",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_wasm",
                "ironclaw_wasm_product_adapters",
                "ironclaw_webui_v2",
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
                // ironclaw_filesystem is permitted: FilesystemOutboundStateStore
                // routes outbound persistence through ScopedFilesystem under
                // the universal-fs-dispatch rework (plan
                // 2026-05-14-universal-fs-dispatch).
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
            // Trigger core owns source evaluation and trigger-domain state.
            // Durable storage, poller lifecycle, capability registration,
            // product adapters, and outbound delivery are wired by later
            // owners, not by reaching upward from this crate.
            crate_name: "ironclaw_triggers",
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
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_product_adapter_registry",
                "ironclaw_product_adapters",
                "ironclaw_product_workflow",
                "ironclaw_product_workflow_storage",
                "ironclaw_reborn",
                "ironclaw_reborn_cli",
                "ironclaw_reborn_composition",
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
                "ironclaw_wasm",
                "ironclaw_wasm_product_adapters",
                "ironclaw_webui_v2",
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
            // ironclaw_filesystem is permitted: FilesystemEventLog routes the
            // durable log through the universal RootFilesystem dispatch
            // fabric. See `2026-05-14-universal-fs-dispatch.md`.
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
            crate_name: "ironclaw_secrets",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_dispatcher",
                "ironclaw_events",
                "ironclaw_extensions",
                // ironclaw_filesystem is permitted: FilesystemSecretStore /
                // FilesystemCredentialBroker route secret + credential
                // persistence through ScopedFilesystem under the
                // universal-fs-dispatch rework (plan
                // 2026-05-14-universal-fs-dispatch).
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
                // ironclaw_filesystem is permitted: FilesystemSessionThreadService
                // routes thread/transcript persistence through ScopedFilesystem
                // under the universal-fs-dispatch rework (plan
                // 2026-05-14-universal-fs-dispatch).
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_run_state",
                // ironclaw_safety is permitted: thread/transcript storage
                // validates provider-originated replay metadata before it can
                // be persisted or exposed back to a model-visible context.
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
                // ironclaw_filesystem is permitted: FilesystemTurnStateStore
                // routes turn-coordination persistence through ScopedFilesystem
                // under the universal-fs-dispatch rework (plan
                // 2026-05-14-universal-fs-dispatch).
                "ironclaw_hooks",
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
        // The hooks framework depends on `ironclaw_turns` and host primitives
        // but must not pull in runtime adapters or dispatcher concretions.
        // This keeps the contract surface narrow and prevents the framework
        // from acquiring authority it should not have.
        BoundaryRule {
            crate_name: "ironclaw_hooks",
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
                "ironclaw_reborn",
                "ironclaw_run_state",
                "ironclaw_scripts",
                "ironclaw_secrets",
                "ironclaw_wasm",
            ],
        },
        // The agent-loop framework crate owns reusable loop mechanics
        // (executor, strategies, families, state) and depends upward on neutral
        // contracts in `ironclaw_turns`. It must not import host runtime crates,
        // product adapters, dispatcher, capability host, filesystem, network,
        // secrets, DB backends, or the loop-support adapter layer — those all
        // sit above agent_loop in the stack and would create an inversion.
        BoundaryRule {
            crate_name: "ironclaw_agent_loop",
            forbidden: vec![
                "ironclaw",
                "ironclaw_approvals",
                "ironclaw_auth",
                "ironclaw_authorization",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_dispatcher",
                "ironclaw_engine",
                "ironclaw_event_projections",
                "ironclaw_event_streams",
                "ironclaw_extensions",
                "ironclaw_filesystem",
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_loop_support",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_product_adapter_registry",
                "ironclaw_product_adapters",
                "ironclaw_product_workflow",
                "ironclaw_reborn",
                "ironclaw_reborn_cli",
                "ironclaw_reborn_composition",
                "ironclaw_reborn_config",
                "ironclaw_reborn_event_store",
                "ironclaw_reborn_traces",
                "ironclaw_reborn_webui_ingress",
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
        // `reborn_boundary_rules_active_crates_are_workspace_members` covers
        // present-on-disk crates that are missing from `cargo metadata`.
        return;
    };
    for forbidden in forbidden {
        assert!(
            !actual.iter().any(|dependency| dependency == forbidden),
            "{crate_name} must not have a normal dependency on {forbidden}; actual normal ironclaw deps: {actual:?}"
        );
    }
}

/// Recursively concatenate every `.rs` file under `dir` into `out`,
/// descending into subdirectories. Matches the recursion pattern used by
/// `collect_forbidden_*` walkers above so future boundary checks over
/// `runtime/` can reuse the same helper. Used by
/// `reborn_cli_binary_crate_stays_separate_from_v1_root` to scan the
/// entire `runtime/` module tree for forbidden imports.
fn collect_runtime_rs(dir: &std::path::Path, out: &mut String) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|err| {
        panic!(
            "Reborn CLI runtime directory must be readable at {}: {err}",
            dir.display()
        )
    }) {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_runtime_rs(&path, out);
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        let content = std::fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!(
                "Reborn CLI runtime file {} unreadable: {err}",
                path.display()
            )
        });
        out.push_str(&content);
        out.push('\n');
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

fn collect_forbidden_uses(
    dir: &std::path::Path,
    root: &std::path::Path,
    forbidden: &[ForbiddenUse],
    violations: &mut Vec<String>,
) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| panic!("failed to read dir entry: {error}"));
        let path = entry.path();
        if path.is_dir() {
            collect_forbidden_uses(&path, root, forbidden, violations);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        for (line_number, line) in contents.lines().enumerate() {
            for rule in forbidden {
                if rule.exempt.is_some_and(|exempt| exempt(line)) {
                    continue;
                }
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

fn collect_forbidden_reborn_auth_path_uses(
    module_dir: &std::path::Path,
    legacy_file: &std::path::Path,
    root: &std::path::Path,
    forbidden: &[ForbiddenUse],
    violations: &mut Vec<String>,
) {
    if module_dir.is_dir() {
        collect_forbidden_uses(module_dir, root, forbidden, violations);
        return;
    }
    collect_forbidden_reborn_auth_file_uses(legacy_file, root, forbidden, violations);
}

fn collect_forbidden_reborn_auth_file_uses(
    path: &std::path::Path,
    root: &std::path::Path,
    forbidden: &[ForbiddenUse],
    violations: &mut Vec<String>,
) {
    let message = format!(
        "failed to read Reborn product-auth boundary file {}",
        path.display()
    );
    let contents = std::fs::read_to_string(path).expect(&message);
    for (line_number, line) in contents.lines().enumerate() {
        for rule in forbidden {
            if rule.exempt.is_some_and(|exempt| exempt(line)) {
                continue;
            }
            if !line.contains(rule.pattern) {
                continue;
            }
            violations.push(format!(
                "{}:{} contains forbidden product-auth implementation pattern `{}`: {}",
                path.strip_prefix(root).unwrap_or(path).display(),
                line_number + 1,
                rule.pattern,
                rule.reason
            ));
        }
    }
}

fn is_reborn_tracing_target_line(line: &str) -> bool {
    line.contains("target: \"ironclaw::reborn::") || line.contains("target = \"ironclaw::reborn::")
}

#[test]
fn collect_forbidden_reborn_auth_file_uses_detects_violation() {
    let root = std::env::temp_dir().join(format!(
        "ironclaw-reborn-auth-boundary-test-{}",
        std::process::id()
    ));
    let src = root.join("crates/ironclaw_reborn_composition/src");
    std::fs::create_dir_all(&src).expect("test source directory must be created");
    let auth_rs = src.join("auth.rs");
    std::fs::write(&auth_rs, "fn forbidden() { let _ = \"reqwest\"; }\n")
        .expect("test auth.rs must be written");

    let mut violations = Vec::new();
    collect_forbidden_reborn_auth_file_uses(
        &auth_rs,
        &root,
        &[ForbiddenUse {
            pattern: "reqwest",
            reason: "provider transport must stay outside product auth composition",
            exempt: None,
        }],
        &mut violations,
    );

    std::fs::remove_dir_all(&root).expect("test source directory must be removed");

    assert_eq!(violations.len(), 1);
    assert!(
        violations[0].contains("crates/ironclaw_reborn_composition/src/auth.rs"),
        "violation should report the relative auth.rs path: {:?}",
        violations
    );
    assert!(
        violations[0].contains("provider transport must stay outside product auth composition"),
        "violation should report the forbidden-use reason: {:?}",
        violations
    );
}

#[test]
fn collect_forbidden_reborn_auth_file_uses_allows_reborn_tracing_targets() {
    let root = std::env::temp_dir().join(format!(
        "ironclaw-reborn-auth-boundary-tracing-test-{}",
        std::process::id()
    ));
    let src = root.join("crates/ironclaw_reborn_composition/src");
    std::fs::create_dir_all(&src).expect("test source directory must be created");
    let auth_rs = src.join("auth.rs");
    std::fs::write(
        &auth_rs,
        "fn allowed() { tracing::warn!(target: \"ironclaw::reborn::product_auth::oauth\"); }\n",
    )
    .expect("test auth.rs must be written");

    let mut violations = Vec::new();
    collect_forbidden_reborn_auth_file_uses(
        &auth_rs,
        &root,
        &[ForbiddenUse {
            pattern: "ironclaw::",
            reason: "Reborn product auth must not depend on the v1 root crate",
            exempt: Some(is_reborn_tracing_target_line),
        }],
        &mut violations,
    );

    std::fs::remove_dir_all(&root).expect("test source directory must be removed");

    assert!(
        violations.is_empty(),
        "Reborn tracing targets are log namespaces, not v1 root crate references: {:?}",
        violations
    );
}

#[test]
fn collect_forbidden_uses_allows_reborn_tracing_targets() {
    let root = std::env::temp_dir().join(format!(
        "ironclaw-reborn-auth-boundary-dir-tracing-test-{}",
        std::process::id()
    ));
    let src = root.join("crates/ironclaw_reborn_composition/src/product_auth_serve");
    std::fs::create_dir_all(&src).expect("test source directory must be created");
    let mod_rs = src.join("mod.rs");
    std::fs::write(
        &mod_rs,
        "fn allowed() { tracing::warn!(target: \"ironclaw::reborn::product_auth::oauth\"); }\n",
    )
    .expect("test mod.rs must be written");

    let mut violations = Vec::new();
    collect_forbidden_uses(
        &src,
        &root,
        &[ForbiddenUse {
            pattern: "ironclaw::",
            reason: "Reborn product auth must not depend on the v1 root crate",
            exempt: Some(is_reborn_tracing_target_line),
        }],
        &mut violations,
    );

    std::fs::remove_dir_all(&root).expect("test source directory must be removed");

    assert!(
        violations.is_empty(),
        "Directory scanner should treat Reborn tracing targets as log namespaces: {:?}",
        violations
    );
}

#[test]
fn collect_forbidden_uses_detects_violation() {
    let root = std::env::temp_dir().join(format!(
        "ironclaw-forbidden-use-dir-test-{}",
        std::process::id()
    ));
    let src = root.join("crates/example/src");
    std::fs::create_dir_all(&src).expect("test source directory must be created");
    let mod_rs = src.join("mod.rs");
    std::fs::write(&mod_rs, "fn forbidden() { let _ = \"reqwest\"; }\n")
        .expect("test mod.rs must be written");

    let mut violations = Vec::new();
    collect_forbidden_uses(
        &src,
        &root,
        &[ForbiddenUse {
            pattern: "reqwest",
            reason: "provider transport must stay outside product auth composition",
            exempt: None,
        }],
        &mut violations,
    );

    std::fs::remove_dir_all(&root).expect("test source directory must be removed");

    assert_eq!(violations.len(), 1);
    assert!(
        violations[0].contains("crates/example/src/mod.rs"),
        "violation should report the relative mod.rs path: {:?}",
        violations
    );
    assert!(
        violations[0].contains("provider transport must stay outside product auth composition"),
        "violation should report the forbidden-use reason: {:?}",
        violations
    );
}
