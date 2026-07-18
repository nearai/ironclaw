use std::{fs, path::Path, process::Command};

fn reborn_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ironclaw-reborn")
}

#[test]
fn extension_search_json_reads_reborn_home_local_dev_packages() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    write_extension_fixture(&reborn_home, "zztest-mcp");

    let json = run_extension_json(&reborn_home, &["search", "zztest", "--json"]);

    assert_eq!(json["phase"], "discovered");
    assert_eq!(json["payload"]["kind"], "extension_search");
    assert_eq!(json["payload"]["count"], 1);
    assert_eq!(
        json["payload"]["extensions"][0]["package_ref"]["id"],
        "zztest-mcp"
    );
}

#[test]
fn extension_search_json_without_query_lists_local_dev_packages() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    write_extension_fixture(&reborn_home, "zztest-alpha");
    write_extension_fixture(&reborn_home, "zztest-beta");

    let json = run_extension_json(&reborn_home, &["search", "--json"]);
    let extensions = json["payload"]["extensions"]
        .as_array()
        .expect("extensions array");
    let ids = extensions
        .iter()
        .filter_map(|extension| extension["package_ref"]["id"].as_str())
        .collect::<Vec<_>>();

    assert_eq!(json["phase"], "discovered");
    assert_eq!(json["payload"]["kind"], "extension_search");
    assert!(ids.contains(&"zztest-alpha"), "ids: {ids:?}");
    assert!(ids.contains(&"zztest-beta"), "ids: {ids:?}");
}

#[test]
fn extension_install_json_uses_reborn_home_without_v1_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let v1_base_dir = temp.path().join("v1-state");
    write_extension_fixture(&reborn_home, "zztest-mcp");

    let output = Command::new(reborn_bin())
        .arg("extension")
        .arg("install")
        .arg("zztest-mcp")
        .arg("--json")
        .env_clear()
        .env("IRONCLAW_DISABLE_OS_KEYCHAIN", "1")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("IRONCLAW_BASE_DIR", &v1_base_dir)
        .output()
        .expect("ironclaw-reborn extension install --json should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    assert_eq!(json["phase"], "installed");
    assert_eq!(json["package_ref"]["id"], "zztest-mcp");
    assert_eq!(json["payload"]["kind"], "extension_install");
    assert_eq!(json["payload"]["installed"], true);
    assert!(
        reborn_home
            .join("local-dev/system/extensions/zztest-mcp/manifest.toml")
            .exists(),
        "extension install should operate inside Reborn home"
    );
    assert!(
        !v1_base_dir.exists(),
        "extension install should not create explicit v1 base directories"
    );
}

#[test]
fn extension_search_human_output_escapes_control_characters() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    write_extension_fixture_with_metadata(
        &reborn_home,
        "zztest-evil",
        "Bad\u{1b}[31mName",
        "Line\rRewrite",
    );

    let output = Command::new(reborn_bin())
        .arg("extension")
        .arg("search")
        .arg("zztest-evil")
        .env_clear()
        .env("IRONCLAW_DISABLE_OS_KEYCHAIN", "1")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn extension search should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains('\u{1b}'), "stdout: {stdout:?}");
    assert!(!stdout.contains('\r'), "stdout: {stdout:?}");
    assert!(stdout.contains("\\u{1b}"), "stdout: {stdout:?}");
    assert!(stdout.contains("\\r"), "stdout: {stdout:?}");
}

#[test]
fn extension_activate_and_remove_json_use_persisted_installation_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    write_extension_fixture(&reborn_home, "zztest-mcp");

    let install = run_extension_json(&reborn_home, &["install", "zztest-mcp", "--json"]);
    assert_eq!(install["phase"], "installed");

    let activate = run_extension_json(&reborn_home, &["activate", "zztest-mcp", "--json"]);
    assert_eq!(activate["phase"], "active");
    assert_eq!(activate["payload"]["kind"], "extension_activate");
    assert_eq!(activate["payload"]["activated"], true);

    let remove = run_extension_json(&reborn_home, &["remove", "zztest-mcp", "--json"]);
    assert_eq!(remove["phase"], "removed");
    assert_eq!(remove["payload"]["kind"], "extension_remove");
    assert_eq!(remove["payload"]["removed"], true);
    assert!(
        !reborn_home
            .join("local-dev/system/extensions/zztest-mcp")
            .exists(),
        "extension remove should delete the installed package files"
    );
}

fn run_extension_json(reborn_home: &Path, args: &[&str]) -> serde_json::Value {
    let output = Command::new(reborn_bin())
        .arg("extension")
        .args(args)
        .env_clear()
        .env("IRONCLAW_DISABLE_OS_KEYCHAIN", "1")
        .env("IRONCLAW_REBORN_HOME", reborn_home)
        .output()
        .expect("ironclaw-reborn extension command should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.trim()).expect("valid JSON")
}

fn write_extension_fixture(reborn_home: &Path, extension_id: &str) {
    write_extension_fixture_with_metadata(
        reborn_home,
        extension_id,
        "GitHub MCP",
        "GitHub MCP helper",
    );
}

fn write_extension_fixture_with_metadata(
    reborn_home: &Path,
    extension_id: &str,
    name: &str,
    description: &str,
) {
    let extension_root = reborn_home
        .join("local-dev/system/extensions")
        .join(extension_id);
    fs::create_dir_all(&extension_root).expect("fixture extension dir");
    let name = toml_basic_string_value(name);
    let description = toml_basic_string_value(description);
    // Filesystem-discovered manifests validate as `InstalledLocal` (#5499),
    // which forbids the legacy top-level `[[capabilities]]` shape — the
    // fixture uses the installed-legal `capability_provider` host_api form.
    fs::write(
        extension_root.join("manifest.toml"),
        format!(
            r#"schema_version = "reborn.extension_manifest.v2"
id = "{extension_id}"
name = "{name}"
version = "0.1.0"
description = "{description}"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "stdio"
command = "zztest-mcp-server"
args = ["--stdio"]

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "{extension_id}.search_issues"
description = "Search GitHub issues"
effects = ["network", "dispatch_capability"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/zztest-mcp/search_issues.input.v1.json"
output_schema_ref = "schemas/zztest-mcp/search_issues.output.v1.json"
prompt_doc_ref = "prompts/zztest-mcp/search_issues.md"
"#
        ),
    )
    .expect("fixture extension manifest");
}

fn toml_basic_string_value(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\r' => escaped.push_str("\\r"),
            '\n' => escaped.push_str("\\n"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}
