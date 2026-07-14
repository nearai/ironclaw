//! Zero-legacy gate for the NEA-25 unified extension model.
//!
//! The unified model retired an entire vocabulary: the connectable-channels
//! rail, the one-variant lifecycle surface kind, the conflated extension
//! `kind` wire string, the split `slack_bot` package identity, the
//! `slack_personal` provider id, and the contract-free manifest parse paths.
//! This test pins all of it at **zero occurrences** across Reborn code
//! (`crates/`, the WebUI frontend sources, `tests/integration/`, and Reborn
//! Python E2E scenarios) so none of it can be reintroduced silently.
//!
//! Sanctioned exceptions are path-scoped, not term-scoped:
//! - `crates/ironclaw_reborn_migration/` reads v1 domain vocabulary by design;
//! - the two one-time forward data migrations legitimately name the retired
//!   identities they fold forward
//!   (`extension_host/extension_installation_store.rs`, its adjacent test
//!   module, `product_auth/durable/`, and the dedicated factory migration
//!   integration module);
//! - this test names every term on purpose.
//!
//! v1 (`src/`, root `tests/*.rs`) is out of scope: it is being strangled
//! wholesale, not policed term-by-term.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("architecture crate under crates")
        .to_path_buf()
}

/// Retired vocabulary. Every term here was deleted by the NEA-25 stack; a hit
/// outside the sanctioned paths is a regression, not a style issue.
const RETIRED_TERMS: &[&str] = &[
    // The connectable-channels rail (replaced by extension-surface discovery).
    "ConnectableChannelsProductFacade",
    "RebornConnectableChannelInfo",
    "RebornConnectableChannelListResponse",
    "list_connectable_channels",
    "listConnectableChannels",
    "SlackOperatorRouteVisibility",
    "channel_connection_facade_slot",
    "RebornChannelConnectAction",
    // `RebornChannelConnectStrategy` remains current. These three variants do
    // not: Train A retained only OAuth and inbound proof-code setup.
    "AdminManagedChannels",
    "WebGeneratedCode",
    "QrCode",
    // The one-variant lifecycle surface enum (replaced by the shared
    // CapabilitySurfaceKind vocabulary in ironclaw_host_api).
    "LifecycleExtensionSurfaceKind",
    // The conflated extension `kind` wire string (replaced by
    // runtime + surfaces).
    "isChannelExtensionKind",
    "KIND_LABELS",
    "extension_kind(",
    "wire_kind(",
    // Slack-specific identity resolution (replaced by the generic
    // ProviderIdentityActorResolver parameterized by manifest data).
    "slack_actor_identity",
    "SlackUserIdentityActorResolver",
    "SlackHostBetaLegacySetup",
    "SlackHostBetaActorUserResolver",
    // Residual shims that kept the split-extension presentation alive.
    "is_internal_extension_package_ref",
    "is_webui_v2_llm_config_route_id",
    "SLACK_TOOLS_EXTENSION_ID",
    // Retired browser/API routes. Reborn clients discover channel setup from
    // extension surfaces and group MCP-backed tools by those same surfaces.
    "/api/webchat/v2/channels/connectable",
    "/v2/extensions/mcp",
    // Contract-free / legacy manifest parse paths (one parse entry point
    // remains: ExtensionManifestV2::parse with a contract registry).
    "parse_with_host_api_contracts",
    "parse_with_optional_host_api_contracts",
    "from_toml_with_contracts",
    "LegacyTopLevelCapabilitiesForInstalledSource",
];

/// Retired identities that survive only as *substrings* of sanctioned names:
/// `slack_bot_token` / `slack_signing_secret` are workspace credential
/// handles, so the identity forms are matched exactly. The retired extension
/// `kind` wire VALUES are likewise matched as exact quoted strings: bare
/// `channel`/`mcp` remain legitimate vocabulary (the surface kind and the
/// runtime label), but nothing in Reborn code may compare against the old
/// conflated kind strings.
const RETIRED_IDENTITY_FORMS: &[&str] = &[
    "\"slack_bot\"",
    "'slack_bot'",
    "\"slack_personal\"",
    "'slack_personal'",
    "assets/slack_bot/",
    // The conflated extension `kind` wire values (replaced by
    // runtime + surfaces).
    "\"wasm_channel\"",
    "'wasm_channel'",
    "\"channel_relay\"",
    "'channel_relay'",
    "\"wasm_tool\"",
    "'wasm_tool'",
    "\"mcp_server\"",
    "'mcp_server'",
];

/// Path fragments allowed to reference retired vocabulary.
const SANCTIONED_PATHS: &[&str] = &[
    // v1 → Reborn converter reads v1 domain names by design.
    "crates/ironclaw_reborn_migration/",
    // The v1 gateway is a legacy enclave being strangled wholesale — its
    // static JS still serves the v1 `kind` wire and is not policed
    // term-by-term (same footing as `src/`).
    "crates/ironclaw_gateway/",
    // One-time forward data migrations name what they fold forward.
    "extension_host/extension_installation_store.rs",
    "extension_host/extension_installation_store/",
    "product_auth/durable/",
    "tests/facade_factory/product_auth_migration.rs",
    // This gate names every term on purpose.
    "reborn_retired_taxonomy.rs",
];

fn is_sanctioned(path: &str) -> bool {
    SANCTIONED_PATHS
        .iter()
        .any(|fragment| path.contains(fragment))
}

fn is_reborn_python_scenario(relative: &str, contents: &str) -> bool {
    let file_name = relative.rsplit('/').next().unwrap_or_default();
    let imports_reborn_webui_harness = contents.lines().any(|line| {
        line.trim_start()
            .starts_with("from reborn_webui_harness import")
    });
    let uses_reborn_binary_fixture = contents
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .any(|token| token.starts_with("ironclaw_reborn_") && token.ends_with("_binary"));
    relative.starts_with("tests/e2e/scenarios/")
        && file_name.ends_with(".py")
        && (file_name.starts_with("test_reborn_webui_v2_")
            || imports_reborn_webui_harness
            || uses_reborn_binary_fixture)
}

fn record_hits(relative: &str, contents: &str, hits: &mut Vec<String>) {
    for (line_index, line) in contents.lines().enumerate() {
        for term in RETIRED_TERMS {
            if line.contains(term) {
                hits.push(format!("{relative}:{}: `{term}`", line_index + 1));
            }
        }
        for form in RETIRED_IDENTITY_FORMS {
            if line.contains(form) {
                hits.push(format!("{relative}:{}: `{form}`", line_index + 1));
            }
        }
    }
}

fn scan_dir(root: &Path, dir: &Path, hits: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name == "target" || name == "node_modules" || name == ".git" {
                continue;
            }
            scan_dir(root, &path, hits);
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let is_rust = name.ends_with(".rs");
        let is_frontend = name.ends_with(".ts")
            || name.ends_with(".tsx")
            || name.ends_with(".mts")
            || name.ends_with(".mjs")
            || name.ends_with(".js");
        let is_manifest = name.ends_with(".toml");
        let is_python_scenario =
            relative.starts_with("tests/e2e/scenarios/") && name.ends_with(".py");
        if !(is_rust || is_frontend || is_manifest || is_python_scenario) {
            continue;
        }
        if is_sanctioned(&relative) {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        if is_python_scenario && !is_reborn_python_scenario(&relative, &contents) {
            continue;
        }
        record_hits(&relative, &contents, hits);
    }
}

#[test]
fn reborn_code_never_references_retired_taxonomy() {
    let root = workspace_root();
    let mut hits = Vec::new();
    scan_dir(&root, &root.join("crates"), &mut hits);
    scan_dir(&root, &root.join("tests/integration"), &mut hits);
    scan_dir(&root, &root.join("tests/e2e/scenarios"), &mut hits);
    hits.sort();
    hits.dedup();
    assert!(
        hits.is_empty(),
        "retired NEA-25 taxonomy reintroduced (extension = the product object; \
         channel = a capability surface; runtime is implementation only):\n{}",
        hits.join("\n")
    );
}

#[test]
fn python_scan_follows_reborn_webui_and_harness_boundaries() {
    let root = std::env::temp_dir().join(format!(
        "ironclaw-retired-taxonomy-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos()
    ));
    let scenarios = root.join("tests/e2e/scenarios");
    std::fs::create_dir_all(&scenarios).expect("create Python fixture directory");
    std::fs::write(
        scenarios.join("test_reborn_webui_v2_stale_fixture.py"),
        "fixture = {\"kind\": \"wasm_channel\"}\n",
    )
    .expect("write Reborn Python fixture");
    std::fs::write(
        scenarios.join("test_admin_api.py"),
        "from reborn_webui_harness import reborn_v2_server\nfixture = {\"kind\": \"wasm_channel\"}\n",
    )
    .expect("write Reborn server-harness fixture");
    std::fs::write(
        scenarios.join("test_skills.py"),
        "from reborn_webui_harness import reborn_v2_page\nfixture = {\"kind\": \"wasm_channel\"}\n",
    )
    .expect("write Reborn browser-harness fixture");
    std::fs::write(
        scenarios.join("test_reborn_private_tool_installs.py"),
        "from reborn_webui_harness import reborn_v2_private_installs_yolo_server\nfixture = {\"kind\": \"wasm_channel\"}\n",
    )
    .expect("write alternate Reborn harness fixture");
    std::fs::write(
        scenarios.join("test_reborn_responses_api.py"),
        "def fixture(ironclaw_reborn_openai_compat_binary):\n    value = {\"kind\": \"wasm_channel\"}\n",
    )
    .expect("write standalone Reborn binary fixture");
    for legacy_file in [
        "test_extensions.py",
        "test_reborn_gateway_smoke.py",
        "test_telegram_hot_activation.py",
    ] {
        std::fs::write(
            scenarios.join(legacy_file),
            "fixture = {\"kind\": \"wasm_channel\"}\n",
        )
        .expect("write legacy gateway Python fixture");
    }
    std::fs::write(
        scenarios.join("test_v2_notion_mcp_oauth_flow.py"),
        "# docs: ironclaw_reborn_composition\nfixture = {\"kind\": \"wasm_channel\"}\n",
    )
    .expect("write legacy gateway fixture with a Reborn documentation reference");

    let mut hits = Vec::new();
    scan_dir(&root, &scenarios, &mut hits);
    std::fs::remove_dir_all(&root).expect("remove Python fixture directory");
    hits.sort();

    assert_eq!(
        hits,
        vec![
            "tests/e2e/scenarios/test_admin_api.py:2: `\"wasm_channel\"`",
            "tests/e2e/scenarios/test_reborn_private_tool_installs.py:2: `\"wasm_channel\"`",
            "tests/e2e/scenarios/test_reborn_responses_api.py:2: `\"wasm_channel\"`",
            "tests/e2e/scenarios/test_reborn_webui_v2_stale_fixture.py:1: `\"wasm_channel\"`",
            "tests/e2e/scenarios/test_skills.py:2: `\"wasm_channel\"`",
        ],
        "Reborn WebUI and harness fixtures must be policed while legacy gateway fixtures remain out of scope"
    );
}

#[test]
fn full_reborn_e2e_runs_train_a_contract_gates() {
    let root = workspace_root();
    let fixture_root = std::env::temp_dir().join(format!(
        "ironclaw-reborn-e2e-script-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos()
    ));
    let bin_dir = fixture_root.join("bin");
    let log_path = fixture_root.join("cargo.log");
    std::fs::create_dir_all(&bin_dir).expect("create fake cargo directory");
    let cargo_path = bin_dir.join("cargo");
    std::fs::write(
        &cargo_path,
        "#!/usr/bin/env bash\nprintf '%s\\n' \"$*\" >> \"$IRONCLAW_FAKE_CARGO_LOG\"\n",
    )
    .expect("write fake cargo");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = std::fs::metadata(&cargo_path)
            .expect("stat fake cargo")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&cargo_path, permissions).expect("make fake cargo executable");
    }
    let inherited_path = std::env::var("PATH").unwrap_or_default();
    let output = std::process::Command::new("bash")
        .arg(root.join("scripts/reborn-e2e-rust.sh"))
        .arg("architecture")
        .env("PATH", format!("{}:{inherited_path}", bin_dir.display()))
        .env("IRONCLAW_FAKE_CARGO_LOG", &log_path)
        .output()
        .expect("execute Reborn architecture group with fake cargo");
    assert!(
        output.status.success(),
        "Reborn architecture group must execute successfully with fake cargo: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let invocations = std::fs::read_to_string(&log_path).expect("read fake cargo invocations");
    std::fs::remove_dir_all(&fixture_root).expect("remove fake cargo fixture");
    for required_gate in [
        "test -p ironclaw_extensions --test manifest_v2_contract",
        "test -p ironclaw_product_adapter_registry --test manifest_ingestion",
        "test -p ironclaw_architecture --test reborn_retired_taxonomy",
    ] {
        assert!(
            invocations
                .lines()
                .any(|line| line.starts_with(required_gate)),
            "the architecture group must execute `{required_gate}`; got:\n{invocations}"
        );
    }
}
