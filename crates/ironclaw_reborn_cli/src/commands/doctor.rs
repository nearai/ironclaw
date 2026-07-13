use anyhow::Context as _;
use clap::Args;
use ironclaw_reborn_composition::{
    RebornRuntimeComponentStatus, reborn_runtime_readiness_snapshot,
};
use ironclaw_reborn_config::{RebornConfigFile, RebornDoctorReport};

use crate::context::RebornCliContext;
use crate::dto::{CheckCategory, CheckOutcome, DoctorCheck, DoctorDto, DoctorSummary};
use crate::render::{self, OutputMode, Renderable, terminal_safe_text};
use std::io::Write;

#[derive(Debug, Args)]
pub(crate) struct DoctorCommand {
    /// Output as JSON.
    #[arg(long)]
    json: bool,
}

impl DoctorCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let dto = build_doctor_dto(&context);
        let mode = if self.json {
            OutputMode::Json
        } else {
            OutputMode::Text
        };
        render::output(&dto, mode)
    }
}

fn build_doctor_dto(context: &RebornCliContext) -> DoctorDto {
    let mut checks = Vec::new();

    let report = RebornDoctorReport::from_config(context.boot_config().clone());

    checks.push(DoctorCheck {
        name: "reborn_home".to_string(),
        category: CheckCategory::Core,
        outcome: if report.home_path().is_dir() {
            CheckOutcome::Pass
        } else {
            CheckOutcome::Fail
        },
        detail: format!(
            "{} ({})",
            report.home_path().display(),
            report.home_source_label()
        ),
    });

    checks.push(DoctorCheck {
        name: "profile".to_string(),
        category: CheckCategory::Core,
        outcome: CheckOutcome::Pass,
        detail: report.profile().to_string(),
    });

    checks.push(DoctorCheck {
        name: "v1_state".to_string(),
        category: CheckCategory::Core,
        outcome: CheckOutcome::Pass,
        detail: report.v1_state().to_string(),
    });

    checks.push(migration_check(context));

    let config_path = context.boot_config().home().config_file_path();
    checks.push(check_config_file(&config_path));

    let providers_path = context.boot_config().home().providers_file_path();
    checks.push(check_providers_file(&providers_path));

    let snapshot = reborn_runtime_readiness_snapshot();

    checks.push(driver_check("text_only_driver", &snapshot.text_only_driver));
    checks.push(driver_check("planned_driver", &snapshot.planned_driver));
    checks.push(driver_check(
        "subagent_planned_driver",
        &snapshot.subagent_planned_driver,
    ));
    checks.push(driver_check(
        "planned_default_profile",
        &snapshot.planned_default_profile,
    ));

    let (pass, fail, skip) = checks
        .iter()
        .fold((0, 0, 0), |counts, check| match check.outcome {
            CheckOutcome::Pass => (counts.0 + 1, counts.1, counts.2),
            CheckOutcome::Fail => (counts.0, counts.1 + 1, counts.2),
            CheckOutcome::Skip => (counts.0, counts.1, counts.2 + 1),
        });

    DoctorDto {
        checks,
        summary: DoctorSummary { pass, fail, skip },
    }
}

fn check_config_file(path: &std::path::Path) -> DoctorCheck {
    match RebornConfigFile::load(path) {
        Ok(Some(_)) => DoctorCheck {
            name: "config_file".to_string(),
            category: CheckCategory::Core,
            outcome: CheckOutcome::Pass,
            detail: "valid".to_string(),
        },
        Ok(None) => DoctorCheck {
            name: "config_file".to_string(),
            category: CheckCategory::Core,
            outcome: CheckOutcome::Skip,
            detail: "absent (using defaults)".to_string(),
        },
        Err(error) => DoctorCheck {
            name: "config_file".to_string(),
            category: CheckCategory::Core,
            outcome: CheckOutcome::Fail,
            detail: error.to_string(),
        },
    }
}

#[cfg(feature = "root-llm-provider")]
fn check_providers_file(path: &std::path::Path) -> DoctorCheck {
    match std::fs::read_to_string(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => DoctorCheck {
            name: "providers_file".to_string(),
            category: CheckCategory::Core,
            outcome: CheckOutcome::Skip,
            detail: "absent (using built-in providers)".to_string(),
        },
        Err(error) => DoctorCheck {
            name: "providers_file".to_string(),
            category: CheckCategory::Core,
            outcome: CheckOutcome::Fail,
            detail: format!("failed to read provider catalog: {error}"),
        },
        Ok(contents) => {
            match ironclaw_reborn_composition::validate_reborn_provider_catalog_contents(&contents)
            {
                Ok(()) => DoctorCheck {
                    name: "providers_file".to_string(),
                    category: CheckCategory::Core,
                    outcome: CheckOutcome::Pass,
                    detail: "valid provider catalog".to_string(),
                },
                Err(error) => DoctorCheck {
                    name: "providers_file".to_string(),
                    category: CheckCategory::Core,
                    outcome: CheckOutcome::Fail,
                    detail: format!("invalid provider catalog: {error}"),
                },
            }
        }
    }
}

#[cfg(not(feature = "root-llm-provider"))]
fn check_providers_file(_path: &std::path::Path) -> DoctorCheck {
    DoctorCheck {
        name: "providers_file".to_string(),
        category: CheckCategory::Core,
        outcome: CheckOutcome::Skip,
        detail: "root LLM provider support not compiled".to_string(),
    }
}

fn driver_check(name: &str, status: &RebornRuntimeComponentStatus) -> DoctorCheck {
    let (outcome, detail) = match status {
        RebornRuntimeComponentStatus::Initialized => {
            (CheckOutcome::Pass, "initialized".to_string())
        }
        RebornRuntimeComponentStatus::Failed(reason) => {
            (CheckOutcome::Fail, format!("unavailable: {reason}"))
        }
    };
    DoctorCheck {
        name: name.to_string(),
        category: CheckCategory::Drivers,
        outcome,
        detail,
    }
}

impl Renderable for DoctorDto {
    fn render_text_to(&self, w: &mut impl Write) -> std::io::Result<()> {
        writeln!(w, "IronClaw Reborn doctor")?;
        writeln!(w)?;
        let mut current_category: Option<CheckCategory> = None;
        for check in &self.checks {
            if current_category != Some(check.category) {
                current_category = Some(check.category);
                let label = match check.category {
                    CheckCategory::Core => "Core",
                    CheckCategory::Drivers => "Drivers",
                };
                writeln!(w, "  {label}")?;
            }
            let icon = match check.outcome {
                CheckOutcome::Pass => "\u{2714}",
                CheckOutcome::Fail => "\u{2718}",
                CheckOutcome::Skip => "-",
            };
            if check.name == "v1_migration_state" {
                writeln!(
                    w,
                    "  {icon} {}: {}",
                    terminal_safe_text(&check.name),
                    terminal_safe_text(&check.detail)
                )?;
            } else {
                writeln!(
                    w,
                    "  {icon} {:<28} {}",
                    terminal_safe_text(&check.name),
                    terminal_safe_text(&check.detail)
                )?;
            }
        }
        writeln!(w)?;
        writeln!(
            w,
            "{} passed, {} failed, {} skipped",
            self.summary.pass, self.summary.fail, self.summary.skip,
        )?;
        Ok(())
    }
}

fn migration_check(context: &RebornCliContext) -> DoctorCheck {
    let detail = match migration_state(context) {
        Ok(Some(detail)) => detail,
        Ok(None) if context.v1_migration_source_candidate().is_some() => "available".to_string(),
        Ok(None) => "not_detected".to_string(),
        Err(error) => format!("invalid: {error}"),
    };
    let outcome = match detail
        .split_once(':')
        .map_or(detail.as_str(), |(state, _)| state)
    {
        "invalid" | "applying" | "failed" | "applied" | "verifying" => CheckOutcome::Fail,
        "not_detected" | "available" | "planned" => CheckOutcome::Skip,
        "verified" => CheckOutcome::Pass,
        _ => CheckOutcome::Fail,
    };
    DoctorCheck {
        name: "v1_migration_state".to_string(),
        category: CheckCategory::Core,
        outcome,
        detail,
    }
}

fn migration_state(context: &RebornCliContext) -> anyhow::Result<Option<String>> {
    match crate::commands::migrate::read_activation_state_status(context) {
        Ok(Some(status)) => return Ok(Some(status.as_str().to_string())),
        Err(error) => {
            return Err(error).context("failed to inspect target migration quarantine state");
        }
        Ok(None) => {}
    }

    let marker = crate::commands::onboard::onboarding_marker_path(context.boot_config().home());
    let body = match std::fs::read_to_string(&marker) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to read onboarding marker at {}", marker.display())
            });
        }
    };
    let document: serde_json::Value = serde_json::from_str(&body)
        .with_context(|| format!("invalid onboarding marker at {}", marker.display()))?;
    let recorded = document
        .pointer("/v1_migration/state")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    let Some(recorded) = recorded else {
        return Ok(None);
    };
    let Some(manifest_path) = document
        .pointer("/v1_migration/manifest")
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(Some(recorded));
    };
    Ok(status_from_document(std::path::Path::new(manifest_path))?
        .map(|status| status.as_str().to_string())
        .or(Some(recorded)))
}

fn status_from_document(
    path: &std::path::Path,
) -> anyhow::Result<Option<crate::commands::migrate::MigrationLifecycleStatus>> {
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read migration manifest at {}", path.display()))?;
    let document = serde_json::from_str::<serde_json::Value>(&body)
        .with_context(|| format!("invalid migration manifest at {}", path.display()))?;
    document
        .get("status")
        .and_then(serde_json::Value::as_str)
        .map(crate::commands::migrate::MigrationLifecycleStatus::parse)
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::RebornCliContext;

    #[test]
    fn doctor_dto_builds_with_defaults() {
        let (_tmp, context) = RebornCliContext::test_context();
        let dto = build_doctor_dto(&context);
        assert!(!dto.checks.is_empty());
        assert_eq!(
            dto.summary.pass + dto.summary.fail + dto.summary.skip,
            dto.checks.len()
        );
    }

    #[test]
    fn doctor_has_core_and_driver_checks() {
        let (_tmp, context) = RebornCliContext::test_context();
        let dto = build_doctor_dto(&context);
        assert!(dto.checks.iter().any(|c| c.category == CheckCategory::Core));
        assert!(
            dto.checks
                .iter()
                .any(|c| c.category == CheckCategory::Drivers)
        );
    }

    #[test]
    fn doctor_fails_quarantined_migration_states() {
        let (_tmp, context) = RebornCliContext::test_context();
        let marker = context
            .boot_config()
            .home()
            .path()
            .join(crate::commands::migrate::MIGRATION_STATE_MARKER_FILE);
        std::fs::create_dir_all(context.boot_config().home().path()).expect("create home");

        for status in ["applying", "applied", "verifying"] {
            std::fs::write(
                &marker,
                serde_json::json!({
                    "schema_version": "ironclaw.reborn.migration-state/v1",
                    "migration_protocol_version": 1,
                    "release_version": env!("CARGO_PKG_VERSION"),
                    "status": status,
                })
                .to_string(),
            )
            .expect("write marker");
            let check = migration_check(&context);
            assert_eq!(check.outcome, CheckOutcome::Fail, "status {status}");
        }
    }

    #[test]
    fn doctor_reports_corrupt_onboarding_migration_state() {
        let (_tmp, context) = RebornCliContext::test_context();
        let marker = crate::commands::onboard::onboarding_marker_path(context.boot_config().home());
        std::fs::create_dir_all(context.boot_config().home().path()).expect("create home");
        std::fs::write(&marker, "not-json").expect("write corrupt marker");

        let check = migration_check(&context);

        assert_eq!(check.outcome, CheckOutcome::Fail);
        assert!(check.detail.contains("invalid"), "detail: {}", check.detail);
        assert!(
            check.detail.contains("onboarding"),
            "detail: {}",
            check.detail
        );
    }

    #[test]
    fn doctor_reports_missing_recorded_migration_manifest() {
        let (_tmp, context) = RebornCliContext::test_context();
        let marker = crate::commands::onboard::onboarding_marker_path(context.boot_config().home());
        let missing_manifest = context
            .boot_config()
            .home()
            .path()
            .join("missing-manifest.json");
        std::fs::create_dir_all(context.boot_config().home().path()).expect("create home");
        std::fs::write(
            &marker,
            serde_json::json!({
                "v1_migration": {
                    "state": "planned",
                    "manifest": missing_manifest,
                }
            })
            .to_string(),
        )
        .expect("write marker");

        let check = migration_check(&context);

        assert_eq!(check.outcome, CheckOutcome::Fail);
        assert!(check.detail.contains("invalid"), "detail: {}", check.detail);
        assert!(
            check.detail.contains("manifest"),
            "detail: {}",
            check.detail
        );
    }

    #[test]
    fn doctor_config_file_absent_is_skip() {
        let check = check_config_file(std::path::Path::new("/nonexistent/config.toml"));
        assert_eq!(check.outcome, CheckOutcome::Skip);
    }

    #[test]
    fn doctor_providers_file_absent_is_skip() {
        let check = check_providers_file(std::path::Path::new("/nonexistent/providers.json"));
        assert_eq!(check.outcome, CheckOutcome::Skip);
    }

    #[test]
    fn doctor_valid_config_file_is_pass() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "api_version = \"ironclaw.runtime/v1\"\n").expect("write");
        let check = check_config_file(&path);
        assert_eq!(check.outcome, CheckOutcome::Pass);
    }

    #[test]
    fn doctor_invalid_config_file_is_fail() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "not valid { toml").expect("write");
        let check = check_config_file(&path);
        assert_eq!(check.outcome, CheckOutcome::Fail);
    }

    #[test]
    #[cfg(feature = "root-llm-provider")]
    fn doctor_valid_providers_file_is_pass() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("providers.json");
        std::fs::write(&path, "[]").expect("write");
        let check = check_providers_file(&path);
        assert_eq!(check.outcome, CheckOutcome::Pass);
    }

    #[test]
    #[cfg(feature = "root-llm-provider")]
    fn doctor_invalid_providers_file_is_fail() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("providers.json");
        std::fs::write(&path, "not json").expect("write");
        let check = check_providers_file(&path);
        assert_eq!(check.outcome, CheckOutcome::Fail);
    }

    #[test]
    #[cfg(feature = "root-llm-provider")]
    fn doctor_well_formed_but_invalid_providers_catalog_is_fail() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("providers.json");
        std::fs::write(&path, "{}").expect("write");
        let check = check_providers_file(&path);
        assert_eq!(check.outcome, CheckOutcome::Fail);
    }

    #[cfg(unix)]
    #[cfg(feature = "root-llm-provider")]
    #[test]
    fn doctor_unreadable_providers_file_is_fail() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("providers.json");
        std::fs::create_dir(&path).expect("create directory at providers path");
        let check = check_providers_file(&path);
        assert_eq!(check.outcome, CheckOutcome::Fail);
        assert!(check.detail.contains("failed to read"));
    }

    #[test]
    fn driver_check_failed_status_produces_fail_outcome() {
        let status = RebornRuntimeComponentStatus::Failed("timeout".to_string());
        let check = driver_check("test_driver", &status);
        assert_eq!(check.outcome, CheckOutcome::Fail);
        assert_eq!(check.category, CheckCategory::Drivers);
        assert_eq!(check.name, "test_driver");
        assert!(
            check.detail.contains("unavailable: timeout"),
            "detail should contain reason: {}",
            check.detail
        );
    }
}
