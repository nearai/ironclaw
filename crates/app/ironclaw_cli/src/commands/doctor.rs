use clap::Args;
use ironclaw_composition::{RebornRuntimeComponentStatus, reborn_runtime_readiness_snapshot};
use ironclaw_config::{RebornConfigFile, RebornDoctorReport};

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
    build_doctor_dto_with_ambient_proxy_presence(context, context.ambient_proxy_present())
}

fn build_doctor_dto_with_ambient_proxy_presence(
    context: &RebornCliContext,
    ambient_proxy_present: bool,
) -> DoctorDto {
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

    let config_path = context.boot_config().home().config_file_path();
    checks.push(check_config_file(&config_path));

    let providers_path = context.boot_config().home().providers_file_path();
    checks.push(check_providers_file(&providers_path));

    if ambient_proxy_present {
        checks.push(DoctorCheck {
            name: "host_mediated_ambient_proxy".to_string(),
            category: CheckCategory::Core,
            outcome: CheckOutcome::Skip,
            detail: "ambient proxy variables are configured but ignored by host-mediated ReqwestNetworkTransport so approved pinned destinations remain authoritative; LLM clients and sandbox egress use separate proxy policies".to_string(),
        });
    }

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
            match ironclaw_operator::llm_admin::llm_catalog::validate_reborn_provider_catalog_contents(&contents)
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

    const HOST_MEDIATED_PROXY_CHECK: &str = "host_mediated_ambient_proxy";
    const HOST_MEDIATED_PROXY_DETAIL: &str = "ambient proxy variables are configured but ignored by host-mediated ReqwestNetworkTransport so approved pinned destinations remain authoritative; LLM clients and sandbox egress use separate proxy policies";
    const AMBIENT_PROXY_ENV_VARS: [&str; 6] = [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ];

    fn cleared_ambient_proxy_env() -> Vec<crate::runtime::test_env::EnvGuard> {
        AMBIENT_PROXY_ENV_VARS
            .into_iter()
            .map(crate::runtime::test_env::EnvGuard::clear)
            .collect()
    }

    fn render_doctor_text(dto: &DoctorDto) -> String {
        let mut output = Vec::new();
        dto.render_text_to(&mut output).expect("render doctor text");
        String::from_utf8(output).expect("doctor text is UTF-8")
    }

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
    fn doctor_reports_ignored_ambient_proxy_without_leaking_values() {
        let (_tmp, context) = RebornCliContext::test_context();
        // The diagnostic builder deliberately receives only presence state.
        // Proxy values therefore cannot cross into either rendering surface.
        let dto = build_doctor_dto_with_ambient_proxy_presence(&context, true);
        let check = dto
            .checks
            .iter()
            .find(|check| check.name == HOST_MEDIATED_PROXY_CHECK)
            .expect("configured ambient proxy variables must produce one stable diagnostic");

        assert_eq!(check.category, CheckCategory::Core);
        assert_eq!(check.outcome, CheckOutcome::Skip);
        assert_eq!(check.detail, HOST_MEDIATED_PROXY_DETAIL);

        let text = render_doctor_text(&dto);
        let json = serde_json::to_string(&dto).expect("serialize doctor JSON");
        for output in [&text, &json] {
            assert!(
                output.contains(HOST_MEDIATED_PROXY_CHECK),
                "diagnostic identity missing from output: {output}"
            );
            assert!(
                output.contains("host-mediated ReqwestNetworkTransport"),
                "output must identify the affected transport boundary: {output}"
            );
            assert!(
                output.contains("LLM clients") && output.contains("sandbox egress"),
                "output must distinguish LLM and sandbox proxy policy: {output}"
            );
            for sensitive_fragment in [
                "proxy-password",
                "proxy-token",
                "proxy.internal",
                "18443",
                "secret-path",
            ] {
                assert!(
                    !output.contains(sensitive_fragment),
                    "proxy configuration fragment leaked into output: {sensitive_fragment}"
                );
            }
        }
    }

    #[test]
    fn doctor_omits_ambient_proxy_diagnostic_when_unconfigured() {
        let (_tmp, context) = RebornCliContext::test_context();
        let dto = build_doctor_dto_with_ambient_proxy_presence(&context, false);
        let text = render_doctor_text(&dto);
        let json = serde_json::to_string(&dto).expect("serialize doctor JSON");

        assert!(
            dto.checks
                .iter()
                .all(|check| check.name != HOST_MEDIATED_PROXY_CHECK),
            "an absent ambient proxy configuration must not produce a warning"
        );
        assert!(!text.contains(HOST_MEDIATED_PROXY_CHECK));
        assert!(!json.contains(HOST_MEDIATED_PROXY_CHECK));
    }

    #[test]
    fn doctor_uses_proxy_presence_captured_when_cli_context_is_resolved() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let _cleared_proxy_env = cleared_ambient_proxy_env();
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = crate::runtime::test_env::EnvGuard::set(
            "HOME",
            temp.path().to_str().expect("temporary HOME must be UTF-8"),
        );
        let reborn_home = temp.path().join("reborn-home");
        let _reborn_home = crate::runtime::test_env::EnvGuard::set(
            "IRONCLAW_REBORN_HOME",
            reborn_home
                .to_str()
                .expect("temporary Reborn home must be UTF-8"),
        );

        let absent_context =
            RebornCliContext::resolve_from_env().expect("absent-proxy context must resolve");
        let present_context = {
            let _http_proxy = crate::runtime::test_env::EnvGuard::set(
                "HTTP_PROXY",
                "http://context-boundary-sentinel.invalid:18443/secret-path",
            );
            RebornCliContext::resolve_from_env().expect("present-proxy context must resolve")
        };
        assert!(
            std::env::var_os("HTTP_PROXY").is_none(),
            "the proxy variable must be absent before either context is rendered"
        );

        let absent_dto = build_doctor_dto(&absent_context);
        assert!(
            absent_dto
                .checks
                .iter()
                .all(|check| check.name != HOST_MEDIATED_PROXY_CHECK),
            "a context resolved without ambient proxy configuration must omit the diagnostic"
        );

        let present_dto = build_doctor_dto(&present_context);
        assert!(
            present_dto
                .checks
                .iter()
                .any(|check| check.name == HOST_MEDIATED_PROXY_CHECK),
            "a context resolved while an ambient proxy variable was present must retain the diagnostic after the process environment changes"
        );
    }

    #[cfg(unix)]
    #[test]
    fn doctor_treats_non_utf8_proxy_value_as_present() {
        use std::os::unix::ffi::OsStringExt as _;

        let _lock = crate::runtime::test_env::lock_runtime_env();
        let _cleared_proxy_env = cleared_ambient_proxy_env();
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = crate::runtime::test_env::EnvGuard::set(
            "HOME",
            temp.path().to_str().expect("temporary HOME must be UTF-8"),
        );
        let reborn_home = temp.path().join("reborn-home");
        let _reborn_home = crate::runtime::test_env::EnvGuard::set(
            "IRONCLAW_REBORN_HOME",
            reborn_home
                .to_str()
                .expect("temporary Reborn home must be UTF-8"),
        );

        // A non-empty, non-UTF-8 value is a real ambient proxy setting and
        // must be treated as present, not silently coerced to "absent" the
        // way an `Err(_)` from `std::env::var` would be.
        let invalid = std::ffi::OsString::from_vec(vec![0xff, 0xfe]);
        let _http_proxy = crate::runtime::test_env::EnvGuard::set_os("HTTP_PROXY", &invalid);

        let context =
            RebornCliContext::resolve_from_env().expect("non-UTF-8 proxy context must resolve");
        let dto = build_doctor_dto(&context);
        assert!(
            dto.checks
                .iter()
                .any(|check| check.name == HOST_MEDIATED_PROXY_CHECK),
            "a non-UTF-8 but non-empty proxy variable must still be reported as present"
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
