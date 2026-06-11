//! End-to-end parser tests for `ironclaw.harness/v1`.

use ironclaw_harness::{HarnessError, parse};

const RED_TEAM: &str = r#"
api_version = "ironclaw.harness/v1"
kind = "Harness"
id = "red-team"
name = "Red Team Operator"
description = "Offensive-security operator persona for authorized engagements"
trust = "user_trusted"

[prompt_overlay]
text_ref = "prompts/red-team-system.md"

[runtime_constraints]
max_profile = "Sandboxed"
require_deployment_mode = ["LocalSingleUser", "EnterpriseDedicated"]
network_mode = "Brokered"

[[required_extensions]]
id = "nmap-wasm"
[[required_extensions]]
id = "nuclei-wasm"

[[required_skills]]
id = "evidence-capture"

[capability_surface]
allow = ["nmap-wasm.scan", "memory.write", "report.render"]
deny = ["shell.run", "process.spawn", "filesystem.write"]

[memory_schema]
findings_root = "/memory/projects/${project}/findings"
runbook_root = "/memory/projects/${project}/runbooks"

[exit_artifacts]
report = "/artifacts/${run}/engagement-report.md"
evidence_bundle = "/artifacts/${run}/evidence.tar.zst"
"#;

#[test]
fn parses_full_manifest() {
    let h = parse(RED_TEAM).expect("manifest parses");
    assert_eq!(h.id, "red-team");
    assert_eq!(h.required_extensions.len(), 2);
    assert_eq!(h.required_skills.len(), 1);
    let constraints = h.runtime_constraints.as_ref().expect("constraints present");
    assert_eq!(constraints.max_profile.as_deref(), Some("Sandboxed"));
    assert_eq!(constraints.require_deployment_mode.len(), 2);
    let memory = h.memory_schema.as_ref().expect("memory schema present");
    assert!(memory.contains_key("findings_root"));
}

#[test]
fn round_trip_is_stable() {
    let first = parse(RED_TEAM).expect("parses");
    let reemitted = toml::to_string(&first).expect("serializes");
    let second = parse(&reemitted).expect("re-parses");
    assert_eq!(first, second, "parse -> emit -> parse must be identical");
}

#[test]
fn rejects_unknown_key() {
    let src = "api_version = \"ironclaw.harness/v1\"\nkind = \"Harness\"\nid = \"x\"\nbogus = 1\n";
    let err = parse(src).expect_err("unknown key rejected");
    assert!(matches!(err, HarnessError::Toml(_)));
}

#[test]
fn rejects_wrong_api_version() {
    let src = "api_version = \"ironclaw.harness/v2\"\nkind = \"Harness\"\nid = \"x\"\n";
    let err = parse(src).expect_err("wrong major rejected");
    assert!(matches!(err, HarnessError::UnsupportedApiVersion { .. }));
}

#[test]
fn rejects_inline_secret_with_path() {
    let src = "api_version = \"ironclaw.harness/v1\"\nkind = \"Harness\"\nid = \"x\"\n\
               [memory_schema]\ntoken = \"sk-proj-abcdef1234567890abcdef1234\"\n";
    let err = parse(src).expect_err("inline secret rejected");
    match err {
        HarnessError::InlineSecret { path, .. } => assert_eq!(path, "memory_schema.token"),
        other => panic!("expected InlineSecret, got {other:?}"),
    }
}

#[test]
fn rejects_invalid_id() {
    let src = "api_version = \"ironclaw.harness/v1\"\nkind = \"Harness\"\nid = \"bad id\"\n";
    let err = parse(src).expect_err("bad id rejected");
    assert!(matches!(err, HarnessError::InvalidIdentifier { .. }));
}
