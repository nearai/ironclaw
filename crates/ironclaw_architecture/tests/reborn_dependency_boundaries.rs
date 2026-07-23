// arch-exempt: large_file, crate layer boundary gate stays with existing architecture suite, plan #5852
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
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
        let manifest = if rule.crate_name == "ironclaw" {
            root.join("crates/ironclaw_reborn_cli/Cargo.toml")
        } else {
            root.join("crates").join(rule.crate_name).join("Cargo.toml")
        };
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
fn reborn_workspace_crates_declare_layers_and_follow_layer_matrix() {
    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");

    let mut layers = BTreeMap::new();
    let mut invalid_workspace_package_names = Vec::new();
    let mut missing_metadata = Vec::new();
    let mut invalid_layers = Vec::new();
    for package in packages {
        let Some(name) = package["name"].as_str() else {
            continue;
        };
        if !is_ironclaw_workspace_package(name) {
            invalid_workspace_package_names.push(name.to_string());
            continue;
        }

        let manifest_path = package["manifest_path"]
            .as_str()
            .unwrap_or("<unknown manifest>");
        let Some(layer) = package_layer(package) else {
            missing_metadata.push(format!("{name} at {manifest_path}"));
            continue;
        };
        if !IRONCLAW_CRATE_LAYERS.contains(&layer) {
            invalid_layers.push(format!("{name} at {manifest_path} declares `{layer}`"));
            continue;
        }
        layers.insert(name.to_string(), layer.to_string());
    }

    assert!(
        invalid_workspace_package_names.is_empty(),
        "Workspace packages must follow the IronClaw naming convention \
         (`ironclaw` or `ironclaw_*`). Invalid packages:\n{}",
        invalid_workspace_package_names.join("\n")
    );
    assert!(
        missing_metadata.is_empty(),
        "Workspace packages must declare [package.metadata.ironclaw] layer = \"...\":\n{}",
        missing_metadata.join("\n")
    );
    assert!(
        invalid_layers.is_empty(),
        "Workspace packages declare unknown IronClaw layers; valid layers are \
         {IRONCLAW_CRATE_LAYERS:?}:\n{}",
        invalid_layers.join("\n")
    );

    let mut used_exceptions = BTreeSet::new();
    let mut violations = Vec::new();
    for package in packages {
        let Some(crate_name) = package["name"].as_str() else {
            continue;
        };
        let Some(crate_layer) = layers.get(crate_name) else {
            continue;
        };
        for dependency in
            workspace_dependency_names(package).filter(|dep| is_normal_dependency(dep))
        {
            let Some(dependency_name) = dependency["name"].as_str() else {
                continue;
            };
            let Some(dependency_layer) = layers.get(dependency_name) else {
                continue;
            };
            if !layer_allows_dependency(crate_layer, dependency_layer) {
                if let Some(exception) = layer_matrix_exception(crate_name, dependency_name) {
                    used_exceptions.insert((exception.crate_name, exception.dependency_name));
                    continue;
                }
                violations.push(format!(
                    "{crate_name} ({crate_layer}) must not depend on \
                     {dependency_name} ({dependency_layer})"
                ));
                continue;
            }
            if crate_name == "ironclaw_agent_loop" && *dependency_layer != "contracts" {
                if let Some(exception) = layer_matrix_exception(crate_name, dependency_name) {
                    used_exceptions.insert((exception.crate_name, exception.dependency_name));
                    continue;
                }
                violations.push(format!(
                    "ironclaw_agent_loop userland rule allows only contracts-layer normal \
                     dependencies, but it depends on {dependency_name} ({dependency_layer})"
                ));
            }
        }

        for dependency in
            workspace_dependency_names(package).filter(|dep| is_normal_dependency(dep))
        {
            let Some(dependency_name) = dependency["name"].as_str() else {
                continue;
            };
            let Some(dependency_layer) = layers.get(dependency_name) else {
                continue;
            };
            if dependency_layer == "legacy" && crate_name != "ironclaw_legacy" {
                if let Some(exception) = layer_matrix_exception(crate_name, dependency_name) {
                    used_exceptions.insert((exception.crate_name, exception.dependency_name));
                    continue;
                }
                violations.push(format!(
                    "{crate_name} ({crate_layer}) must not depend on legacy crate \
                     {dependency_name} via a normal dependency"
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "IronClaw crate layer matrix violations:\n{}",
        violations.join("\n")
    );

    let stale_exceptions = LAYER_MATRIX_EXCEPTIONS
        .iter()
        .filter(|exception| {
            !used_exceptions.contains(&(exception.crate_name, exception.dependency_name))
        })
        .map(|exception| {
            format!(
                "{} -> {} from {} should be removed in {}: {}",
                exception.crate_name,
                exception.dependency_name,
                exception.introduced,
                exception.removes_in,
                exception.reason
            )
        })
        .collect::<Vec<_>>();

    assert!(
        stale_exceptions.is_empty(),
        "Stale IronClaw crate layer matrix exceptions:\n{}",
        stale_exceptions.join("\n")
    );
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

    // Provider-neutral memory contract: among internal ironclaw crates it may
    // depend ONLY on `ironclaw_host_api`. Enforced as an allowlist (forbid every
    // other workspace ironclaw crate) so future deps — e.g. `ironclaw_turns`,
    // `ironclaw_product`, `ironclaw_runner` — cannot silently slip past a
    // blocklist that only names today's offenders.
    let memory_contract_allowed = ["ironclaw_memory", "ironclaw_host_api"];
    assert_no_normal_workspace_deps(
        &dependencies,
        "ironclaw_memory",
        workspace_ironclaw_crates(&dependencies)
            .into_iter()
            .filter(|name| !memory_contract_allowed.contains(name))
            .collect::<Vec<_>>(),
    );
    // Native memory provider: only the contract + the host/filesystem substrate it
    // is built on, among internal ironclaw crates.
    let memory_native_allowed = [
        "ironclaw_memory_native",
        "ironclaw_host_api",
        "ironclaw_filesystem",
        "ironclaw_memory",
        "ironclaw_prompt_envelope",
        "ironclaw_safety",
    ];
    assert_no_normal_workspace_deps(
        &dependencies,
        "ironclaw_memory_native",
        workspace_ironclaw_crates(&dependencies)
            .into_iter()
            .filter(|name| !memory_native_allowed.contains(name))
            .collect::<Vec<_>>(),
    );

    // Canonical Reborn identity layer: it maps external identities to a stable
    // `UserId` at the bottom of the stack, so among internal ironclaw crates it
    // may depend ONLY on `ironclaw_host_api` (identity/scope newtypes) and
    // `ironclaw_filesystem` (the durable substrate it persists behind). Enforced
    // as an allowlist so it can never reach UPSTREAM (into
    // `ironclaw_reborn_composition` / `ironclaw_product`) or onto the v1
    // legacy enclave — the "never reach upstream" property the crate guarantees.
    let reborn_identity_allowed = [
        "ironclaw_reborn_identity",
        "ironclaw_host_api",
        "ironclaw_filesystem",
    ];
    assert_no_normal_workspace_deps(
        &dependencies,
        "ironclaw_reborn_identity",
        workspace_ironclaw_crates(&dependencies)
            .into_iter()
            .filter(|name| !reborn_identity_allowed.contains(name))
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
        "crates/ironclaw_product/src",
        "crates/ironclaw_webui/src",
        "crates/ironclaw_telegram_extension/src",
        "crates/ironclaw_slack_extension/src",
        "crates/ironclaw_telegram_v2_adapter/src",
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
        manifest.contains("name = \"ironclaw\""),
        "Reborn CLI crate package name should be ironclaw"
    );
    assert!(
        manifest.contains("[[bin]]") && manifest.contains("name = \"ironclaw\""),
        "Reborn CLI crate must declare the canonical ironclaw binary explicitly"
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
        "ironclaw",
        [
            "ironclaw_extension_host",
            "ironclaw_first_party_extensions",
            "ironclaw_reborn_composition",
            "ironclaw_reborn_config",
            "ironclaw_reborn_traces",
            "ironclaw_webui",
            "ironclaw_slack_extension",
            "ironclaw_telegram_extension",
        ],
        "ironclaw should enter Reborn through ironclaw_reborn_composition (assembled-runtime and provider-admin facade), ironclaw_reborn_config (boot-config contract), ironclaw_reborn_traces (contributor-side TraceCommons client extracted from the legacy monolith), and ironclaw_webui (host-owned WebUI serve lifecycle) — plus ironclaw_extension_host (the NativeExtensionFactory contract) and concrete extension crates for the binary-assembled native factory registry (DEL-7: only the binary and tests may link concrete extension crates). Adding any other workspace crate here re-opens speculative public API access to internal Reborn types.",
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
        "use ironclaw_runner::",
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
fn reborn_runner_llm_wiring_is_isolated() {
    let root = workspace_root();

    let reborn_gateway = root.join("crates/ironclaw_runner/src/model_gateway.rs");
    assert!(
        reborn_gateway.exists(),
        "expected Reborn LLM gateway wiring at {}",
        reborn_gateway.display()
    );
    let reborn_gateway_source = std::fs::read_to_string(&reborn_gateway)
        .expect("Reborn model gateway source must be readable");
    assert!(
        reborn_gateway_source.contains("LlmProviderModelGateway"),
        "Reborn LLM gateway wiring should expose LlmProviderModelGateway from crates/ironclaw_runner"
    );

    // Reborn crates may reuse the extracted LLM crate, but never on its default
    // terms: `ironclaw_llm`'s defaults pull in extra provider/backend features
    // (e.g. `bedrock`) Reborn doesn't need. `default-features = false` on every
    // edge is the durable invariant, keeping the Reborn stack's dependency
    // footprint minimal and explicit.
    for manifest_path in [
        "crates/ironclaw_runner/Cargo.toml",
        "crates/ironclaw_reborn_composition/Cargo.toml",
    ] {
        let manifest = std::fs::read_to_string(root.join(manifest_path))
            .unwrap_or_else(|_| panic!("{manifest_path} must be readable"));
        let llm_dep = manifest
            .lines()
            .find(|line| line.trim_start().starts_with("ironclaw_llm = "))
            .unwrap_or_else(|| panic!("{manifest_path} must depend on ironclaw_llm"));
        assert!(
            llm_dep.contains("default-features = false"),
            "{manifest_path} must depend on `ironclaw_llm` with `default-features = false`, \
             so the Reborn stack never enables the root app's default postgres/libsql/tui \
             feature set: {llm_dep}"
        );
    }
}

#[test]
fn provider_tool_names_stay_at_model_protocol_boundaries() {
    let root = workspace_root();
    let mut uses = Vec::new();
    collect_provider_tool_name_boundary_uses(&root.join("crates"), &root, &mut uses);

    let allowed = BTreeSet::from([
        // Type definition and provider-wire validation.
        "crates/ironclaw_host_api/src/ids.rs",
        "crates/ironclaw_safety/src/lib.rs",
        "crates/ironclaw_safety/src/provider_validation.rs",
        // Host loop/run/thread protocol structs that preserve exact model
        // provider names for tool-result roundtrips and historical replay.
        // The provider-tool-call DTOs live in the `capability` submodule after
        // the `host.rs` -> `host/` decomposition.
        "crates/ironclaw_turns/src/run_profile/host/capability.rs",
        "crates/ironclaw_threads/src/tool_result_reference.rs",
        // Loop support owns capability-id <-> provider-name surface snapshots,
        // synthetic provider tools, provider-call registration, and replay refs.
        "crates/ironclaw_loop_host/src/lib.rs",
        "crates/ironclaw_loop_host/src/capability_info.rs",
        "crates/ironclaw_loop_host/src/capability_port.rs",
        "crates/ironclaw_loop_host/src/capability_port/provider_validation.rs",
        "crates/ironclaw_loop_host/src/capability_port/surface_snapshot.rs",
        "crates/ironclaw_loop_host/src/subagent_spawn_port.rs",
        // The model gateway is the LLM wire boundary. Executor helpers may
        // rebuild provider calls only from stored replay metadata.
        "crates/ironclaw_runner/src/model_gateway.rs",
        "crates/ironclaw_agent_loop/src/executor/capability_helpers.rs",
        // Progressive tool disclosure is itself a model-protocol boundary: the
        // catalog/selector and the bridging decorator map provider tool names
        // (advertised, deferred, and synthetic bridge names) to/from capability
        // ids and rebuild provider calls for the resolved target.
        "crates/ironclaw_runner/src/tool_disclosure.rs",
        "crates/ironclaw_runner/src/tool_disclosure_port.rs",
        // Composition-local protocol surfaces that reconstruct provider-shaped
        // output or local-dev provider tools.
        "crates/ironclaw_reborn_composition/src/llm_admin/openai_compat_serve.rs",
        "crates/ironclaw_reborn_composition/src/runtime/local_dev/external_tool_capability.rs",
        "crates/ironclaw_reborn_composition/src/runtime/local_dev/synthetic_capability.rs",
        "crates/ironclaw_reborn_composition/src/observability/trace_capture.rs",
    ]);
    let violations = uses
        .into_iter()
        .filter(|use_site| !allowed.contains(use_site.path.as_str()))
        .map(|use_site| {
            format!(
                "{}:{} contains `{}`",
                use_site.path, use_site.line_number, use_site.pattern
            )
        })
        .collect::<Vec<_>>();

    assert!(
        violations.is_empty(),
        "ProviderToolName/provider_tool_name is provider-protocol identity, not canonical \
         capability identity. Product, frontend, workflow, and ordinary routing code must use \
         CapabilityId and convert only at model-provider/replay boundaries. Unexpected uses:\n{}",
        violations.join("\n")
    );
}

/// Lock the narrowed `ironclaw_runner` public surface in place.
///
/// `ironclaw_runner` previously exposed ~25 types as a wall of `pub use`
/// re-exports (capability resolvers, surface profile filters, milestone
/// scope/sink, model route policies, planned-driver factory helpers, the
/// loop-driver-host factory, etc.). Internal-trace audits found that **no
/// crate outside the reborn family ever named any of those items** and that
/// composition does not need them either — it imports via submodule paths
/// (`ironclaw_runner::driver_registry::DriverRegistry`, etc.). The wall was
/// pure speculative public API.
///
/// This test pins the cleanup: `crates/ironclaw_runner/src/lib.rs` must be a
/// directory of `pub mod` declarations and nothing else. A future contributor
/// who tries to re-add the convenience `pub use` block fails this test
/// alongside the boundary rule that forbids any non-composition crate from
/// taking a normal cargo dep on `ironclaw_runner`.
#[test]
fn reborn_internal_crate_keeps_directory_of_modules_lib_rs() {
    let root = workspace_root();
    let lib = std::fs::read_to_string(root.join("crates/ironclaw_runner/src/lib.rs"))
        .expect("ironclaw_runner lib.rs must be readable");

    // The forbidden re-export prefixes correspond to the original noisy
    // wall. Anyone wanting these items must reach them through a `pub mod`
    // path or (preferably) consume them through `ironclaw_reborn_composition`.
    let forbidden_reexports = [
        "pub use ironclaw_loop_host::",
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
            "ironclaw_runner/src/lib.rs must not re-export internal items via `{forbidden}`. \
             Reach them through the `pub mod` path or through ironclaw_reborn_composition. \
             See `reborn_internal_crate_keeps_directory_of_modules_lib_rs` for context."
        );
    }

    // The composition root is the sanctioned consumer of `ironclaw_runner`'s
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
        "use ironclaw_runner::runtime::",
        "build_default_planned_runtime",
        "DefaultPlannedRuntimeParts",
    ] {
        assert!(
            composition_runtime_source.contains(required),
            "composition runtime.rs missing `{required}` -- the runtime assembly slice \
             must live in `ironclaw_reborn_composition` so the CLI and other \
             ingress points can avoid importing `ironclaw_runner` directly."
        );
    }
    assert!(
        composition_runtime_sources.contains("use ironclaw_loop_host::")
            && composition_runtime_sources.contains("LoopCapabilityPortFactory"),
        "composition runtime module set missing loop-host capability factory wiring -- \
         the host adapter assembly may live in a runtime submodule, but it must stay inside \
         `ironclaw_reborn_composition` rather than the CLI or other ingress points."
    );
}

#[test]
fn composition_runtime_has_no_slack_output_policy() {
    let disguised_policy = r#"
        struct WorkspaceEntityMaskingGateway;

        impl HostManagedModelGateway for WorkspaceEntityMaskingGateway {
            async fn stream_model(&self, request: HostManagedModelRequest) {
                let uses_integration = request.capabilities.iter()
                    .any(|capability| capability.starts_with("slack."));
                if uses_integration {
                    response.output = ParentLoopOutput::AssistantReply(
                        response.content.replace("W0FIXTURE1", "[redacted]")
                    );
                }
            }
        }
    "#;
    assert!(
        source_has_slack_specific_model_output_policy(disguised_policy),
        "the boundary classifier must catch a renamed or moved Slack model-output decorator"
    );

    let imported_policy_installation = r#"
        use crate::{
            outbound::GenericDeliveryProvider,
            slack::entity_policy::WorkspaceEntityMaskingGateway,
        };

        async fn build_runtime(model_gateway: Arc<dyn HostManagedModelGateway>) {
            let model_gateway: Arc<dyn HostManagedModelGateway> = Arc::new(
                WorkspaceEntityMaskingGateway::new(model_gateway),
            );
        }
    "#;
    assert!(
        source_installs_slack_specific_model_output_policy(imported_policy_installation),
        "the boundary classifier must catch an imported Slack decorator at a runtime seam"
    );

    let distant_helper_policy = format!(
        r#"
            struct WorkspaceEntityPolicyGateway;
            impl HostManagedModelGateway for WorkspaceEntityPolicyGateway {{
                async fn stream_model(&self, request: HostManagedModelRequest) {{
                    self.rewrite_response(request).await
                }}
            }}
            {}
            impl WorkspaceEntityPolicyGateway {{
                fn rewrite_response(&self, response: &mut HostManagedModelResponse) {{
                    if response.capability_id.starts_with("slack.") {{
                        response.content = response.content.replace("W0FIXTURE1", "[redacted]");
                    }}
                }}
            }}
        "#,
        "\n".repeat(200)
    );
    assert!(
        source_has_slack_specific_model_output_policy(&distant_helper_policy),
        "the boundary classifier must not depend on helper proximity"
    );

    let allowed_outbound_delivery = r#"
        struct SlackOutboundDeliveryTargetProvider;

        impl OutboundDeliveryTargetProvider for SlackOutboundDeliveryTargetProvider {
            async fn deliver(&self, payload: ProductOutboundPayload) {
                self.slack_client.send(payload).await;
            }
        }
    "#;
    assert!(
        !source_has_slack_specific_model_output_policy(allowed_outbound_delivery),
        "normal integration-owned outbound delivery must remain allowed"
    );

    let co_located_but_unrelated = r#"
        struct GenericSecretMaskingGateway;
        impl HostManagedModelGateway for GenericSecretMaskingGateway {
            async fn stream_model(&self, response: HostManagedModelResponse) {
                response.content = mask_secret(response.content);
            }
        }

        struct SlackOutboundDeliveryTargetProvider;
        impl OutboundDeliveryTargetProvider for SlackOutboundDeliveryTargetProvider {
            async fn deliver(&self, payload: ProductOutboundPayload) {
                self.slack_client.send(payload).await;
            }
        }

        async fn build_runtime(model_gateway: Arc<dyn HostManagedModelGateway>) {
            let model_gateway = Arc::new(GenericSecretMaskingGateway::new(model_gateway));
        }
    "#;
    assert!(
        !source_has_slack_specific_model_output_policy(co_located_but_unrelated),
        "unrelated Slack delivery and generic model policy in one module must not false-positive"
    );
    assert!(
        !source_installs_slack_specific_model_output_policy(co_located_but_unrelated),
        "nearby Slack delivery must not taint a generic gateway installation"
    );

    let aliased_multiline_policy = r#"
        use ironclaw_loop_host::HostManagedModelGateway as ModelGateway;
        struct SlackEntityPolicyGateway;
        impl
            ModelGateway
            for SlackEntityPolicyGateway
        {
            async fn stream_model(&self, response: HostManagedModelResponse) {
                response.content = mask_slack_id(response.content);
            }
        }
    "#;
    assert!(
        source_has_slack_specific_model_output_policy(aliased_multiline_policy),
        "trait aliases and multiline impl headers must not evade the definition scan"
    );

    let interleaved_test_module = r#"
        fn production_before() {}
        #[cfg(test)]
        mod shell_tests;
        fn production_after() {}
    "#;
    let production = source_without_cfg_test_modules(interleaved_test_module);
    assert!(production.contains("production_before"));
    assert!(production.contains("production_after"));
    assert!(!production.contains("shell_tests"));

    let test_support_module = r#"
        #[cfg(any(test, feature = "test-support"))]
        mod runtime_support {
            fn feature_enabled_runtime_policy() {}
        }
    "#;
    let production = source_without_cfg_test_modules(test_support_module);
    assert!(
        production.contains("feature_enabled_runtime_policy"),
        "modules available through a production feature must remain visible to the neutrality scan"
    );

    let production_only_module = r#"
        #[cfg(not(test))]
        mod slack_host_state {
            fn production_runtime_policy() {}
        }
    "#;
    let production = source_without_cfg_test_modules(production_only_module);
    assert!(
        production.contains("production_runtime_policy"),
        "cfg(not(test)) modules are production code and must remain visible to the neutrality scan"
    );

    for production_cfg_module in [
        r#"
            #[cfg(all(unix, any(test, feature = "test-support")))]
            mod nested_test_support {
                fn production_runtime_policy() {}
            }
        "#,
        r#"
            #[cfg(all(unix, /* future,test,arm */ feature = "runtime-policy"))]
            mod commented_cfg_terms {
                fn production_runtime_policy() {}
            }
        "#,
        r#"
            #[cfg(all(unix, feature = "future,test,arm"))]
            mod comma_delimited_feature_name {
                fn production_runtime_policy() {}
            }
        "#,
    ] {
        let production = source_without_cfg_test_modules(production_cfg_module);
        assert!(
            production.contains("production_runtime_policy"),
            "only a direct positive test conjunct may make a cfg(all(...)) module test-only"
        );
    }

    let positive_test_conjunct = r#"
        #[cfg(all(unix, test))]
        mod unix_tests {
            fn test_only_policy() {}
        }
    "#;
    assert!(
        !source_without_cfg_test_modules(positive_test_conjunct).contains("test_only_policy"),
        "a direct positive test conjunct must still be removed from the production scan"
    );

    let root = workspace_root();
    let composition_sources = production_composition_sources(&root);
    let violations = composition_sources
        .iter()
        .filter_map(|(path, source)| {
            let relative = path.strip_prefix(&root).unwrap_or(path).to_string_lossy();
            let is_runtime_root = relative == "crates/ironclaw_reborn_composition/src/runtime.rs";
            let is_runtime_source = is_runtime_root
                || relative.starts_with("crates/ironclaw_reborn_composition/src/runtime/");
            let defines_policy = source_has_slack_specific_model_output_policy(source);
            let installs_policy =
                is_runtime_source && source_installs_slack_specific_model_output_policy(source);
            (defines_policy || installs_policy).then(|| {
                let kind = if defines_policy && installs_policy {
                    "defines and installs"
                } else if defines_policy {
                    "defines"
                } else {
                    "installs"
                };
                format!("{relative}: {kind} Slack-specific model-output policy")
            })
        })
        .collect::<Vec<_>>();
    assert!(
        violations.is_empty(),
        "Reborn composition runtime must remain integration-neutral and must not \
         intercept model output with Slack-specific policy. Violations:\n{}",
        violations.join("\n")
    );

    let slack_policy_module =
        root.join("crates/ironclaw_reborn_composition/src/runtime/slack_output_hygiene.rs");
    assert!(
        !slack_policy_module.exists(),
        "Reborn composition must not own the Slack-specific output policy module at {}",
        slack_policy_module.display()
    );
}

fn production_composition_sources(root: &std::path::Path) -> Vec<(PathBuf, String)> {
    let composition_src = root.join("crates/ironclaw_reborn_composition/src");
    let mut paths = Vec::new();
    let mut pending = vec![composition_src];

    while let Some(dir) = pending.pop() {
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", dir.display()));
        for entry in entries {
            let path = entry
                .unwrap_or_else(|error| panic!("failed to read runtime dir entry: {error}"))
                .path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
                paths.push(path);
            }
        }
    }

    paths.sort();
    paths
        .into_iter()
        .filter(|path| {
            let relative = path.strip_prefix(root).unwrap_or(path).to_string_lossy();
            !is_rust_test_source_path(&relative)
        })
        .map(|path| {
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            (path, source_without_cfg_test_modules(&source))
        })
        .collect()
}

fn source_without_cfg_test_modules(source: &str) -> String {
    let lines = source.lines().collect::<Vec<_>>();
    let mut output = Vec::with_capacity(lines.len());
    let mut index = 0;

    while index < lines.len() {
        let trimmed = lines[index].trim();
        if cfg_attribute_is_test_only(trimmed) {
            let mut module_line = index + 1;
            while module_line < lines.len()
                && (lines[module_line].trim().is_empty()
                    || lines[module_line].trim_start().starts_with("#["))
            {
                module_line += 1;
            }
            if module_line < lines.len() && lines[module_line].trim_start().starts_with("mod ") {
                let module_declaration = lines[module_line];
                if module_declaration.trim_end().ends_with(';') {
                    index = module_line + 1;
                    continue;
                }

                let mut depth = 0;
                let mut saw_opening_brace = false;
                let mut in_block_comment = false;
                index = module_line;
                while index < lines.len() {
                    let code = strip_line_strings_and_comments(lines[index], &mut in_block_comment);
                    if code.contains('{') {
                        saw_opening_brace = true;
                    }
                    depth = update_brace_depth(depth, &code);
                    index += 1;
                    if saw_opening_brace && depth == 0 {
                        break;
                    }
                }
                continue;
            }
        }

        output.push(lines[index]);
        index += 1;
    }

    output.join("\n")
}

fn cfg_attribute_is_test_only(attribute: &str) -> bool {
    if !attribute.starts_with("#[cfg(") {
        return false;
    }
    let Some(expression) = attribute
        .strip_prefix("#[cfg(")
        .and_then(|value| value.strip_suffix(")]"))
        .map(str::trim)
    else {
        return false;
    };
    if expression == "test" {
        return true;
    }
    let Some(all_terms) = expression
        .strip_prefix("all")
        .map(str::trim_start)
        .and_then(|value| value.strip_prefix('('))
        .and_then(|value| value.strip_suffix(')'))
    else {
        return false;
    };
    cfg_all_has_direct_test_conjunct(all_terms)
}

fn cfg_all_has_direct_test_conjunct(all_terms: &str) -> bool {
    let bytes = all_terms.as_bytes();
    let mut index = 0;
    let mut parenthesis_depth = 0_usize;
    let mut normalized_term = Vec::new();

    while index < bytes.len() {
        if bytes[index..].starts_with(b"/*") {
            let Some(comment_end) = nested_block_comment_end(bytes, index) else {
                return false;
            };
            index = comment_end;
            continue;
        }
        if bytes[index..].starts_with(b"//") {
            return false;
        }
        if let Some((quote_index, hash_count)) = raw_string_delimiter(bytes, index) {
            if parenthesis_depth == 0 {
                normalized_term.push(b'"');
            }
            let Some(string_end) = raw_string_end(bytes, quote_index, hash_count) else {
                return false;
            };
            index = string_end;
            continue;
        }
        if matches!(bytes[index], b'"' | b'\'') {
            if parenthesis_depth == 0 {
                normalized_term.push(bytes[index]);
            }
            let Some(string_end) = quoted_literal_end(bytes, index, bytes[index]) else {
                return false;
            };
            index = string_end;
            continue;
        }

        match bytes[index] {
            b'(' => {
                if parenthesis_depth == 0 {
                    normalized_term.push(b'(');
                }
                parenthesis_depth += 1;
            }
            b')' => {
                let Some(next_depth) = parenthesis_depth.checked_sub(1) else {
                    return false;
                };
                parenthesis_depth = next_depth;
                if parenthesis_depth == 0 {
                    normalized_term.push(b')');
                }
            }
            b',' if parenthesis_depth == 0 => {
                if normalized_term == b"test" {
                    return true;
                }
                normalized_term.clear();
            }
            byte if parenthesis_depth == 0 && !byte.is_ascii_whitespace() => {
                normalized_term.push(byte);
            }
            _ => {}
        }
        index += 1;
    }

    parenthesis_depth == 0 && normalized_term == b"test"
}

fn nested_block_comment_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start + 2;
    let mut depth = 1_usize;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"/*") {
            depth += 1;
            index += 2;
        } else if bytes[index..].starts_with(b"*/") {
            depth -= 1;
            index += 2;
            if depth == 0 {
                return Some(index);
            }
        } else {
            index += 1;
        }
    }
    None
}

fn raw_string_delimiter(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut index = start;
    if bytes.get(index) == Some(&b'b') {
        index += 1;
    }
    if bytes.get(index) != Some(&b'r') {
        return None;
    }
    index += 1;
    let hash_start = index;
    while bytes.get(index) == Some(&b'#') {
        index += 1;
    }
    (bytes.get(index) == Some(&b'"')).then_some((index, index - hash_start))
}

fn raw_string_end(bytes: &[u8], quote_index: usize, hash_count: usize) -> Option<usize> {
    let mut index = quote_index + 1;
    while index < bytes.len() {
        if bytes[index] == b'"'
            && bytes
                .get(index + 1..index + 1 + hash_count)
                .is_some_and(|hashes| hashes.iter().all(|byte| *byte == b'#'))
        {
            return Some(index + 1 + hash_count);
        }
        index += 1;
    }
    None
}

fn quoted_literal_end(bytes: &[u8], quote_index: usize, quote: u8) -> Option<usize> {
    let mut index = quote_index + 1;
    let mut escaped = false;
    while index < bytes.len() {
        if escaped {
            escaped = false;
        } else if bytes[index] == b'\\' {
            escaped = true;
        } else if bytes[index] == quote {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}

fn source_has_slack_specific_model_output_policy(source: &str) -> bool {
    const OUTPUT_MUTATION_MARKERS: &[&str] = &[
        "safe_text_update",
        "safe_reasoning_update",
        "safe_text_deltas",
        "safe_reasoning_deltas",
        "parentloopoutput::assistantreply",
        "response.output =",
        ".content =",
        ".content.replace",
        "redact",
        "sanitize",
        "mask",
    ];

    let impl_blocks = rust_impl_blocks(source);
    let gateway_interception_markers = gateway_trait_impl_markers(source);
    let gateway_types = impl_blocks
        .iter()
        .flat_map(|(header, _)| {
            let normalized_header = normalize_rust_whitespace(header);
            gateway_interception_markers
                .iter()
                .filter_map(move |marker| impl_type_after_marker(&normalized_header, marker))
        })
        .collect::<BTreeSet<_>>();

    gateway_types.into_iter().any(|gateway_type| {
        let owned_regions = impl_blocks
            .iter()
            .filter(|(header, _)| rust_header_mentions_type(header, &gateway_type))
            .map(|(_, body)| body.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            .to_ascii_lowercase();
        owned_regions.contains("slack")
            && OUTPUT_MUTATION_MARKERS
                .iter()
                .any(|marker| owned_regions.contains(marker))
    })
}

fn gateway_trait_impl_markers(source: &str) -> BTreeSet<String> {
    const GATEWAY_TRAITS: &[&str] = &["HostManagedModelGateway", "HostManagedModelStreamSink"];

    let normalized_source = normalize_rust_whitespace(source);
    let mut markers = GATEWAY_TRAITS
        .iter()
        .map(|trait_name| format!("{trait_name} for"))
        .collect::<BTreeSet<_>>();
    for trait_name in GATEWAY_TRAITS {
        let alias_prefix = format!("{trait_name} as ");
        let mut remainder = normalized_source.as_str();
        while let Some(index) = remainder.find(&alias_prefix) {
            let after_alias = &remainder[index + alias_prefix.len()..];
            let alias = after_alias
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .next()
                .unwrap_or_default();
            if !alias.is_empty() {
                markers.insert(format!("{alias} for"));
            }
            remainder = after_alias;
        }
    }
    markers
}

fn normalize_rust_whitespace(source: &str) -> String {
    source.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn rust_impl_blocks(source: &str) -> Vec<(String, String)> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut blocks = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        let trimmed = lines[index].trim_start();
        if !(trimmed == "impl" || trimmed.starts_with("impl ") || trimmed.starts_with("impl<")) {
            index += 1;
            continue;
        }

        let start = index;
        let mut depth = 0;
        let mut saw_opening_brace = false;
        let mut in_block_comment = false;
        while index < lines.len() {
            let code = strip_line_strings_and_comments(lines[index], &mut in_block_comment);
            if code.contains('{') {
                saw_opening_brace = true;
            }
            depth = update_brace_depth(depth, &code);
            index += 1;
            if saw_opening_brace && depth == 0 {
                break;
            }
        }

        let body = lines[start..index].join("\n");
        let header = body.split('{').next().unwrap_or_default().to_string();
        blocks.push((header, body));
    }

    blocks
}

fn impl_type_after_marker(header: &str, marker: &str) -> Option<String> {
    header
        .split_once(marker)
        .and_then(|(_, suffix)| {
            suffix
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .find(|token| !token.is_empty())
        })
        .map(ToString::to_string)
}

fn rust_header_mentions_type(header: &str, type_name: &str) -> bool {
    header
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(|token| token == type_name)
}

fn source_installs_slack_specific_model_output_policy(source: &str) -> bool {
    const WRAPPER_CONSTRUCTION_MARKERS: &[&str] = &[
        "arc::new",
        "box::new",
        "::new(",
        "wrap(",
        "decorate",
        "let model_gateway",
    ];

    let (slack_import_symbols, has_slack_wildcard_import) = slack_import_symbols(source);
    model_gateway_statements(source)
        .into_iter()
        .any(|statement| {
            let lower_statement = statement.to_ascii_lowercase();
            let constructs_wrapper = WRAPPER_CONSTRUCTION_MARKERS
                .iter()
                .any(|marker| lower_statement.contains(marker));
            let names_slack_directly = lower_statement.contains("slack::")
                || lower_statement.contains("crate::slack")
                || lower_statement
                    .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                    .any(|token| token.starts_with("slack") && token.len() > "slack".len());
            let uses_slack_import = slack_import_symbols
                .iter()
                .any(|symbol| rust_header_mentions_type(&statement, symbol));
            constructs_wrapper
                && (names_slack_directly || uses_slack_import || has_slack_wildcard_import)
        })
}

fn model_gateway_statements(source: &str) -> Vec<String> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut statements = BTreeSet::new();
    for (index, line) in lines.iter().enumerate() {
        if !line.contains("model_gateway") {
            continue;
        }

        let mut start = index;
        while start > 0 {
            let trimmed = lines[start].trim_start();
            if trimmed.starts_with("let ") || trimmed.starts_with("model_gateway =") {
                break;
            }
            let previous = lines[start - 1].trim();
            if previous.ends_with(';') || previous.ends_with('{') || previous.ends_with('}') {
                break;
            }
            start -= 1;
        }

        let mut end = index;
        while end + 1 < lines.len() && !lines[end].contains(';') {
            end += 1;
        }
        statements.insert(lines[start..=end].join("\n"));
    }
    statements.into_iter().collect()
}

fn slack_import_symbols(source: &str) -> (BTreeSet<String>, bool) {
    const IGNORED_IMPORT_TOKENS: &[&str] = &["as", "crate", "pub", "self", "slack", "super", "use"];

    let mut symbols = BTreeSet::new();
    let mut has_wildcard = false;
    for statement in rust_use_statements(source) {
        let normalized = normalize_rust_whitespace(&statement);
        let lower = normalized.to_ascii_lowercase();
        if lower.contains("slack::*") {
            has_wildcard = true;
        }

        let mut search_from = 0;
        while let Some(relative_index) = lower[search_from..].find("slack::") {
            let suffix_start = search_from + relative_index + "slack::".len();
            let suffix = &normalized[suffix_start..];
            let segment = slack_import_segment(suffix);
            for token in segment
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            {
                if token.len() > 2 && !IGNORED_IMPORT_TOKENS.contains(&token) {
                    symbols.insert(token.to_string());
                }
            }
            search_from = suffix_start;
        }

        if let Some(alias_index) = lower.find("slack as ") {
            let alias = normalized[alias_index + "slack as ".len()..]
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .next()
                .unwrap_or_default();
            if !alias.is_empty() {
                symbols.insert(alias.to_string());
            }
        }
    }
    (symbols, has_wildcard)
}

fn rust_use_statements(source: &str) -> Vec<String> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut statements = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim_start();
        if !(trimmed.starts_with("use ") || trimmed.starts_with("pub use ")) {
            index += 1;
            continue;
        }

        let start = index;
        while index + 1 < lines.len() && !lines[index].contains(';') {
            index += 1;
        }
        statements.push(lines[start..=index].join("\n"));
        index += 1;
    }
    statements
}

fn slack_import_segment(suffix: &str) -> &str {
    if !suffix.starts_with('{') {
        return suffix.split([',', ';', '}']).next().unwrap_or_default();
    }

    let mut depth = 0;
    for (index, character) in suffix.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &suffix[1..index];
                }
            }
            _ => {}
        }
    }
    suffix
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
    let llm_catalog = root.join("crates/ironclaw_reborn_composition/src/llm_admin/llm_catalog.rs");
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
fn wasm_sandbox_core_module_stays_domain_free_v1_parity_kernel() {
    let workspace = workspace_root();
    let module = workspace.join("crates/ironclaw_wasm/src/wasm_sandbox_core.rs");
    assert!(
        module.exists(),
        "shared WASM sandbox core should stay as a module inside ironclaw_wasm after W2.3"
    );
    let guardrails = std::fs::read_to_string(workspace.join("crates/ironclaw_wasm/CLAUDE.md"))
        .expect("ironclaw_wasm guardrails must be readable");
    assert!(
        guardrails.contains("wasm_sandbox_core")
            && guardrails.contains("Do not put ProductAdapter"),
        "ironclaw_wasm guardrails must preserve the folded sandbox-core domain-free rule"
    );

    let source = std::fs::read_to_string(&module).expect("WASM sandbox core module is readable");
    for forbidden in [
        "ironclaw_product",
        "ironclaw_dispatcher",
        "ironclaw_extensions",
        "ironclaw_filesystem",
        "ironclaw_network",
        "ironclaw_secrets",
        "ironclaw_host_runtime",
        "ironclaw_reborn_composition",
    ] {
        assert!(
            !source.contains(forbidden),
            "folded WASM sandbox core module must stay independent of product/runtime/app crates; \
             unexpected reference to `{forbidden}`"
        );
    }

    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");
    assert!(
        packages
            .iter()
            .all(|package| package["name"] != "ironclaw_wasm_sandbox_core"),
        "ironclaw_wasm_sandbox_core should remain folded into ironclaw_wasm after W2.3"
    );

    let limiter_package = packages
        .iter()
        .find(|package| package["name"] == "ironclaw_wasm_limiter")
        .expect("ironclaw_wasm_limiter must be a workspace package");
    let limiter_workspace_deps = workspace_dependency_names(limiter_package)
        .filter_map(|dependency| dependency["name"].as_str())
        .collect::<Vec<_>>();
    assert!(
        limiter_workspace_deps.is_empty(),
        "ironclaw_wasm_limiter is allowed only as low-level WASM accounting; \
         got workspace deps: {limiter_workspace_deps:?}"
    );
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
fn hosted_mcp_discovery_is_never_driven_by_ambient_startup_composition() {
    let root = workspace_root();
    let factory =
        std::fs::read_to_string(root.join("crates/ironclaw_reborn_composition/src/factory.rs"))
            .expect("composition factory source must be readable");
    let owner_transaction = std::fs::read_to_string(
        root.join("crates/ironclaw_extension_host/src/activation_transaction.rs"),
    )
    .expect("extension-host activation transaction source must be readable");

    for forbidden in [
        "reconcile_hosted_mcp_runtime_readiness",
        "reconcile_hosted_mcp_startup",
    ] {
        assert!(
            !factory.contains(forbidden),
            "composition startup must not invoke hosted-MCP discovery through `{forbidden}`; \
             discovery requires a real caller/run ResourceScope"
        );
        assert!(
            !owner_transaction.contains(forbidden),
            "extension-host must not expose ambient hosted-MCP startup probing through \
             `{forbidden}`"
        );
    }
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
        "crates/ironclaw_runner/src",
        "crates/ironclaw_reborn_cli/src",
        "crates/ironclaw_reborn_composition/src",
        "crates/ironclaw_reborn_config/src",
        "crates/ironclaw_reborn_event_store/src",
        "crates/ironclaw_reborn_api/src",
        "crates/ironclaw_reborn_openai_compat/src",
        "crates/ironclaw_product/src",
        "crates/ironclaw_product/src",
        "crates/ironclaw_product/src",
        "crates/ironclaw_telegram_extension/src",
        "crates/ironclaw_slack_extension/src",
        "crates/ironclaw_outbound/src",
        "crates/ironclaw_conversations/src",
        "crates/ironclaw_turns/src",
        "crates/ironclaw_threads/src",
        "crates/ironclaw_loop_host/src",
        // WebChat v2 route surface: a Product/API crate that exposes
        // axum handler functions and `IngressRouteDescriptor`s but must
        // never bind sockets or call `axum::serve` itself — that is
        // host composition's job. Without this entry the contract fails
        // open for the new route crate.
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
fn reborn_openai_compat_routes_do_not_depend_on_v1_gateway_or_legacy_streams() {
    let forbidden = [
        ForbiddenUse {
            pattern: "src/channels/web",
            reason: "OpenAI-compatible Reborn routes must not route through v1 gateway handlers",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "crate::channels::web",
            reason: "OpenAI-compatible Reborn routes must not import v1 gateway modules",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "ironclaw::channels::web",
            reason: "OpenAI-compatible Reborn routes must not import v1 gateway modules",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "GatewayState",
            reason: "OpenAI-compatible Reborn routes must not depend on v1 gateway state",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "SseManager",
            reason: "OpenAI-compatible Reborn streaming must use projection-stream ports, not raw legacy SSE streams",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "AppEvent",
            reason: "OpenAI-compatible Reborn streaming must translate ProductProjectionItem state, not raw legacy AppEvent streams",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "IncomingMessage",
            reason: "OpenAI-compatible Reborn routes must enter through ProductSurface, not legacy channel ingress",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "get_or_create_assistant_conversation",
            reason: "OpenAI-compatible Reborn retrieve/cancel must use opaque refs and projection readers, not legacy conversation reconstruction",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "ConversationManager",
            reason: "OpenAI-compatible Reborn routes must not reconstruct legacy conversations directly",
            exempt: None,
        },
    ];

    let root = workspace_root();
    let compat_src = root.join("crates/ironclaw_reborn_openai_compat/src");
    let mut violations = Vec::new();
    collect_forbidden_uses(&compat_src, &root, &forbidden, &mut violations);

    assert!(
        violations.is_empty(),
        "Reborn OpenAI-compatible routes must stay ProductSurface/projection-port backed and independent of v1 gateway handlers, legacy SSE/AppEvent streams, and legacy conversation reconstruction:\n{}",
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
        &root.join("crates/ironclaw_reborn_composition/src/product_auth/api/auth.rs"),
        &root,
        &forbidden,
        &mut violations,
    );
    collect_forbidden_reborn_auth_path_uses(
        &root.join("crates/ironclaw_reborn_composition/src/product_auth/serve"),
        &root.join("crates/ironclaw_reborn_composition/src/product_auth/serve.rs"),
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

struct ProviderToolNameUse {
    path: String,
    line_number: usize,
    pattern: &'static str,
}

fn collect_provider_tool_name_boundary_uses(
    dir: &std::path::Path,
    root: &std::path::Path,
    uses: &mut Vec<ProviderToolNameUse>,
) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}"));
        let path = entry.path();
        if path.is_dir() {
            collect_provider_tool_name_boundary_uses(&path, root, uses);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        if is_rust_test_source_path(&relative) {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        collect_provider_tool_name_uses_in_source(&relative, &contents, uses);
    }
}

fn is_rust_test_source_path(relative: &str) -> bool {
    relative.contains("/tests/")
        || relative.ends_with("/tests.rs")
        || relative.ends_with("_tests.rs")
}

fn collect_provider_tool_name_uses_in_source(
    relative: &str,
    contents: &str,
    uses: &mut Vec<ProviderToolNameUse>,
) {
    let mut pending_cfg_test = false;
    let mut skipping_test_module_depth: Option<usize> = None;
    let mut in_block_comment = false;
    for (index, line) in contents.lines().enumerate() {
        let line_number = index + 1;
        let code = strip_line_strings_and_comments(line, &mut in_block_comment);
        if let Some(depth) = skipping_test_module_depth {
            let next_depth = update_brace_depth(depth, &code);
            if next_depth == 0 {
                skipping_test_module_depth = None;
            } else {
                skipping_test_module_depth = Some(next_depth);
            }
            continue;
        }

        let trimmed = line.trim();
        if trimmed.starts_with("#[cfg(test)]") {
            pending_cfg_test = true;
            continue;
        }
        let code_trimmed = code.trim();
        if pending_cfg_test {
            if code_trimmed.starts_with("#[") || code_trimmed.is_empty() {
                continue;
            }
            if code_trimmed.contains('{') {
                let depth = update_brace_depth(0, &code);
                if depth > 0 {
                    skipping_test_module_depth = Some(depth);
                }
                pending_cfg_test = false;
                continue;
            }
            pending_cfg_test = false;
        }

        for pattern in ["ProviderToolName", "provider_tool_name"] {
            if code.contains(pattern) {
                uses.push(ProviderToolNameUse {
                    path: relative.to_string(),
                    line_number,
                    pattern,
                });
            }
        }
    }
}

fn strip_line_strings_and_comments(line: &str, in_block_comment: &mut bool) -> String {
    let mut output = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(character) = chars.next() {
        if *in_block_comment {
            if character == '*' && chars.peek() == Some(&'/') {
                chars.next();
                *in_block_comment = false;
                output.push(' ');
                output.push(' ');
            } else {
                output.push(' ');
            }
            continue;
        }
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            output.push(' ');
            continue;
        }
        if character == '"' {
            in_string = true;
            output.push(' ');
            continue;
        }
        if character == '/' && chars.peek() == Some(&'/') {
            break;
        }
        if character == '/' && chars.peek() == Some(&'*') {
            chars.next();
            *in_block_comment = true;
            output.push(' ');
            output.push(' ');
            continue;
        }
        output.push(character);
    }
    output
}

fn update_brace_depth(depth: usize, line: &str) -> usize {
    let opens = line.chars().filter(|character| *character == '{').count();
    let closes = line.chars().filter(|character| *character == '}').count();
    depth.saturating_add(opens).saturating_sub(closes)
}

struct BoundaryRule {
    crate_name: &'static str,
    forbidden: Vec<&'static str>,
}

fn boundary_rules() -> Vec<BoundaryRule> {
    vec![
        BoundaryRule {
            crate_name: "ironclaw_product",
            forbidden: vec![
                "ironclaw_dispatcher",
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
            // Product auth is a Reborn contract/facade vocabulary plus the
            // recipe-driven auth engine (extension-runtime workstream D). The
            // engine owns token secret storage, so the Reborn-native
            // `ironclaw_secrets` store is allowed; implementation code must
            // still not reach into v1 routes, extension managers, runtimes,
            // or channel-specific stacks.
            crate_name: "ironclaw_auth",
            forbidden: vec![
                "ironclaw_legacy",
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
                "ironclaw_loop_host",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_product",
                "ironclaw_product",
                "ironclaw_runner",
                "ironclaw",
                "ironclaw_reborn_composition",
                "ironclaw_reborn_config",
                "ironclaw_reborn_event_store",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_scripts",
                "ironclaw_skills",
                "ironclaw_storage",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_turns",
                "ironclaw_wasm",
            ],
        },
        // NOTE(webui-merge): the former `ironclaw_webui_v2` BoundaryRule was
        // removed when that crate's route surface was folded into
        // `ironclaw_webui` (as its `webui_v2` module). The
        // "handlers do not touch adapters/dispatcher/runtime directly"
        // invariant is now carried by the `ironclaw_webui`
        // rule's forbidden list.
        BoundaryRule {
            // OpenAI-compatible route surface is a Reborn product/API facade.
            // It may depend on host ingress vocabulary, product adapter
            // contracts, and the ProductSurface facade, but it must not revive
            // v1 gateway/LLM proxy paths or reach into runtime/composition
            // services directly.
            crate_name: "ironclaw_reborn_openai_compat",
            forbidden: vec![
                "ironclaw_legacy",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_dispatcher",
                "ironclaw_engine",
                "ironclaw_event_projections",
                "ironclaw_event_streams",
                "ironclaw_events",
                "ironclaw_extensions",
                // `ironclaw_filesystem` is permitted: the durable
                // FilesystemOpenAiCompatRefStore lives behind the
                // `storage`/`libsql`/`postgres` features and persists opaque refs
                // through the universal RootFilesystem port.
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_loop_host",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_runner",
                "ironclaw",
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
            ],
        },
        BoundaryRule {
            // First-party extensions are userland implementation packages.
            // They may consume scoped storage and pure safety helpers, but
            // must not receive ambient runtime authority or loop-facing
            // runtime handles.
            crate_name: "ironclaw_first_party_extensions",
            forbidden: vec![
                "ironclaw_legacy",
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
                "ironclaw_loop_host",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_product",
                "ironclaw_product",
                "ironclaw_runner",
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
            ],
        },
        BoundaryRule {
            // First-party extension ports are adapter glue above concrete
            // userland implementations. They may depend on loop/turn-facing
            // contracts, but must not reach into host runtime authority or
            // product composition.
            crate_name: "ironclaw_first_party_extension_ports",
            forbidden: vec![
                "ironclaw_legacy",
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
                "ironclaw_product",
                "ironclaw_product",
                "ironclaw_runner",
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
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_reborn_config",
            forbidden: vec![
                "ironclaw_legacy",
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
                "ironclaw_loop_host",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_runner",
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
            crate_name: "ironclaw",
            forbidden: vec![
                "ironclaw_legacy",
                "ironclaw_engine",
                "ironclaw_gateway",
                "ironclaw_llm",
                "ironclaw_loop_host",
                "ironclaw_runner",
                "ironclaw_skills",
                "ironclaw_threads",
                "ironclaw_tui",
                "ironclaw_turns",
            ],
        },
        BoundaryRule {
            // Host-owned WebUI ingress: binds the TCP listener and runs
            // the axum serve loop for the composed v2 Router. Since the
            // `ironclaw_webui_v2` route surface was folded into this crate
            // (as its `webui_v2` module), it now legitimately consumes the
            // `ironclaw_product` `ProductSurface` facade the v2
            // handlers dispatch through. It still must not pull lower
            // substrate handles, product adapters, or v1 surface code into
            // the binary path. Reaches the rest of Reborn through
            // ironclaw_reborn_composition's facade (Router + WebuiAuthenticator
            // trait + WebuiServeConfig + mount vocabulary + product-auth mount
            // builders).
            crate_name: "ironclaw_webui",
            forbidden: vec![
                "ironclaw_legacy",
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
                "ironclaw_loop_host",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_runner",
                "ironclaw",
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
                "ironclaw_legacy",
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
                "ironclaw_legacy",
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
                "ironclaw_product",
                "ironclaw_product",
                "ironclaw_reborn_event_store",
                "ironclaw_runner",
                "ironclaw",
                "ironclaw_reborn_composition",
                "ironclaw_reborn_config",
                "ironclaw_resources",
                "ironclaw_run_state",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_scripts",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_telegram_extension",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            // Concrete Slack protocol adapter owns only Slack payload
            // normalization/rendering over the ProductAdapter DTO surface.
            // Host auth verification, credential resolution, delivery fanout,
            // workflow admission, and runtime/network authority stay outside
            // the adapter crate. `ironclaw_host_api` is deliberately allowed:
            // concrete extension crates depend on host_api /
            // product_adapters CONTRACT types only (`RestrictedEgress`,
            // `SecretHandle` — extension-runtime implementation.md §3), which
            // the P4 `ChannelAdapter` signatures require.
            crate_name: "ironclaw_slack_extension",
            forbidden: vec![
                "ironclaw_legacy",
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
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_loop_host",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_product",
                "ironclaw_runner",
                "ironclaw",
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
                "ironclaw_telegram_extension",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_outbound",
            forbidden: vec![
                "ironclaw_legacy",
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
                "ironclaw_legacy",
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
                "ironclaw_product",
                "ironclaw_product",
                "ironclaw_runner",
                "ironclaw",
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
                "ironclaw_legacy",
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
                // ironclaw_filesystem is permitted: FilesystemTurnStateRowStore
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
                "ironclaw_runner",
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
        // secrets, DB backends, or the loop-host adapter layer — those all
        // sit above agent_loop in the stack and would create an inversion.
        BoundaryRule {
            crate_name: "ironclaw_agent_loop",
            forbidden: vec![
                "ironclaw_legacy",
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
                "ironclaw_loop_host",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_product",
                "ironclaw_product",
                "ironclaw_runner",
                "ironclaw",
                "ironclaw_reborn_composition",
                "ironclaw_reborn_config",
                "ironclaw_reborn_event_store",
                "ironclaw_reborn_traces",
                "ironclaw_webui",
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

const IRONCLAW_CRATE_LAYERS: [&str; 8] = [
    "contracts",
    "substrates",
    "runtimes",
    "kernel",
    "loops",
    "products",
    "app",
    "legacy",
];

struct LayerMatrixException {
    crate_name: &'static str,
    dependency_name: &'static str,
    introduced: &'static str,
    removes_in: &'static str,
    reason: &'static str,
}

const LAYER_MATRIX_EXCEPTIONS: &[LayerMatrixException] = &[
    LayerMatrixException {
        crate_name: "ironclaw_host_runtime",
        dependency_name: "ironclaw_extensions",
        introduced: "2026-07-09",
        removes_in: "W7",
        reason: "host_runtime still owns extension-hosting wiring until kernel consolidation moves only the execution perimeter into kernel",
    },
    LayerMatrixException {
        crate_name: "ironclaw_host_runtime",
        dependency_name: "ironclaw_first_party_extensions",
        introduced: "2026-07-09",
        removes_in: "W7",
        reason: "host_runtime still owns first-party extension activation wiring until kernel consolidation separates host policy from loop/product concerns",
    },
    LayerMatrixException {
        crate_name: "ironclaw_host_runtime",
        dependency_name: "ironclaw_skills",
        introduced: "2026-07-09",
        removes_in: "W7",
        reason: "host_runtime still owns first-party skill management tools and skill URL install limits; remove when kernel consolidation or a dedicated skill-host extraction moves that execution surface out of host_runtime",
    },
    LayerMatrixException {
        crate_name: "ironclaw_capabilities",
        dependency_name: "ironclaw_extensions",
        introduced: "2026-07-09",
        removes_in: "W7",
        reason: "capability hosting still reaches the extension surface until the kernel perimeter is consolidated",
    },
    LayerMatrixException {
        crate_name: "ironclaw_processes",
        dependency_name: "ironclaw_resources",
        introduced: "2026-07-09",
        removes_in: "W7",
        reason: "runtime process management still depends on resource contracts currently classed with kernel behavior",
    },
    LayerMatrixException {
        crate_name: "ironclaw_event_projections",
        dependency_name: "ironclaw_turns",
        introduced: "2026-07-09",
        removes_in: "W4.3",
        reason: "projection state reads turn DTOs that move to turn_contracts if the JIT split fires",
    },
    LayerMatrixException {
        crate_name: "ironclaw_triggers",
        dependency_name: "ironclaw_turns",
        introduced: "2026-07-09",
        removes_in: "W4.3",
        reason: "trigger state reads turn DTOs that move to turn_contracts if the JIT split fires",
    },
    LayerMatrixException {
        crate_name: "ironclaw_conversations",
        dependency_name: "ironclaw_turns",
        introduced: "2026-07-09",
        removes_in: "W4.3",
        reason: "conversation ingress still names turn DTOs that move to turn_contracts if the JIT split fires",
    },
    LayerMatrixException {
        crate_name: "ironclaw_hooks",
        dependency_name: "ironclaw_turns",
        introduced: "2026-07-09",
        removes_in: "W4.3",
        reason: "hook payloads still name turn DTOs that move to turn_contracts if the JIT split fires",
    },
    LayerMatrixException {
        crate_name: "ironclaw_hooks",
        dependency_name: "ironclaw_wasm_limiter",
        introduced: "2026-07-09",
        removes_in: "W6",
        reason: "hooks still reuse the WASM limiter crate before the directory re-layout verifies runtime/substrate placement",
    },
    LayerMatrixException {
        crate_name: "ironclaw_outbound",
        dependency_name: "ironclaw_turns",
        introduced: "2026-07-09",
        removes_in: "W4.3",
        reason: "outbound delivery still names turn DTOs that move to turn_contracts if the JIT split fires",
    },
    LayerMatrixException {
        crate_name: "ironclaw_event_streams",
        dependency_name: "ironclaw_turns",
        introduced: "2026-07-09",
        removes_in: "W4.3",
        reason: "event stream contracts still name turn DTOs that move to turn_contracts if the JIT split fires",
    },
    LayerMatrixException {
        crate_name: "ironclaw_agent_loop",
        dependency_name: "ironclaw_turns",
        introduced: "2026-07-09",
        removes_in: "W4.3",
        reason: "agent_loop still names turn DTOs directly until the turn_contracts JIT split moves the type surface to contracts",
    },
    LayerMatrixException {
        crate_name: "ironclaw_mcp",
        dependency_name: "ironclaw_extensions",
        introduced: "2026-07-09",
        removes_in: "W7",
        reason: "MCP runtime still consumes ExtensionPackage and ExtensionRuntime manifest DTOs; remove when extension runtime descriptors move to a neutral contract or runtime lanes are folded behind the extension-host boundary",
    },
    LayerMatrixException {
        crate_name: "ironclaw_mcp",
        dependency_name: "ironclaw_resources",
        introduced: "2026-07-09",
        removes_in: "W7",
        reason: "MCP runtime support still depends on resource contracts currently classed with kernel behavior",
    },
    LayerMatrixException {
        crate_name: "ironclaw_scripts",
        dependency_name: "ironclaw_extensions",
        introduced: "2026-07-09",
        removes_in: "W7",
        reason: "script runtime still consumes ExtensionPackage and ExtensionRuntime manifest DTOs; remove when extension runtime descriptors move to a neutral contract or runtime lanes are folded behind the extension-host boundary",
    },
    LayerMatrixException {
        crate_name: "ironclaw_scripts",
        dependency_name: "ironclaw_resources",
        introduced: "2026-07-09",
        removes_in: "W7",
        reason: "script runtime support still depends on resource contracts currently classed with kernel behavior",
    },
    LayerMatrixException {
        crate_name: "ironclaw_runner",
        dependency_name: "ironclaw_agent_loop",
        introduced: "2026-07-09",
        removes_in: "W7",
        reason: "the runner intentionally bridges loop-userland contracts until kernel consolidation introduces a neutral dispatch boundary",
    },
    LayerMatrixException {
        crate_name: "ironclaw_runner",
        dependency_name: "ironclaw_loop_host",
        introduced: "2026-07-09",
        removes_in: "W7",
        reason: "the runner intentionally composes loop-host adapters until kernel consolidation introduces a neutral dispatch boundary",
    },
    LayerMatrixException {
        crate_name: "ironclaw_webui",
        dependency_name: "ironclaw_reborn_composition",
        introduced: "2026-07-09",
        removes_in: "W3.6",
        reason: "webui ingress still reaches composition until the composition webui module is folded into ingress and runtime handles are inverted",
    },
];

fn layer_matrix_exception(
    crate_name: &str,
    dependency_name: &str,
) -> Option<&'static LayerMatrixException> {
    LAYER_MATRIX_EXCEPTIONS.iter().find(|exception| {
        exception.crate_name == crate_name && exception.dependency_name == dependency_name
    })
}

fn package_layer(package: &Value) -> Option<&str> {
    package
        .get("metadata")?
        .get("ironclaw")?
        .get("layer")?
        .as_str()
}

fn is_ironclaw_workspace_package(name: &str) -> bool {
    name == "ironclaw" || name.starts_with("ironclaw_")
}

fn layer_allows_dependency(crate_layer: &str, dependency_layer: &str) -> bool {
    match crate_layer {
        "contracts" => matches!(dependency_layer, "contracts"),
        "substrates" => matches!(dependency_layer, "contracts" | "substrates"),
        "runtimes" => matches!(dependency_layer, "contracts" | "substrates" | "runtimes"),
        "kernel" => matches!(
            dependency_layer,
            "contracts" | "substrates" | "runtimes" | "kernel"
        ),
        "loops" => matches!(
            dependency_layer,
            "contracts" | "substrates" | "runtimes" | "kernel" | "loops"
        ),
        "products" => matches!(
            dependency_layer,
            "contracts" | "substrates" | "runtimes" | "kernel" | "loops" | "products"
        ),
        "app" => matches!(
            dependency_layer,
            "contracts" | "substrates" | "runtimes" | "kernel" | "loops" | "products" | "app"
        ),
        // The v1 package/crates may still depend on Reborn while parity work
        // is in flight, but Reborn layers are intentionally not allowed to
        // depend back on legacy.
        "legacy" => true,
        _ => false,
    }
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
