use std::time::Duration;

use ironclaw_reborn_composition::GithubIssueWorkflowSettings;

use super::RuntimeInputCaller;

const MAX_INTERVAL_SECS: u64 = 3600;
const MAX_BATCH_SIZE: usize = 1000;
const MAX_LEASE_DURATION_SECS: u64 = 24 * 3600;

fn strict_env_var(name: &str) -> anyhow::Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) => {
            if value.trim().is_empty() {
                anyhow::bail!(
                    "{name} is set but empty or whitespace-only; either unset it or provide a valid value"
                );
            }
            Ok(Some(value))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            anyhow::bail!(
                "{name} contains non-UTF-8 bytes; either unset it or provide a valid value"
            )
        }
    }
}

fn truncate_env_value_for_display(raw: &str) -> String {
    const MAX_CHARS: usize = 64;
    let mut iter = raw.chars();
    let truncated: String = iter.by_ref().take(MAX_CHARS).collect();
    if iter.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn parse_enabled_env(name: &str, raw: String) -> anyhow::Result<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        _ => {
            let display = truncate_env_value_for_display(&raw);
            anyhow::bail!("{name} must be one of 1, true, 0, false (got {display:?})")
        }
    }
}

fn parse_u64_env(name: &str, raw: String) -> anyhow::Result<u64> {
    raw.trim().parse().map_err(|error| {
        let display = truncate_env_value_for_display(&raw);
        anyhow::anyhow!("{name} must be a positive integer, got {display:?}: {error}")
    })
}

fn parse_usize_env(name: &str, raw: String) -> anyhow::Result<usize> {
    raw.trim().parse().map_err(|error| {
        let display = truncate_env_value_for_display(&raw);
        anyhow::anyhow!("{name} must be a positive integer, got {display:?}: {error}")
    })
}

fn ensure_feature_enabled(
    settings: GithubIssueWorkflowSettings,
) -> anyhow::Result<GithubIssueWorkflowSettings> {
    #[cfg(feature = "github-issue-workflow-beta")]
    {
        Ok(settings)
    }

    #[cfg(not(feature = "github-issue-workflow-beta"))]
    {
        if settings.enabled {
            anyhow::bail!(
                "GitHub issue workflow is enabled in config/env, but this binary was built without the `github-issue-workflow-beta` feature"
            );
        }
        Ok(settings)
    }
}

pub(super) fn github_issue_workflow_settings(
    config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
    _caller: RuntimeInputCaller,
) -> anyhow::Result<GithubIssueWorkflowSettings> {
    let mut settings = GithubIssueWorkflowSettings::disabled();

    if let Some(section) = config_file.and_then(|file| file.github_issue_workflow.as_ref()) {
        if let Some(enabled) = section.enabled {
            settings.enabled = enabled;
        }
        if let Some(secs) = section.poll_interval_secs {
            if secs == 0 || secs > MAX_INTERVAL_SECS {
                anyhow::bail!(
                    "config file [github_issue_workflow].poll_interval_secs must be in 1..={MAX_INTERVAL_SECS}; got {secs}"
                );
            }
            settings.poll_interval = Duration::from_secs(secs);
        }
        if let Some(value) = section.max_repos_per_tick {
            if value == 0 || value > MAX_BATCH_SIZE {
                anyhow::bail!(
                    "config file [github_issue_workflow].max_repos_per_tick must be in 1..={MAX_BATCH_SIZE}; got {value}"
                );
            }
            settings.max_repos_per_tick = value;
        }
        if let Some(value) = section.max_issues_per_repo_per_tick {
            if value == 0 || value > MAX_BATCH_SIZE {
                anyhow::bail!(
                    "config file [github_issue_workflow].max_issues_per_repo_per_tick must be in 1..={MAX_BATCH_SIZE}; got {value}"
                );
            }
            settings.max_issues_per_repo_per_tick = value;
        }
        if let Some(value) = section.max_runnable_runs_per_tick {
            if value == 0 || value > MAX_BATCH_SIZE {
                anyhow::bail!(
                    "config file [github_issue_workflow].max_runnable_runs_per_tick must be in 1..={MAX_BATCH_SIZE}; got {value}"
                );
            }
            settings.max_runnable_runs_per_tick = value;
        }
        if let Some(secs) = section.lease_duration_secs {
            if secs == 0 || secs > MAX_LEASE_DURATION_SECS {
                anyhow::bail!(
                    "config file [github_issue_workflow].lease_duration_secs must be in 1..={MAX_LEASE_DURATION_SECS}; got {secs}"
                );
            }
            settings.lease_duration = Duration::from_secs(secs);
        }
    }

    if let Some(raw) = strict_env_var("IRONCLAW_GITHUB_ISSUE_WORKFLOW_ENABLED")? {
        settings.enabled = parse_enabled_env("IRONCLAW_GITHUB_ISSUE_WORKFLOW_ENABLED", raw)?;
    }
    if let Some(raw) = strict_env_var("IRONCLAW_GITHUB_ISSUE_WORKFLOW_INTERVAL_SECS")? {
        let secs = parse_u64_env("IRONCLAW_GITHUB_ISSUE_WORKFLOW_INTERVAL_SECS", raw)?;
        if secs == 0 || secs > MAX_INTERVAL_SECS {
            anyhow::bail!(
                "IRONCLAW_GITHUB_ISSUE_WORKFLOW_INTERVAL_SECS must be in 1..={MAX_INTERVAL_SECS}; got {secs}"
            );
        }
        settings.poll_interval = Duration::from_secs(secs);
    }
    if let Some(raw) = strict_env_var("IRONCLAW_GITHUB_ISSUE_WORKFLOW_MAX_REPOS_PER_TICK")? {
        let value = parse_usize_env("IRONCLAW_GITHUB_ISSUE_WORKFLOW_MAX_REPOS_PER_TICK", raw)?;
        if value == 0 || value > MAX_BATCH_SIZE {
            anyhow::bail!(
                "IRONCLAW_GITHUB_ISSUE_WORKFLOW_MAX_REPOS_PER_TICK must be in 1..={MAX_BATCH_SIZE}; got {value}"
            );
        }
        settings.max_repos_per_tick = value;
    }
    if let Some(raw) =
        strict_env_var("IRONCLAW_GITHUB_ISSUE_WORKFLOW_MAX_ISSUES_PER_REPO_PER_TICK")?
    {
        let value = parse_usize_env(
            "IRONCLAW_GITHUB_ISSUE_WORKFLOW_MAX_ISSUES_PER_REPO_PER_TICK",
            raw,
        )?;
        if value == 0 || value > MAX_BATCH_SIZE {
            anyhow::bail!(
                "IRONCLAW_GITHUB_ISSUE_WORKFLOW_MAX_ISSUES_PER_REPO_PER_TICK must be in 1..={MAX_BATCH_SIZE}; got {value}"
            );
        }
        settings.max_issues_per_repo_per_tick = value;
    }
    if let Some(raw) = strict_env_var("IRONCLAW_GITHUB_ISSUE_WORKFLOW_MAX_RUNNABLE_RUNS_PER_TICK")?
    {
        let value = parse_usize_env(
            "IRONCLAW_GITHUB_ISSUE_WORKFLOW_MAX_RUNNABLE_RUNS_PER_TICK",
            raw,
        )?;
        if value == 0 || value > MAX_BATCH_SIZE {
            anyhow::bail!(
                "IRONCLAW_GITHUB_ISSUE_WORKFLOW_MAX_RUNNABLE_RUNS_PER_TICK must be in 1..={MAX_BATCH_SIZE}; got {value}"
            );
        }
        settings.max_runnable_runs_per_tick = value;
    }
    if let Some(raw) = strict_env_var("IRONCLAW_GITHUB_ISSUE_WORKFLOW_LEASE_DURATION_SECS")? {
        let secs = parse_u64_env("IRONCLAW_GITHUB_ISSUE_WORKFLOW_LEASE_DURATION_SECS", raw)?;
        if secs == 0 || secs > MAX_LEASE_DURATION_SECS {
            anyhow::bail!(
                "IRONCLAW_GITHUB_ISSUE_WORKFLOW_LEASE_DURATION_SECS must be in 1..={MAX_LEASE_DURATION_SECS}; got {secs}"
            );
        }
        settings.lease_duration = Duration::from_secs(secs);
    }

    ensure_feature_enabled(settings)
}

#[cfg(test)]
mod tests {
    use super::super::RuntimeInputCaller;
    use super::super::test_env::{EnvGuard, lock_trigger_env};
    use super::github_issue_workflow_settings;
    use ironclaw_reborn_config::GithubIssueWorkflowConfigSection;
    use std::time::Duration;

    fn make_config_with_workflow(
        section: GithubIssueWorkflowConfigSection,
    ) -> ironclaw_reborn_config::RebornConfigFile {
        ironclaw_reborn_config::RebornConfigFile {
            github_issue_workflow: Some(section),
            ..Default::default()
        }
    }

    fn clear_workflow_env() -> Vec<EnvGuard> {
        vec![
            EnvGuard::clear("IRONCLAW_GITHUB_ISSUE_WORKFLOW_ENABLED"),
            EnvGuard::clear("IRONCLAW_GITHUB_ISSUE_WORKFLOW_INTERVAL_SECS"),
            EnvGuard::clear("IRONCLAW_GITHUB_ISSUE_WORKFLOW_MAX_REPOS_PER_TICK"),
            EnvGuard::clear("IRONCLAW_GITHUB_ISSUE_WORKFLOW_MAX_ISSUES_PER_REPO_PER_TICK"),
            EnvGuard::clear("IRONCLAW_GITHUB_ISSUE_WORKFLOW_MAX_RUNNABLE_RUNS_PER_TICK"),
            EnvGuard::clear("IRONCLAW_GITHUB_ISSUE_WORKFLOW_LEASE_DURATION_SECS"),
        ]
    }

    #[test]
    fn github_issue_workflow_settings_default_is_disabled() {
        let _lock = lock_trigger_env();
        let _guards = clear_workflow_env();

        let settings =
            github_issue_workflow_settings(None, RuntimeInputCaller::Run).expect("settings");

        assert!(!settings.enabled, "default must be disabled");
        assert_eq!(settings.poll_interval, Duration::from_secs(60));
    }

    #[cfg(feature = "github-issue-workflow-beta")]
    #[test]
    fn github_issue_workflow_settings_config_maps_all_fields() {
        let _lock = lock_trigger_env();
        let _guards = clear_workflow_env();
        let config = make_config_with_workflow(GithubIssueWorkflowConfigSection {
            enabled: Some(true),
            poll_interval_secs: Some(21),
            max_repos_per_tick: Some(3),
            max_issues_per_repo_per_tick: Some(4),
            max_runnable_runs_per_tick: Some(5),
            lease_duration_secs: Some(90),
        });

        let settings = github_issue_workflow_settings(Some(&config), RuntimeInputCaller::Run)
            .expect("settings");

        assert!(settings.enabled);
        assert_eq!(settings.poll_interval, Duration::from_secs(21));
        assert_eq!(settings.max_repos_per_tick, 3);
        assert_eq!(settings.max_issues_per_repo_per_tick, 4);
        assert_eq!(settings.max_runnable_runs_per_tick, 5);
        assert_eq!(settings.lease_duration, Duration::from_secs(90));
    }

    #[cfg(feature = "github-issue-workflow-beta")]
    #[test]
    fn github_issue_workflow_settings_env_overrides_config() {
        let _lock = lock_trigger_env();
        let mut guards = clear_workflow_env();
        guards.push(EnvGuard::set(
            "IRONCLAW_GITHUB_ISSUE_WORKFLOW_ENABLED",
            "true",
        ));
        guards.push(EnvGuard::set(
            "IRONCLAW_GITHUB_ISSUE_WORKFLOW_INTERVAL_SECS",
            "45",
        ));
        let config = make_config_with_workflow(GithubIssueWorkflowConfigSection {
            enabled: Some(false),
            poll_interval_secs: Some(15),
            max_repos_per_tick: None,
            max_issues_per_repo_per_tick: None,
            max_runnable_runs_per_tick: None,
            lease_duration_secs: None,
        });

        let settings = github_issue_workflow_settings(Some(&config), RuntimeInputCaller::Run)
            .expect("settings");

        assert!(settings.enabled);
        assert_eq!(settings.poll_interval, Duration::from_secs(45));
    }

    #[test]
    fn github_issue_workflow_settings_rejects_invalid_env() {
        let _lock = lock_trigger_env();
        let mut guards = clear_workflow_env();
        guards.push(EnvGuard::set(
            "IRONCLAW_GITHUB_ISSUE_WORKFLOW_ENABLED",
            "sometimes",
        ));

        let err = github_issue_workflow_settings(None, RuntimeInputCaller::Run)
            .expect_err("invalid env must fail");

        assert!(
            err.to_string()
                .contains("IRONCLAW_GITHUB_ISSUE_WORKFLOW_ENABLED"),
            "error must mention env var name, got: {err}"
        );
    }

    #[cfg(not(feature = "github-issue-workflow-beta"))]
    #[test]
    fn github_issue_workflow_settings_feature_off_rejects_enabled_config() {
        let _lock = lock_trigger_env();
        let _guards = clear_workflow_env();
        let config = make_config_with_workflow(GithubIssueWorkflowConfigSection {
            enabled: Some(true),
            ..Default::default()
        });

        let err = github_issue_workflow_settings(Some(&config), RuntimeInputCaller::Run)
            .expect_err("feature-off build must reject enabled workflow");

        assert!(
            err.to_string().contains("github-issue-workflow-beta"),
            "error must mention missing feature flag, got: {err}"
        );
    }
}
