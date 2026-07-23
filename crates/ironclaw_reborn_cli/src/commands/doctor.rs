use clap::Args;
use ironclaw_reborn_composition::{
    RebornBuildError, RebornRuntimeComponentStatus, build_reborn_services,
    reborn_runtime_readiness_snapshot,
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

    /// Open and initialize the configured storage and secrets backends.
    /// This may create or migrate Reborn-owned local state.
    #[arg(long)]
    live: bool,
}

impl DoctorCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let dto = assemble_doctor_dto(&context, self.live);
        let mode = if self.json {
            OutputMode::Json
        } else {
            OutputMode::Text
        };
        render::output(&dto, mode)
    }
}

fn assemble_doctor_dto(context: &RebornCliContext, live: bool) -> DoctorDto {
    let config_path = context.boot_config().home().config_file_path();
    let loaded_config = LoadedDoctorConfig::load(&config_path);
    let mut dto = build_doctor_dto(context, &loaded_config);
    dto.checks
        .push(check_llm_readiness(context, &loaded_config));
    if live {
        dto.checks.extend(check_live_dependencies(context));
    } else {
        dto.checks.extend(skipped_live_dependency_checks());
    }
    dto.checks.extend(build_driver_checks());
    refresh_summary(&mut dto);
    dto
}

fn build_doctor_dto(context: &RebornCliContext, loaded_config: &LoadedDoctorConfig) -> DoctorDto {
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

    checks.push(check_config_file(loaded_config));

    let providers_path = context.boot_config().home().providers_file_path();
    checks.push(check_providers_file(&providers_path));

    DoctorDto {
        checks,
        summary: DoctorSummary {
            pass: 0,
            fail: 0,
            skip: 0,
        },
    }
}

fn refresh_summary(dto: &mut DoctorDto) {
    let (pass, fail, skip) =
        dto.checks
            .iter()
            .fold((0, 0, 0), |counts, check| match check.outcome {
                CheckOutcome::Pass => (counts.0 + 1, counts.1, counts.2),
                CheckOutcome::Fail => (counts.0, counts.1 + 1, counts.2),
                CheckOutcome::Skip => (counts.0, counts.1, counts.2 + 1),
            });
    dto.summary = DoctorSummary { pass, fail, skip };
}

fn build_driver_checks() -> Vec<DoctorCheck> {
    let snapshot = reborn_runtime_readiness_snapshot();
    vec![
        driver_check("text_only_driver", &snapshot.text_only_driver),
        driver_check("planned_driver", &snapshot.planned_driver),
        driver_check("subagent_planned_driver", &snapshot.subagent_planned_driver),
        driver_check("planned_default_profile", &snapshot.planned_default_profile),
    ]
}

fn check_llm_readiness(
    context: &RebornCliContext,
    loaded_config: &LoadedDoctorConfig,
) -> DoctorCheck {
    let config = match loaded_config {
        LoadedDoctorConfig::Loaded(config) => config.as_ref(),
        LoadedDoctorConfig::Invalid(error) => {
            return dependency_check(
                "llm_provider",
                CheckOutcome::Fail,
                format!("cannot resolve provider until config is valid: {error}"),
            );
        }
    };
    match ironclaw_reborn_composition::resolve_reborn_runtime_llm(
        context.boot_config(),
        config.as_ref(),
    ) {
        Ok(Some(llm)) => dependency_check(
            "llm_provider",
            CheckOutcome::Pass,
            format!(
                "{} / {} credentials resolved",
                llm.provider_id(),
                llm.model()
            ),
        ),
        Ok(None) => dependency_check(
            "llm_provider",
            CheckOutcome::Fail,
            "no default provider is configured; run `ironclaw models set-provider`".to_string(),
        ),
        Err(error) => dependency_check(
            "llm_provider",
            CheckOutcome::Fail,
            format!("provider or credentials are not ready: {error}"),
        ),
    }
}

fn skipped_live_dependency_checks() -> Vec<DoctorCheck> {
    ["storage_backend", "secrets_store", "runtime_wiring"]
        .into_iter()
        .map(|name| {
            dependency_check(
                name,
                CheckOutcome::Skip,
                "not probed (use `doctor --live`)".to_string(),
            )
        })
        .collect()
}

fn check_live_dependencies(context: &RebornCliContext) -> Vec<DoctorCheck> {
    let input = match crate::runtime::build_services_input_with_options(
        context.boot_config(),
        crate::runtime::RuntimeInputCaller::Run,
        crate::runtime::RuntimeInputOptions::default(),
    ) {
        Ok(input) => input.services_input,
        Err(error) => {
            return vec![
                dependency_check(
                    "storage_backend",
                    CheckOutcome::Fail,
                    format!("runtime storage configuration is not ready: {error}"),
                ),
                dependency_check(
                    "secrets_store",
                    CheckOutcome::Skip,
                    "not probed because runtime storage configuration failed".to_string(),
                ),
                dependency_check(
                    "runtime_wiring",
                    CheckOutcome::Skip,
                    "not probed because runtime storage configuration failed".to_string(),
                ),
            ];
        }
    };

    let result = crate::runtime::block_on_cli(async move {
        Ok::<_, anyhow::Error>(build_reborn_services(input).await)
    });
    match result {
        Ok(Ok(_services)) => vec![
            dependency_check(
                "storage_backend",
                CheckOutcome::Pass,
                "opened and initialized through production composition".to_string(),
            ),
            dependency_check(
                "secrets_store",
                CheckOutcome::Pass,
                "initialized through production composition".to_string(),
            ),
            dependency_check(
                "runtime_wiring",
                CheckOutcome::Pass,
                "production composition validated".to_string(),
            ),
        ],
        Ok(Err(error)) => classify_live_build_error(error),
        Err(error) => vec![
            dependency_check(
                "storage_backend",
                CheckOutcome::Fail,
                format!("dependency probe could not run: {error}"),
            ),
            dependency_check(
                "secrets_store",
                CheckOutcome::Skip,
                "not probed because the dependency probe could not run".to_string(),
            ),
            dependency_check(
                "runtime_wiring",
                CheckOutcome::Skip,
                "not probed because the dependency probe could not run".to_string(),
            ),
        ],
    }
}

fn classify_live_build_error(error: RebornBuildError) -> Vec<DoctorCheck> {
    let detail = error.to_string();

    enum FailureKind {
        Secret,
        Storage,
        RuntimeWiring,
    }

    let kind = match &error {
        RebornBuildError::MissingSecretMasterKey | RebornBuildError::Secret(_) => {
            FailureKind::Secret
        }
        RebornBuildError::MissingProductionTrustPolicy
        | RebornBuildError::MissingRuntimePolicy
        | RebornBuildError::EmptyProductionTrustPolicy
        | RebornBuildError::PlannedRunProfileResolver { .. }
        | RebornBuildError::ProductionWiring { .. }
        | RebornBuildError::InvalidConfig { .. }
        | RebornBuildError::HostRuntime(_)
        | RebornBuildError::Resource(_)
        | RebornBuildError::RunState(_)
        | RebornBuildError::CapabilityLease(_)
        | RebornBuildError::Turn(_)
        | RebornBuildError::Mount(_) => FailureKind::RuntimeWiring,
        RebornBuildError::MissingDatabaseHandle { .. }
        | RebornBuildError::EventStore(_)
        | RebornBuildError::Filesystem(_) => FailureKind::Storage,
    };

    match kind {
        FailureKind::Secret => vec![
            dependency_check(
                "storage_backend",
                CheckOutcome::Skip,
                "storage opened far enough to reach secrets initialization".to_string(),
            ),
            dependency_check("secrets_store", CheckOutcome::Fail, detail),
            dependency_check(
                "runtime_wiring",
                CheckOutcome::Skip,
                "not probed because secrets initialization failed".to_string(),
            ),
        ],
        FailureKind::Storage => vec![
            dependency_check("storage_backend", CheckOutcome::Fail, detail),
            dependency_check(
                "secrets_store",
                CheckOutcome::Skip,
                "not probed because storage initialization failed".to_string(),
            ),
            dependency_check(
                "runtime_wiring",
                CheckOutcome::Skip,
                "not probed because storage initialization failed".to_string(),
            ),
        ],
        FailureKind::RuntimeWiring => vec![
            dependency_check(
                "storage_backend",
                CheckOutcome::Skip,
                "storage opened far enough to reach runtime wiring".to_string(),
            ),
            dependency_check(
                "secrets_store",
                CheckOutcome::Skip,
                "secrets opened far enough to reach runtime wiring".to_string(),
            ),
            dependency_check("runtime_wiring", CheckOutcome::Fail, detail),
        ],
    }
}

fn dependency_check(name: &str, outcome: CheckOutcome, detail: String) -> DoctorCheck {
    DoctorCheck {
        name: name.to_string(),
        category: CheckCategory::Dependencies,
        outcome,
        detail,
    }
}

enum LoadedDoctorConfig {
    Loaded(Box<Option<RebornConfigFile>>),
    Invalid(String),
}

impl LoadedDoctorConfig {
    fn load(path: &std::path::Path) -> Self {
        match RebornConfigFile::load(path) {
            Ok(config) => Self::Loaded(Box::new(config)),
            Err(error) => Self::Invalid(error.to_string()),
        }
    }
}

fn check_config_file(loaded_config: &LoadedDoctorConfig) -> DoctorCheck {
    match loaded_config {
        LoadedDoctorConfig::Loaded(config) if config.is_some() => DoctorCheck {
            name: "config_file".to_string(),
            category: CheckCategory::Core,
            outcome: CheckOutcome::Pass,
            detail: "valid".to_string(),
        },
        LoadedDoctorConfig::Loaded(_) => DoctorCheck {
            name: "config_file".to_string(),
            category: CheckCategory::Core,
            outcome: CheckOutcome::Skip,
            detail: "absent (using defaults)".to_string(),
        },
        LoadedDoctorConfig::Invalid(error) => DoctorCheck {
            name: "config_file".to_string(),
            category: CheckCategory::Core,
            outcome: CheckOutcome::Fail,
            detail: error.clone(),
        },
    }
}

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
                    CheckCategory::Dependencies => "Dependencies",
                    CheckCategory::Drivers => "Drivers",
                };
                writeln!(w, "  {label}")?;
            }
            let icon = match check.outcome {
                CheckOutcome::Pass => "\u{2714}",
                CheckOutcome::Fail => "\u{2718}",
                CheckOutcome::Skip => "-",
            };
            writeln!(
                w,
                "  {icon} {:<28} {}",
                terminal_safe_text(&check.name),
                terminal_safe_text(&check.detail)
            )?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::RebornCliContext;

    #[test]
    fn doctor_dto_builds_with_defaults() {
        let (_tmp, context) = RebornCliContext::test_context();
        let dto = assemble_doctor_dto(&context, false);
        assert!(!dto.checks.is_empty());
        assert_eq!(
            dto.summary.pass + dto.summary.fail + dto.summary.skip,
            dto.checks.len()
        );
    }

    #[test]
    fn doctor_has_core_dependency_and_driver_checks() {
        let (_tmp, context) = RebornCliContext::test_context();
        let dto = assemble_doctor_dto(&context, false);
        assert!(dto.checks.iter().any(|c| c.category == CheckCategory::Core));
        assert!(
            dto.checks
                .iter()
                .any(|c| c.category == CheckCategory::Dependencies)
        );
        assert!(
            dto.checks
                .iter()
                .any(|c| c.category == CheckCategory::Drivers)
        );
    }

    #[test]
    fn secret_build_failure_is_attributed_to_secrets_store() {
        let checks = classify_live_build_error(RebornBuildError::MissingSecretMasterKey);
        assert_eq!(checks[0].name, "storage_backend");
        assert_eq!(checks[0].outcome, CheckOutcome::Skip);
        assert_eq!(checks[1].name, "secrets_store");
        assert_eq!(checks[1].outcome, CheckOutcome::Fail);
        assert_eq!(checks[2].name, "runtime_wiring");
        assert_eq!(checks[2].outcome, CheckOutcome::Skip);
    }

    #[test]
    fn database_build_failure_is_attributed_to_storage_backend() {
        let checks = classify_live_build_error(RebornBuildError::MissingDatabaseHandle {
            backend: "postgres",
        });
        assert_eq!(checks[0].name, "storage_backend");
        assert_eq!(checks[0].outcome, CheckOutcome::Fail);
        assert_eq!(checks[1].name, "secrets_store");
        assert_eq!(checks[1].outcome, CheckOutcome::Skip);
        assert_eq!(checks[2].name, "runtime_wiring");
        assert_eq!(checks[2].outcome, CheckOutcome::Skip);
    }

    #[test]
    fn wiring_build_failure_is_attributed_to_runtime_wiring() {
        let checks = classify_live_build_error(RebornBuildError::MissingProductionTrustPolicy);
        assert_eq!(checks[0].name, "storage_backend");
        assert_eq!(checks[0].outcome, CheckOutcome::Skip);
        assert_eq!(checks[1].name, "secrets_store");
        assert_eq!(checks[1].outcome, CheckOutcome::Skip);
        assert_eq!(checks[2].name, "runtime_wiring");
        assert_eq!(checks[2].outcome, CheckOutcome::Fail);
    }

    #[test]
    fn doctor_config_file_absent_is_skip() {
        let config = LoadedDoctorConfig::load(std::path::Path::new("/nonexistent/config.toml"));
        let check = check_config_file(&config);
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
        let config = LoadedDoctorConfig::load(&path);
        let check = check_config_file(&config);
        assert_eq!(check.outcome, CheckOutcome::Pass);
    }

    #[test]
    fn doctor_invalid_config_file_is_fail() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "not valid { toml").expect("write");
        let config = LoadedDoctorConfig::load(&path);
        let check = check_config_file(&config);
        assert_eq!(check.outcome, CheckOutcome::Fail);
    }

    #[test]
    fn doctor_reuses_loaded_config_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "api_version = \"ironclaw.runtime/v1\"\n").expect("write");
        let config = LoadedDoctorConfig::load(&path);

        std::fs::remove_file(path).expect("remove config after loading");

        assert_eq!(check_config_file(&config).outcome, CheckOutcome::Pass);
    }

    #[test]
    fn doctor_valid_providers_file_is_pass() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("providers.json");
        std::fs::write(&path, "[]").expect("write");
        let check = check_providers_file(&path);
        assert_eq!(check.outcome, CheckOutcome::Pass);
    }

    #[test]
    fn doctor_invalid_providers_file_is_fail() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("providers.json");
        std::fs::write(&path, "not json").expect("write");
        let check = check_providers_file(&path);
        assert_eq!(check.outcome, CheckOutcome::Fail);
    }

    #[test]
    fn doctor_well_formed_but_invalid_providers_catalog_is_fail() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("providers.json");
        std::fs::write(&path, "{}").expect("write");
        let check = check_providers_file(&path);
        assert_eq!(check.outcome, CheckOutcome::Fail);
    }

    #[cfg(unix)]
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
