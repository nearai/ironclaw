use serde::Serialize;

use crate::dto::{
    CheckCategory, CheckOutcome, ComponentStatus, ConfigGetDto, ConfigListDto, DoctorDto, StatusDto,
};

#[derive(Debug, Clone, Copy, Default)]
pub(crate) enum OutputMode {
    #[default]
    Text,
    Json,
}

pub(crate) trait Renderable: Serialize {
    fn render_text(&self);
}

pub(crate) fn output(dto: &impl Renderable, mode: OutputMode) -> anyhow::Result<()> {
    match mode {
        OutputMode::Text => {
            dto.render_text();
            Ok(())
        }
        OutputMode::Json => {
            println!("{}", serde_json::to_string_pretty(dto)?);
            Ok(())
        }
    }
}

// ─── Status ────────────────────────────────────────────────────────────────

impl Renderable for StatusDto {
    fn render_text(&self) {
        println!("IronClaw Reborn status");
        println!();
        kv("version", &self.version);
        kv("reborn_home", &self.reborn_home.display().to_string());
        kv("home_source", self.home_source);
        kv("profile", &self.profile);
        kv(
            "config_file",
            &format!(
                "{} ({})",
                self.config_file.path.display(),
                if self.config_file.present {
                    "present"
                } else {
                    "absent"
                }
            ),
        );
        kv(
            "providers_file",
            &format!(
                "{} ({})",
                self.providers_file.path.display(),
                if self.providers_file.present {
                    "present"
                } else {
                    "absent"
                }
            ),
        );
        kv("model_slots", &self.model_slots.join(", "));
        println!();
        println!("drivers:");
        driver_line("  text_only", &self.drivers.text_only);
        driver_line("  planned", &self.drivers.planned);
        driver_line("  subagent_planned", &self.drivers.subagent_planned);
        driver_line(
            "  planned_default_profile",
            &self.drivers.planned_default_profile,
        );
    }
}

fn driver_line(label: &str, status: &ComponentStatus) {
    match status {
        ComponentStatus::Initialized => println!("{label}: initialized"),
        ComponentStatus::Failed { reason } => println!("{label}: unavailable ({reason})"),
    }
}

// ─── Doctor ────────────────────────────────────────────────────────────────

impl Renderable for DoctorDto {
    fn render_text(&self) {
        println!("IronClaw Reborn doctor");
        println!();

        let mut current_category: Option<CheckCategory> = None;
        for check in &self.checks {
            if current_category != Some(check.category) {
                current_category = Some(check.category);
                let label = match check.category {
                    CheckCategory::Core => "Core",
                    CheckCategory::Drivers => "Drivers",
                };
                println!("  {label}");
            }
            let icon = match check.outcome {
                CheckOutcome::Pass => "\u{2714}",
                CheckOutcome::Fail => "\u{2718}",
                CheckOutcome::Skip => "-",
            };
            println!("  {icon} {:<28} {}", check.name, check.detail);
        }

        println!();
        println!(
            "{} passed, {} failed, {} skipped",
            self.summary.pass, self.summary.fail, self.summary.skip,
        );
    }
}

// ─── Config ────────────────────────────────────────────────────────────────

impl Renderable for ConfigListDto {
    fn render_text(&self) {
        println!("IronClaw Reborn config ({})", self.config_file.display());
        println!();
        for entry in &self.entries {
            match &entry.value {
                Some(value) => println!("{:<44} {value}", entry.key),
                None => println!("{:<44} (not set)", entry.key),
            }
        }
    }
}

impl Renderable for ConfigGetDto {
    fn render_text(&self) {
        match &self.value {
            Some(value) => println!("{value}"),
            None => println!("(not set)"),
        }
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────────

fn kv(key: &str, value: &str) {
    println!("{:<20} {value}", format!("{key}:"));
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::dto::{
        CheckCategory, CheckOutcome, ComponentStatus, ConfigEntry, ConfigGetDto, ConfigListDto,
        ConfigValue, DoctorCheck, DoctorDto, DoctorSummary, DriversSnapshot, FilePresence,
        StatusDto,
    };

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
                    outcome: CheckOutcome::Pass,
                    detail: "valid".to_string(),
                },
                DoctorCheck {
                    name: "text_only_driver".to_string(),
                    category: CheckCategory::Drivers,
                    outcome: CheckOutcome::Pass,
                    detail: "initialized".to_string(),
                },
            ],
            summary: DoctorSummary {
                pass: 3,
                fail: 0,
                skip: 0,
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
    }

    #[test]
    fn doctor_json_round_trips() {
        let dto = sample_doctor();
        let json = serde_json::to_string_pretty(&dto).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["checks"][0]["name"], "reborn_home");
        assert_eq!(parsed["checks"][0]["outcome"], "pass");
        assert_eq!(parsed["checks"][2]["category"], "drivers");
        assert_eq!(parsed["summary"]["pass"], 3);
        assert_eq!(parsed["summary"]["fail"], 0);
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
    fn config_value_display() {
        assert_eq!(
            ConfigValue::String("hello".to_string()).to_string(),
            "hello"
        );
        assert_eq!(ConfigValue::Bool(true).to_string(), "true");
        assert_eq!(ConfigValue::Integer(42).to_string(), "42");
        assert_eq!(ConfigValue::Float(1.5).to_string(), "1.5");
        assert_eq!(
            ConfigValue::List(vec!["a".to_string(), "b".to_string()]).to_string(),
            "[a, b]"
        );
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
}
