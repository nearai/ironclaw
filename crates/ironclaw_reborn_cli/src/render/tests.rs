use std::path::PathBuf;

use super::Renderable;
use crate::dto::{
    CheckCategory, CheckOutcome, ComponentStatus, ConfigEntry, ConfigGetDto, ConfigListDto,
    ConfigValue, DoctorCheck, DoctorDto, DoctorSummary, DriversSnapshot, FilePresence,
    ServiceStateDto, StatusDto,
};

fn render_to_string(dto: &impl Renderable) -> String {
    let mut buf = Vec::new();
    dto.render_text_to(&mut buf).expect("render");
    String::from_utf8(buf).expect("utf8")
}

fn sample_status() -> StatusDto {
    StatusDto {
        version: "0.1.0".to_string(),
        reborn_home: PathBuf::from("/home/user/.ironclaw/reborn"),
        home_source: "default",
        profile: "local-dev".to_string(),
        config_file: FilePresence {
            path: PathBuf::from("/home/user/.ironclaw/reborn/config.toml"),
            present: true,
        },
        providers_file: FilePresence {
            path: PathBuf::from("/home/user/.ironclaw/reborn/providers.json"),
            present: false,
        },
        model_slots: vec!["default".to_string(), "mission".to_string()],
        drivers: DriversSnapshot {
            text_only: ComponentStatus::Initialized,
            planned: ComponentStatus::Initialized,
            subagent_planned: ComponentStatus::Failed {
                reason: "missing loop family".to_string(),
            },
            planned_default_profile: ComponentStatus::Initialized,
        },
        login_link: Some("http://127.0.0.1:3000/login?token=sample-token".to_string()),
        login_note: None,
        service: ServiceStateDto::Running,
    }
}

fn sample_doctor() -> DoctorDto {
    DoctorDto {
        checks: vec![
            DoctorCheck {
                name: "reborn_home".to_string(),
                category: CheckCategory::Core,
                outcome: CheckOutcome::Pass,
                detail: "/home/user/.ironclaw/reborn".to_string(),
            },
            DoctorCheck {
                name: "config_file".to_string(),
                category: CheckCategory::Core,
                outcome: CheckOutcome::Fail,
                detail: "missing".to_string(),
            },
            DoctorCheck {
                name: "text_only_driver".to_string(),
                category: CheckCategory::Drivers,
                outcome: CheckOutcome::Pass,
                detail: "initialized".to_string(),
            },
            DoctorCheck {
                name: "subagent_planned_driver".to_string(),
                category: CheckCategory::Drivers,
                outcome: CheckOutcome::Skip,
                detail: "not configured".to_string(),
            },
        ],
        summary: DoctorSummary {
            pass: 2,
            fail: 1,
            skip: 1,
        },
    }
}

fn sample_config_list() -> ConfigListDto {
    ConfigListDto {
        config_file: PathBuf::from("/home/user/.ironclaw/reborn/config.toml"),
        entries: vec![
            ConfigEntry {
                key: "boot.profile".to_string(),
                value: Some(ConfigValue::String("local-dev".to_string())),
            },
            ConfigEntry {
                key: "identity.tenant".to_string(),
                value: None,
            },
            ConfigEntry {
                key: "runner.heartbeat_interval_secs".to_string(),
                value: Some(ConfigValue::Integer(5)),
            },
        ],
    }
}

#[test]
fn status_json_round_trips() {
    let dto = sample_status();
    let json = serde_json::to_string_pretty(&dto).expect("serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert_eq!(parsed["version"], "0.1.0");
    assert_eq!(parsed["profile"], "local-dev");
    assert_eq!(parsed["config_file"]["present"], true);
    assert_eq!(parsed["providers_file"]["present"], false);
    assert_eq!(parsed["drivers"]["text_only"]["status"], "initialized");
    assert_eq!(parsed["drivers"]["subagent_planned"]["status"], "failed");
    assert_eq!(
        parsed["drivers"]["subagent_planned"]["reason"],
        "missing loop family"
    );
    assert_eq!(parsed["service"], "running");
    // login_link embeds a live bearer token; must never reach JSON output.
    assert!(
        parsed.get("login_link").is_none(),
        "login_link must be skipped in JSON serialization: {json}"
    );
    assert!(
        !json.contains("sample-token"),
        "token leaked into JSON: {json}"
    );
}

#[test]
fn status_render_text_contains_all_fields() {
    let text = render_to_string(&sample_status());
    assert!(text.contains("IronClaw Reborn status"));
    assert!(text.contains("version:"));
    assert!(text.contains("0.1.0"));
    assert!(text.contains("reborn_home:"));
    assert!(text.contains("/home/user/.ironclaw/reborn"));
    assert!(text.contains("home_source:"));
    assert!(text.contains("profile:"));
    assert!(text.contains("local-dev"));
    assert!(text.contains("config_file:"));
    assert!(text.contains("(present)"));
    assert!(text.contains("providers_file:"));
    assert!(text.contains("(absent)"));
    assert!(text.contains("model_slots:"));
    assert!(text.contains("default, mission"));
    assert!(text.contains("service:"));
    assert!(text.contains("running"));
    assert!(text.contains("drivers:"));
    assert!(text.contains("text_only: initialized"));
    assert!(text.contains("planned: initialized"));
    assert!(text.contains("subagent_planned: unavailable (missing loop family)"));
    assert!(text.contains("planned_default_profile: initialized"));
    assert!(text.contains("login_link:"));
    assert!(text.contains("http://127.0.0.1:3000/login?token=sample-token"));
}

#[test]
fn status_render_text_omits_login_link_line_when_absent() {
    let mut status = sample_status();
    status.login_link = None;
    let text = render_to_string(&status);
    assert!(
        !text.contains("login_link:"),
        "no login_link line should be printed when the DTO carries None: {text}"
    );
}

#[test]
fn doctor_json_round_trips() {
    let dto = sample_doctor();
    let json = serde_json::to_string_pretty(&dto).expect("serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert_eq!(parsed["checks"][0]["name"], "reborn_home");
    assert_eq!(parsed["checks"][0]["outcome"], "pass");
    assert_eq!(parsed["checks"][1]["outcome"], "fail");
    assert_eq!(parsed["checks"][3]["outcome"], "skip");
    assert_eq!(parsed["checks"][3]["category"], "drivers");
    assert_eq!(parsed["summary"]["pass"], 2);
    assert_eq!(parsed["summary"]["fail"], 1);
    assert_eq!(parsed["summary"]["skip"], 1);
}

#[test]
fn doctor_render_text_contains_all_three_outcome_icons() {
    let text = render_to_string(&sample_doctor());
    assert!(text.contains('\u{2714}'), "missing pass icon ✔");
    assert!(text.contains('\u{2718}'), "missing fail icon ✘");
    assert!(
        text.contains("- subagent_planned_driver"),
        "missing skip icon -"
    );
    assert!(text.contains("2 passed, 1 failed, 1 skipped"));
}

#[test]
fn config_list_json_round_trips() {
    let dto = sample_config_list();
    let json = serde_json::to_string_pretty(&dto).expect("serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert_eq!(parsed["entries"][0]["key"], "boot.profile");
    assert_eq!(parsed["entries"][0]["value"], "local-dev");
    assert!(parsed["entries"][1]["value"].is_null());
    assert_eq!(parsed["entries"][2]["value"], 5);
}

#[test]
fn config_list_render_text_covers_entries() {
    let text = render_to_string(&sample_config_list());
    assert!(text.contains("IronClaw Reborn config"));
    assert!(text.contains("config.toml"));
    assert!(text.contains("boot.profile"));
    assert!(text.contains("local-dev"));
    assert!(text.contains("identity.tenant"));
    assert!(text.contains("(not set)"));
    assert!(text.contains("runner.heartbeat_interval_secs"));
    assert!(text.contains("5"));
}

#[test]
fn config_value_display() {
    assert_eq!(
        ConfigValue::String("hello".to_string()).to_string(),
        "hello"
    );
    assert_eq!(ConfigValue::Bool(true).to_string(), "true");
    assert_eq!(ConfigValue::Integer(42).to_string(), "42");
    assert_eq!(ConfigValue::Float(1.5).to_string(), "1.5");
    assert_eq!(ConfigValue::Float(5.0).to_string(), "5.0");
    assert_eq!(
        ConfigValue::List(vec!["a".to_string(), "b".to_string()]).to_string(),
        "[a, b]"
    );
}

#[test]
fn text_rendering_replaces_terminal_control_characters() {
    let dto = ConfigGetDto {
        key: "boot.profile".to_string(),
        value: Some(ConfigValue::String(
            "safe\nforged: row\u{1b}[31m".to_string(),
        )),
    };
    assert_eq!(render_to_string(&dto), "safe forged: row [31m\n");

    let mut status = sample_status();
    status.drivers.subagent_planned = ComponentStatus::Failed {
        reason: "failed\nforged\u{1b}[31m".to_string(),
    };
    let text = render_to_string(&status);
    assert!(!text.contains("\nforged"));
    assert!(!text.contains('\u{1b}'));
}

#[test]
fn config_get_json_set_value() {
    let dto = ConfigGetDto {
        key: "boot.profile".to_string(),
        value: Some(ConfigValue::String("local-dev".to_string())),
    };
    let json = serde_json::to_string_pretty(&dto).expect("serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert_eq!(parsed["key"], "boot.profile");
    assert_eq!(parsed["value"], "local-dev");
}

#[test]
fn config_get_json_unset_value() {
    let dto = ConfigGetDto {
        key: "identity.tenant".to_string(),
        value: None,
    };
    let json = serde_json::to_string_pretty(&dto).expect("serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert_eq!(parsed["key"], "identity.tenant");
    assert!(parsed["value"].is_null());
}

#[test]
fn config_get_render_text_set_value() {
    let dto = ConfigGetDto {
        key: "boot.profile".to_string(),
        value: Some(ConfigValue::String("local-dev".to_string())),
    };
    let text = render_to_string(&dto);
    assert!(text.contains("local-dev"));
    assert!(!text.contains("(not set)"));
}

#[test]
fn config_get_render_text_unset_value() {
    let dto = ConfigGetDto {
        key: "identity.tenant".to_string(),
        value: None,
    };
    let text = render_to_string(&dto);
    assert!(text.contains("(not set)"));
}
