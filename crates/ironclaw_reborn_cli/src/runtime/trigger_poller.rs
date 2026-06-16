use std::time::Duration;

use ironclaw_reborn_composition::TriggerPollerSettings;

use super::RuntimeInputCaller;

/// Read a trigger-poller env var with **strict** presence semantics.
///
/// `IRONCLAW_TRIGGER_POLLER_*` env vars are operator-control knobs: presence
/// is authoritative, not just non-empty content. Treat the var as
///
/// - unset → `Ok(None)` (fall through to the config/default layer)
/// - set, empty or all-whitespace → fatal (operator must unset or fix)
/// - set, non-empty → `Ok(Some(value))` (caller validates content)
///
/// Distinct from the broader `optional_nonempty_env` used by optional-config
/// callers (OAuth, etc.), which intentionally collapses present-blank to
/// absent. Here, a present-but-blank env slot is almost always a bug — a
/// shell typo, a half-set deployment template, or a credential injector
/// that failed to populate the slot — and falling through silently would
/// drop the operator's intended kill-switch or interval override with no
/// visible signal.
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
        Err(std::env::VarError::NotUnicode(_)) => anyhow::bail!(
            "{name} contains non-UTF-8 bytes; either unset it or provide a valid value"
        ),
    }
}

/// Upper bound on `poll_interval_secs` (config) and
/// `IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS` (env). One hour caps the
/// common typo class of writing a value in milliseconds — `60_000`,
/// `86_400_000` — which would otherwise suspend the poller for hours
/// or years before the operator notices nothing is firing.
const MAX_POLL_INTERVAL_SECS: u64 = 3600;

/// Upper bound on the two jitter knobs. Larger than this, jitter no
/// longer feels like jitter and a misconfig has effectively replaced
/// the poll interval with a multi-hour randomised pause.
const MAX_JITTER_SECS: u64 = 3600;

/// Upper bound on `fires_per_tick`. The compiled default is 32; 1000
/// leaves plenty of headroom for a future high-throughput deployment
/// while rejecting accidents like `u32::MAX` (~4B dispatches per tick).
const MAX_FIRES_PER_TICK: u32 = 1000;

/// Truncate an env-var value to a bounded length before echoing it in an
/// error message. Prevents the value from blowing up startup logs if the
/// operator accidentally pastes a long string (e.g. a credential) into the
/// env slot. Char-aware so we cannot split a multi-byte UTF-8 codepoint.
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

/// Build [`TriggerPollerSettings`] by merging three layers of configuration.
///
/// Precedence (highest first):
/// 1. Environment variables:
///    - `IRONCLAW_TRIGGER_POLLER_ENABLED` — `1`/`true` → enabled, `0`/`false` → disabled
///      (case-insensitive). Overrides any `enabled` value from the config file.
///    - `IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS` — parse as `u64`; overrides the
///      config-file `poll_interval_secs`.  Must be > 0.
/// 2. Config-file `[trigger_poller]` section — all fields are optional; any field
///    absent here falls through to the compiled default.
/// 3. Compiled default — `TriggerPollerSettings::default()` (all limits at the
///    `ironclaw_triggers` crate defaults). The `enabled` default depends on the
///    caller: the local `serve` surface enables the scheduler by default so
///    automations actually run, while every other caller defaults to disabled.
///    Config and env still override this (an env kill-switch wins), because
///    this layer is applied first.
///
/// V1 invariant: `max_concurrent_fires_per_trigger` must be exactly 1. Passing
/// any other value (via config or, were an env override ever added, env) returns
/// an error rather than silently breaking per-trigger serialisation. The same
/// invariant is also enforced at spawn time by `TriggerPollerWorkerConfig::validate`
/// (in `ironclaw_triggers`); the CLI layer keeps the guard here so an invalid
/// config fails at boot-config parse rather than at first poller spawn.
pub(super) fn trigger_poller_settings(
    config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
    caller: RuntimeInputCaller,
) -> anyhow::Result<TriggerPollerSettings> {
    // Layer 3: compiled default. `enabled` is on by default for the local
    // `serve` surface (so scheduled automations actually fire) and off
    // everywhere else; config/env below still override it.
    let mut settings = TriggerPollerSettings::default();
    if caller == RuntimeInputCaller::Serve {
        settings.enabled = true;
    }

    // Layer 2: config-file [trigger_poller] section.
    if let Some(section) = config_file.and_then(|file| file.trigger_poller.as_ref()) {
        if let Some(enabled) = section.enabled {
            settings.enabled = enabled;
        }

        // Build a mutable worker config to apply section overrides.
        let mut worker = settings.worker;

        if let Some(secs) = section.poll_interval_secs {
            if secs == 0 || secs > MAX_POLL_INTERVAL_SECS {
                anyhow::bail!(
                    "config file [trigger_poller].poll_interval_secs must be in 1..={MAX_POLL_INTERVAL_SECS}; \
                     got {secs}"
                );
            }
            worker.poll_interval = Duration::from_secs(secs);
        }

        if let Some(fires) = section.fires_per_tick {
            if fires == 0 || fires > MAX_FIRES_PER_TICK {
                anyhow::bail!(
                    "config file [trigger_poller].fires_per_tick must be in 1..={MAX_FIRES_PER_TICK}; \
                     got {fires}"
                );
            }
            worker.fires_per_tick = fires as usize;
        }

        if let Some(max_concurrent) = section.max_concurrent_fires_per_trigger {
            // V1 invariant: per-trigger concurrency is locked at 1.
            if max_concurrent != 1 {
                anyhow::bail!(
                    "config file [trigger_poller].max_concurrent_fires_per_trigger must be 1 \
                     (V1 per-trigger serialisation invariant); got {max_concurrent}"
                );
            }
            worker.max_concurrent_fires_per_trigger = max_concurrent as usize;
        }

        if let Some(jitter_secs) = section.startup_jitter_max_secs {
            if jitter_secs > MAX_JITTER_SECS {
                anyhow::bail!(
                    "config file [trigger_poller].startup_jitter_max_secs must be <= {MAX_JITTER_SECS}; \
                     got {jitter_secs}"
                );
            }
            settings.startup_jitter_max = Duration::from_secs(jitter_secs);
        }

        if let Some(jitter_secs) = section.tick_jitter_max_secs {
            if jitter_secs > MAX_JITTER_SECS {
                anyhow::bail!(
                    "config file [trigger_poller].tick_jitter_max_secs must be <= {MAX_JITTER_SECS}; \
                     got {jitter_secs}"
                );
            }
            settings.tick_jitter_max = Duration::from_secs(jitter_secs);
        }

        settings.worker = worker;
    }

    // Layer 1: environment variable overrides. Uses strict presence
    // semantics — a present-but-blank value is fatal, not a silent
    // fall-through to config/default.
    if let Some(raw) = strict_env_var("IRONCLAW_TRIGGER_POLLER_ENABLED")? {
        match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" => settings.enabled = true,
            "0" | "false" => settings.enabled = false,
            _ => {
                // Display the operator's original value (case preserved), not the
                // lowercased match key — they need to find it in their config.
                let display = truncate_env_value_for_display(&raw);
                anyhow::bail!(
                    "IRONCLAW_TRIGGER_POLLER_ENABLED must be one of 1, true, 0, false (got {display:?})"
                )
            }
        }
    }

    if let Some(raw) = strict_env_var("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS")? {
        let secs: u64 = raw.trim().parse().map_err(|e| {
            let display = truncate_env_value_for_display(&raw);
            anyhow::anyhow!(
                "IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS must be a positive integer, got {display:?}: {e}"
            )
        })?;
        if secs == 0 || secs > MAX_POLL_INTERVAL_SECS {
            anyhow::bail!(
                "IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS must be in 1..={MAX_POLL_INTERVAL_SECS}; got {secs}"
            );
        }
        settings.worker.poll_interval = Duration::from_secs(secs);
    }

    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::super::test_env::{EnvGuard, lock_trigger_env};
    use super::{RuntimeInputCaller, trigger_poller_settings};
    use ironclaw_reborn_config::TriggerPollerConfigSection;
    use std::time::Duration;

    fn make_config_with_trigger_poller(
        section: TriggerPollerConfigSection,
    ) -> ironclaw_reborn_config::RebornConfigFile {
        ironclaw_reborn_config::RebornConfigFile {
            trigger_poller: Some(section),
            ..Default::default()
        }
    }

    #[test]
    fn trigger_poller_settings_default_is_disabled() {
        // No config file, no env → disabled with zero jitter. Hold the env
        // lock for the whole test so a sibling test cannot mutate the env
        // between EnvGuard::clear and the call below.
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_ENABLED");
        let _interval = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS");

        let settings = trigger_poller_settings(None, RuntimeInputCaller::Run)
            .expect("default trigger poller settings");

        assert!(!settings.enabled, "default must be disabled");
        assert_eq!(settings.startup_jitter_max, Duration::ZERO);
        assert_eq!(settings.tick_jitter_max, Duration::ZERO);
    }

    #[test]
    fn trigger_poller_settings_serve_default_is_enabled() {
        // Regression: the local `serve` surface must default the scheduler on so
        // scheduled automations actually fire, while other callers stay off.
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_ENABLED");
        let _interval = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS");

        let settings = trigger_poller_settings(None, RuntimeInputCaller::Serve)
            .expect("serve trigger poller settings");

        assert!(settings.enabled, "serve must default the scheduler on");
    }

    #[test]
    fn trigger_poller_settings_serve_default_respects_env_kill_switch() {
        // The serve-on default is layer 3, so an explicit env kill-switch still
        // wins and turns the scheduler off.
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::set("IRONCLAW_TRIGGER_POLLER_ENABLED", "0");
        let _interval = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS");

        let settings = trigger_poller_settings(None, RuntimeInputCaller::Serve)
            .expect("serve trigger poller settings with kill switch");

        assert!(
            !settings.enabled,
            "env kill-switch must override the serve-on default"
        );
    }

    #[test]
    fn trigger_poller_settings_config_enabled_maps_worker_fields() {
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_ENABLED");
        let _interval = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS");

        let section = TriggerPollerConfigSection {
            enabled: Some(true),
            poll_interval_secs: Some(15),
            fires_per_tick: Some(50),
            max_concurrent_fires_per_trigger: Some(1),
            startup_jitter_max_secs: Some(3),
            tick_jitter_max_secs: Some(7),
        };
        let config = make_config_with_trigger_poller(section);

        let settings = trigger_poller_settings(Some(&config), RuntimeInputCaller::Run)
            .expect("trigger poller settings from config");

        assert!(settings.enabled, "config enabled=true must be reflected");
        assert_eq!(settings.worker.poll_interval, Duration::from_secs(15));
        assert_eq!(settings.worker.fires_per_tick, 50);
        assert_eq!(settings.worker.max_concurrent_fires_per_trigger, 1);
        assert_eq!(settings.startup_jitter_max, Duration::from_secs(3));
        assert_eq!(settings.tick_jitter_max, Duration::from_secs(7));
    }

    #[test]
    fn trigger_poller_settings_max_concurrent_fires_greater_than_1_is_error() {
        let section = TriggerPollerConfigSection {
            enabled: Some(true),
            max_concurrent_fires_per_trigger: Some(2),
            ..Default::default()
        };
        let config = make_config_with_trigger_poller(section);

        let err = trigger_poller_settings(Some(&config), RuntimeInputCaller::Run)
            .expect_err("max_concurrent_fires_per_trigger=2 must be rejected");

        assert!(
            err.to_string().contains("max_concurrent_fires_per_trigger"),
            "error must mention the field, got: {err}",
        );
    }

    #[test]
    fn trigger_poller_settings_config_poll_interval_zero_is_error() {
        let section = TriggerPollerConfigSection {
            enabled: Some(true),
            poll_interval_secs: Some(0),
            ..Default::default()
        };
        let config = make_config_with_trigger_poller(section);

        let err = trigger_poller_settings(Some(&config), RuntimeInputCaller::Run)
            .expect_err("poll_interval_secs=0 must be rejected");

        assert!(
            err.to_string().contains("poll_interval_secs"),
            "error must mention the field, got: {err}",
        );
    }

    #[test]
    fn trigger_poller_settings_env_enabled_overrides_config_disabled() {
        // Config says disabled; env says enabled — env must win.
        let _lock = lock_trigger_env();
        let _interval = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS");
        let _enabled = EnvGuard::set("IRONCLAW_TRIGGER_POLLER_ENABLED", "true");

        let section = TriggerPollerConfigSection {
            enabled: Some(false),
            ..Default::default()
        };
        let config = make_config_with_trigger_poller(section);

        let settings = trigger_poller_settings(Some(&config), RuntimeInputCaller::Run)
            .expect("env override should succeed");
        assert!(
            settings.enabled,
            "IRONCLAW_TRIGGER_POLLER_ENABLED=true must override config enabled=false"
        );
    }

    #[test]
    fn trigger_poller_settings_env_disabled_overrides_config_enabled() {
        // Operator kill-switch: config has enabled=true but the env var disables.
        let _lock = lock_trigger_env();
        let _interval = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS");
        let _enabled = EnvGuard::set("IRONCLAW_TRIGGER_POLLER_ENABLED", "false");

        let section = TriggerPollerConfigSection {
            enabled: Some(true),
            ..Default::default()
        };
        let config = make_config_with_trigger_poller(section);

        let settings = trigger_poller_settings(Some(&config), RuntimeInputCaller::Run)
            .expect("env kill-switch should succeed");
        assert!(
            !settings.enabled,
            "IRONCLAW_TRIGGER_POLLER_ENABLED=false must override config enabled=true"
        );
    }

    #[test]
    fn trigger_poller_settings_env_interval_overrides_config_interval() {
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_ENABLED");
        let _interval = EnvGuard::set("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS", "45");

        let section = TriggerPollerConfigSection {
            enabled: Some(true),
            poll_interval_secs: Some(15),
            ..Default::default()
        };
        let config = make_config_with_trigger_poller(section);

        let settings = trigger_poller_settings(Some(&config), RuntimeInputCaller::Run)
            .expect("env interval override should succeed");
        assert_eq!(
            settings.worker.poll_interval,
            Duration::from_secs(45),
            "env value 45 must override config value 15"
        );
    }

    #[test]
    fn trigger_poller_settings_env_interval_zero_is_error() {
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_ENABLED");
        let _interval = EnvGuard::set("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS", "0");

        let err = trigger_poller_settings(None, RuntimeInputCaller::Run)
            .expect_err("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS=0 must be rejected");

        assert!(
            err.to_string()
                .contains("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS"),
            "error must mention the env var, got: {err}",
        );
    }

    #[test]
    fn trigger_poller_settings_env_interval_non_numeric_preserves_parse_error() {
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_ENABLED");
        let _interval = EnvGuard::set("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS", "10s");

        let err = trigger_poller_settings(None, RuntimeInputCaller::Run)
            .expect_err("non-numeric env interval must be rejected");

        let msg = err.to_string();
        assert!(
            msg.contains("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS") && msg.contains("10s"),
            "error must surface env var name and offending value, got: {msg}",
        );
    }

    #[test]
    fn trigger_poller_settings_env_enabled_invalid_value_is_error() {
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::set("IRONCLAW_TRIGGER_POLLER_ENABLED", "yes");
        let _interval = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS");

        let err = trigger_poller_settings(None, RuntimeInputCaller::Run)
            .expect_err("IRONCLAW_TRIGGER_POLLER_ENABLED=yes must be rejected");

        let msg = err.to_string();
        assert!(
            msg.contains("IRONCLAW_TRIGGER_POLLER_ENABLED") && msg.contains("yes"),
            "error must surface env var name and offending value, got: {msg}",
        );
    }

    #[test]
    fn trigger_poller_settings_config_fires_per_tick_zero_is_error() {
        let section = TriggerPollerConfigSection {
            enabled: Some(true),
            fires_per_tick: Some(0),
            ..Default::default()
        };
        let config = make_config_with_trigger_poller(section);

        let err = trigger_poller_settings(Some(&config), RuntimeInputCaller::Run)
            .expect_err("fires_per_tick=0 must be rejected");

        assert!(
            err.to_string().contains("fires_per_tick"),
            "error must mention the field, got: {err}",
        );
    }

    #[test]
    fn trigger_poller_settings_config_poll_interval_above_cap_is_error() {
        // 86_400 secs = 1 day = far above the 3600s cap. Models the
        // common typo class of writing a millisecond value (e.g. 86_400_000).
        let section = TriggerPollerConfigSection {
            enabled: Some(true),
            poll_interval_secs: Some(86_400),
            ..Default::default()
        };
        let config = make_config_with_trigger_poller(section);

        let err = trigger_poller_settings(Some(&config), RuntimeInputCaller::Run)
            .expect_err("poll_interval_secs above the cap must be rejected");

        let msg = err.to_string();
        assert!(
            msg.contains("poll_interval_secs") && msg.contains("86400"),
            "error must mention the field and the offending value, got: {msg}",
        );
    }

    #[test]
    fn trigger_poller_settings_config_fires_per_tick_above_cap_is_error() {
        let section = TriggerPollerConfigSection {
            enabled: Some(true),
            fires_per_tick: Some(10_000),
            ..Default::default()
        };
        let config = make_config_with_trigger_poller(section);

        let err = trigger_poller_settings(Some(&config), RuntimeInputCaller::Run)
            .expect_err("fires_per_tick above the cap must be rejected");

        let msg = err.to_string();
        assert!(
            msg.contains("fires_per_tick") && msg.contains("10000"),
            "error must mention the field and the offending value, got: {msg}",
        );
    }

    #[test]
    fn trigger_poller_settings_config_startup_jitter_above_cap_is_error() {
        let section = TriggerPollerConfigSection {
            enabled: Some(true),
            startup_jitter_max_secs: Some(3601),
            ..Default::default()
        };
        let config = make_config_with_trigger_poller(section);

        let err = trigger_poller_settings(Some(&config), RuntimeInputCaller::Run)
            .expect_err("startup_jitter_max_secs above the cap must be rejected");

        assert!(
            err.to_string().contains("startup_jitter_max_secs"),
            "error must mention the field, got: {err}",
        );
    }

    #[test]
    fn trigger_poller_settings_config_tick_jitter_above_cap_is_error() {
        let section = TriggerPollerConfigSection {
            enabled: Some(true),
            tick_jitter_max_secs: Some(3601),
            ..Default::default()
        };
        let config = make_config_with_trigger_poller(section);

        let err = trigger_poller_settings(Some(&config), RuntimeInputCaller::Run)
            .expect_err("tick_jitter_max_secs above the cap must be rejected");

        assert!(
            err.to_string().contains("tick_jitter_max_secs"),
            "error must mention the field, got: {err}",
        );
    }

    #[test]
    fn trigger_poller_settings_env_interval_above_cap_is_error() {
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_ENABLED");
        let _interval = EnvGuard::set("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS", "86400");

        let err = trigger_poller_settings(None, RuntimeInputCaller::Run)
            .expect_err("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS above the cap must be rejected");

        let msg = err.to_string();
        assert!(
            msg.contains("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS") && msg.contains("86400"),
            "error must surface env var name and value, got: {msg}",
        );
    }

    #[test]
    fn trigger_poller_settings_env_enabled_invalid_value_preserves_case() {
        // Operator must see what they actually typed (e.g. "YES"), not the
        // lowercased match key, so they can find the value in their config.
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::set("IRONCLAW_TRIGGER_POLLER_ENABLED", "YES");
        let _interval = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS");

        let err = trigger_poller_settings(None, RuntimeInputCaller::Run)
            .expect_err("IRONCLAW_TRIGGER_POLLER_ENABLED=YES must be rejected");

        let msg = err.to_string();
        assert!(
            msg.contains("YES"),
            "error must surface the operator's original (un-lowercased) value, got: {msg}",
        );
    }

    #[test]
    fn trigger_poller_settings_config_poll_interval_at_cap_is_accepted() {
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_ENABLED");
        let _interval = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS");
        let section = TriggerPollerConfigSection {
            enabled: Some(true),
            poll_interval_secs: Some(3600),
            ..Default::default()
        };
        let config = make_config_with_trigger_poller(section);

        let settings = trigger_poller_settings(Some(&config), RuntimeInputCaller::Run)
            .expect("poll_interval_secs at the cap must be accepted");
        assert_eq!(settings.worker.poll_interval, Duration::from_secs(3600));
    }

    #[test]
    fn trigger_poller_settings_config_fires_per_tick_at_cap_is_accepted() {
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_ENABLED");
        let _interval = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS");
        let section = TriggerPollerConfigSection {
            enabled: Some(true),
            fires_per_tick: Some(1000),
            ..Default::default()
        };
        let config = make_config_with_trigger_poller(section);

        let settings = trigger_poller_settings(Some(&config), RuntimeInputCaller::Run)
            .expect("fires_per_tick at the cap must be accepted");
        assert_eq!(settings.worker.fires_per_tick, 1000);
    }

    #[test]
    fn trigger_poller_settings_config_jitter_at_cap_is_accepted() {
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_ENABLED");
        let _interval = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS");
        let section = TriggerPollerConfigSection {
            enabled: Some(true),
            startup_jitter_max_secs: Some(3600),
            tick_jitter_max_secs: Some(3600),
            ..Default::default()
        };
        let config = make_config_with_trigger_poller(section);

        let settings = trigger_poller_settings(Some(&config), RuntimeInputCaller::Run)
            .expect("jitter at the cap must be accepted");
        assert_eq!(settings.startup_jitter_max, Duration::from_secs(3600));
        assert_eq!(settings.tick_jitter_max, Duration::from_secs(3600));
    }

    #[test]
    fn trigger_poller_settings_max_concurrent_fires_zero_is_error() {
        // 0 has distinct semantic meaning in some systems (unlimited
        // concurrency); confirm the V1 guard rejects it explicitly.
        let section = TriggerPollerConfigSection {
            enabled: Some(true),
            max_concurrent_fires_per_trigger: Some(0),
            ..Default::default()
        };
        let config = make_config_with_trigger_poller(section);

        let err = trigger_poller_settings(Some(&config), RuntimeInputCaller::Run)
            .expect_err("max_concurrent_fires_per_trigger=0 must be rejected");
        assert!(
            err.to_string().contains("max_concurrent_fires_per_trigger"),
            "error must mention the field, got: {err}",
        );
    }

    #[test]
    fn trigger_poller_settings_env_enabled_numeric_one_enables() {
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::set("IRONCLAW_TRIGGER_POLLER_ENABLED", "1");
        let _interval = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS");

        let settings =
            trigger_poller_settings(None, RuntimeInputCaller::Run).expect("ENABLED=1 must succeed");
        assert!(
            settings.enabled,
            "IRONCLAW_TRIGGER_POLLER_ENABLED=1 must enable"
        );
    }

    #[test]
    fn trigger_poller_settings_env_enabled_numeric_zero_disables() {
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::set("IRONCLAW_TRIGGER_POLLER_ENABLED", "0");
        let _interval = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS");

        let section = TriggerPollerConfigSection {
            enabled: Some(true),
            ..Default::default()
        };
        let config = make_config_with_trigger_poller(section);

        let settings = trigger_poller_settings(Some(&config), RuntimeInputCaller::Run)
            .expect("ENABLED=0 must succeed");
        assert!(
            !settings.enabled,
            "IRONCLAW_TRIGGER_POLLER_ENABLED=0 must disable"
        );
    }

    #[test]
    fn trigger_poller_settings_env_enabled_long_value_is_truncated() {
        // Operator pastes an 80-char string into the env slot by mistake. The
        // error message must NOT echo the full value verbatim (it might be a
        // credential); the truncation ellipsis MUST appear.
        let _lock = lock_trigger_env();
        let long_value: String = "x".repeat(80);
        let _enabled = EnvGuard::set("IRONCLAW_TRIGGER_POLLER_ENABLED", &long_value);
        let _interval = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS");

        let err = trigger_poller_settings(None, RuntimeInputCaller::Run)
            .expect_err("long ENABLED value must be rejected");
        let msg = err.to_string();

        assert!(
            msg.contains('…'),
            "truncation ellipsis must appear in error, got: {msg}",
        );
        assert!(
            !msg.contains(&long_value),
            "full untruncated value must not appear in error, got: {msg}",
        );
    }

    // --- strict env contract: present-blank is fatal, not fall-through ---

    #[test]
    fn trigger_poller_settings_env_enabled_empty_is_error() {
        // ENABLED="" must NOT silently fall through to config — operator's
        // env slot is present but empty, almost always a deployment bug.
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::set("IRONCLAW_TRIGGER_POLLER_ENABLED", "");
        let _interval = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS");

        let section = TriggerPollerConfigSection {
            enabled: Some(true),
            ..Default::default()
        };
        let config = make_config_with_trigger_poller(section);

        let err = trigger_poller_settings(Some(&config), RuntimeInputCaller::Run).expect_err(
            "empty IRONCLAW_TRIGGER_POLLER_ENABLED must be rejected, not silently dropped",
        );

        assert!(
            err.to_string().contains("IRONCLAW_TRIGGER_POLLER_ENABLED"),
            "error must mention the env var, got: {err}",
        );
    }

    #[test]
    fn trigger_poller_settings_env_enabled_whitespace_is_error() {
        // ENABLED="   " (all-whitespace) hits the same fatal path as "".
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::set("IRONCLAW_TRIGGER_POLLER_ENABLED", "   ");
        let _interval = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS");

        let err = trigger_poller_settings(None, RuntimeInputCaller::Run)
            .expect_err("whitespace-only IRONCLAW_TRIGGER_POLLER_ENABLED must be rejected");

        assert!(
            err.to_string().contains("IRONCLAW_TRIGGER_POLLER_ENABLED"),
            "error must mention the env var, got: {err}",
        );
    }

    #[test]
    fn trigger_poller_settings_env_interval_empty_is_error() {
        // INTERVAL_SECS="" follows the same strict contract.
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_ENABLED");
        let _interval = EnvGuard::set("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS", "");

        let err = trigger_poller_settings(None, RuntimeInputCaller::Run).expect_err(
            "empty IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS must be rejected, not silently dropped",
        );

        assert!(
            err.to_string()
                .contains("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS"),
            "error must mention the env var, got: {err}",
        );
    }
}
