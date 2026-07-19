use clap::Args;
use ironclaw_reborn_composition::{
    RebornRuntimeComponentStatus, reborn_model_slot_names, reborn_runtime_readiness_snapshot,
};

use crate::context::RebornCliContext;
use crate::dto::{ComponentStatus, DriversSnapshot, FilePresence, ServiceStateDto, StatusDto};
use crate::render::{self, OutputMode, Renderable, terminal_safe_text};
use std::io::Write;

#[derive(Debug, Args)]
pub(crate) struct StatusCommand {
    /// Output as JSON.
    #[arg(long)]
    json: bool,
}

impl StatusCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let dto = build_status_dto(&context)?;
        let mode = if self.json {
            OutputMode::Json
        } else {
            OutputMode::Text
        };
        render::output(&dto, mode)
    }
}

fn build_status_dto(context: &RebornCliContext) -> anyhow::Result<StatusDto> {
    build_status_dto_with_service_state(context, resolve_service_state())
}

/// Same as [`build_status_dto`] but with the live service-state query
/// injectable (mirrors `commands::service`'s `*_with_runner` seam), so
/// tests don't depend on the test host's actual launchd/systemd state.
fn build_status_dto_with_service_state(
    context: &RebornCliContext,
    service: ServiceStateDto,
) -> anyhow::Result<StatusDto> {
    let home = context.boot_config().home();
    let profile = context.boot_config().profile();
    let config_path = home.config_file_path();
    // Cloned before `config_path` moves into `FilePresence` below —
    // `resolve_login_link_and_note` needs it to check `[webui].env_token_var`.
    let config_path_for_webui_lookup = config_path.clone();
    let providers_path = home.providers_file_path();

    let snapshot = reborn_runtime_readiness_snapshot();
    let model_slots = reborn_model_slot_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    let (login_link, login_note) =
        resolve_login_link_and_note(home, &config_path_for_webui_lookup)?;
    let (login_link, login_note) = apply_service_suppression(service, login_link, login_note);

    Ok(StatusDto {
        version: env!("CARGO_PKG_VERSION").to_string(),
        reborn_home: home.path().to_path_buf(),
        home_source: home.source_label(),
        profile: profile.as_str().to_string(),
        config_file: FilePresence {
            present: config_path.exists(),
            path: config_path,
        },
        providers_file: FilePresence {
            present: providers_path.exists(),
            path: providers_path,
        },
        model_slots,
        drivers: DriversSnapshot {
            text_only: convert_component_status(&snapshot.text_only_driver),
            planned: convert_component_status(&snapshot.planned_driver),
            subagent_planned: convert_component_status(&snapshot.subagent_planned_driver),
            planned_default_profile: convert_component_status(&snapshot.planned_default_profile),
        },
        login_link,
        login_note,
        service,
        google_oauth_degraded: resolve_google_oauth_degraded(context.boot_config())?,
    })
}

const SERVICE_NOT_RUNNING_LOGIN_NOTE: &str = "service is not running — start with \
     `ironclaw-reborn service restart`; link available once running";

/// Overrides `(login_link, login_note)` when the live service state is
/// *known* not-running (`Stopped`/`NotInstalled`) — no login link can be
/// live then, regardless of credential source. `Running`/`Unknown`
/// (detection failed or feature off) pass the original pair through.
fn apply_service_suppression(
    service: ServiceStateDto,
    login_link: Option<String>,
    login_note: Option<String>,
) -> (Option<String>, Option<String>) {
    match service {
        ServiceStateDto::Stopped | ServiceStateDto::NotInstalled => {
            (None, Some(SERVICE_NOT_RUNNING_LOGIN_NOTE.to_string()))
        }
        ServiceStateDto::Running | ServiceStateDto::Unknown => (login_link, login_note),
    }
}

/// Re-runs the boot resolver's public-field asymmetry check over env and
/// `[google]` config.toml purely to report *why* Google OAuth is disabled when
/// partially configured. The encrypted client secret is deliberately not
/// opened: it is optional under public-client PKCE and cannot change this
/// classification. Returns `None` when Google OAuth is either fully
/// unconfigured or fully configured; only the partial case is reported.
fn resolve_google_oauth_degraded(
    config: &ironclaw_reborn_config::RebornBootConfig,
) -> anyhow::Result<Option<String>> {
    Ok(
        crate::runtime::resolve_google_oauth_config_state_from_env(config)?.and_then(|state| {
            state.missing_config_key().map(|missing| {
                format!(
                    "partially configured (missing google.{missing}) — disabled; fix with \
             `ironclaw config set google.{missing} <value>`"
                )
            })
        }),
    )
}

/// Live OS-service state for the `service` DTO field — see
/// `StatusDto::service`'s doc. Detection failure (unsupported platform, a
/// broken `launchctl`/`systemctl` query) folds to `Unknown` rather than
/// failing `status`: this is diagnostic best-effort, not a hard
/// requirement.
#[cfg(feature = "webui-v2-beta")]
fn resolve_service_state() -> ServiceStateDto {
    let state = crate::commands::service::ServicePlatform::detect()
        .and_then(|platform| platform.current_state());
    match state {
        Ok(crate::commands::service::ServiceState::Running) => ServiceStateDto::Running,
        Ok(crate::commands::service::ServiceState::Stopped) => ServiceStateDto::Stopped,
        Ok(crate::commands::service::ServiceState::NotInstalled) => ServiceStateDto::NotInstalled,
        Err(error) => {
            tracing::debug!(error = %error, "service state detection failed");
            ServiceStateDto::Unknown
        }
    }
}

/// `commands::service` (and the OS-service concept it manages) is gated
/// behind `webui-v2-beta` — a build without it has no service to query.
#[cfg(not(feature = "webui-v2-beta"))]
fn resolve_service_state() -> ServiceStateDto {
    ServiceStateDto::Unknown
}

/// `status` reprints the CLI-token login link `onboard` originally printed
/// (a closed browser loses its `sessionStorage` session, so this is how to
/// get a fresh link without rerunning `onboard`). Reuses the shared
/// `webui_token::resolve_login_link_announcement` resolver rather than
/// re-deriving the host:port/token construction.
///
/// - Returns `(login_link, login_note)`, mutually exclusive: file-sourced
///   token → `(Some(link), None)`; active env var → `(None, Some(note))`
///   (the file-token link's route isn't mounted for an env-sourced token,
///   see `commands::serve::execute`'s `cli_login_mount`); neither → `(None, None)`.
/// - Propagates an error when the token env var is set but not valid
///   UTF-8 — see `webui_token::env_token_is_active` — rather than silently
///   treating it as inactive, which would let `status` disagree with `serve`
///   about which credential source is live.
#[cfg(feature = "webui-v2-beta")]
fn resolve_login_link_and_note(
    home: &ironclaw_reborn_config::RebornHome,
    config_path: &std::path::Path,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    let config_file = ironclaw_reborn_config::RebornConfigFile::load(config_path)?;
    Ok(
        match crate::webui_token::resolve_login_link_announcement(home, config_file.as_ref())? {
            crate::webui_token::LoginLinkAnnouncement::Link(link) => (Some(link), None),
            crate::webui_token::LoginLinkAnnouncement::EnvTokenActive { env_var_name } => (
                None,
                Some(format!(
                    "{env_var_name} is set; serve authenticates with that env token directly (no \
                     login link — the CLI-token login route only mounts for a file-sourced token)"
                )),
            ),
            crate::webui_token::LoginLinkAnnouncement::Unavailable => (None, None),
        },
    )
}

#[cfg(not(feature = "webui-v2-beta"))]
fn resolve_login_link_and_note(
    _home: &ironclaw_reborn_config::RebornHome,
    _config_path: &std::path::Path,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    Ok((None, None))
}

pub(super) fn convert_component_status(status: &RebornRuntimeComponentStatus) -> ComponentStatus {
    match status {
        RebornRuntimeComponentStatus::Initialized => ComponentStatus::Initialized,
        RebornRuntimeComponentStatus::Failed(reason) => ComponentStatus::Failed {
            reason: reason.clone(),
        },
    }
}

impl Renderable for StatusDto {
    fn render_text_to(&self, w: &mut impl Write) -> std::io::Result<()> {
        writeln!(w, "IronClaw Reborn status")?;
        writeln!(w)?;
        kv(w, "version", &self.version)?;
        kv(w, "reborn_home", &self.reborn_home.display().to_string())?;
        kv(w, "home_source", self.home_source)?;
        kv(w, "profile", &self.profile)?;
        kv(
            w,
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
        )?;
        kv(
            w,
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
        )?;
        kv(w, "model_slots", &self.model_slots.join(", "))?;
        kv(w, "service", service_state_text(self.service))?;
        if let Some(login_link) = &self.login_link {
            kv(w, "login_link", login_link)?;
        }
        if let Some(login_note) = &self.login_note {
            kv(w, "login_note", login_note)?;
        }
        if let Some(google_oauth_degraded) = &self.google_oauth_degraded {
            kv(w, "google_oauth", google_oauth_degraded)?;
        }
        writeln!(w)?;
        writeln!(w, "drivers:")?;
        driver_line(w, "  text_only", &self.drivers.text_only)?;
        driver_line(w, "  planned", &self.drivers.planned)?;
        driver_line(w, "  subagent_planned", &self.drivers.subagent_planned)?;
        driver_line(
            w,
            "  planned_default_profile",
            &self.drivers.planned_default_profile,
        )?;
        Ok(())
    }
}

/// Human-readable text for the `service:` status line — matches
/// `commands::service::status_label`'s vocabulary
/// (running/stopped/not installed) plus `unknown` for a build/host where
/// live detection isn't possible.
fn service_state_text(state: ServiceStateDto) -> &'static str {
    match state {
        ServiceStateDto::Running => "running",
        ServiceStateDto::Stopped => "stopped",
        ServiceStateDto::NotInstalled => "not installed",
        ServiceStateDto::Unknown => "unknown",
    }
}

fn driver_line(w: &mut impl Write, label: &str, status: &ComponentStatus) -> std::io::Result<()> {
    match status {
        ComponentStatus::Initialized => writeln!(w, "{label}: initialized"),
        ComponentStatus::Failed { reason } => {
            writeln!(w, "{label}: unavailable ({})", terminal_safe_text(reason))
        }
    }
}

fn kv(w: &mut impl Write, key: &str, value: &str) -> std::io::Result<()> {
    writeln!(w, "{:<20} {value}", format!("{key}:"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::RebornCliContext;
    use ironclaw_reborn_composition::RebornRuntimeComponentStatus;

    #[test]
    fn status_dto_builds_without_config_file() {
        let (_tmp, context) = RebornCliContext::test_context();
        let dto = build_status_dto(&context).expect("must build");
        assert_eq!(dto.version, env!("CARGO_PKG_VERSION"));
        assert!(!dto.model_slots.is_empty());
        assert!(
            dto.login_link.is_none(),
            "no webui-token file exists yet, so there is nothing to link into: {:?}",
            dto.login_link
        );
    }

    /// `status` must reprint the same CLI-token login link `onboard`
    /// printed. Drives `build_status_dto_with_service_state(.., Running)`
    /// rather than `build_status_dto` directly to stay hermetic (no
    /// dependency on the test host's actual OS service install).
    #[cfg(feature = "webui-v2-beta")]
    #[test]
    fn status_dto_includes_login_link_once_a_valid_webui_token_file_exists() {
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        std::fs::write(
            home.path().join("webui-token"),
            "reborn-status-test-token-0123456789abcdef",
        )
        .expect("seed webui-token file");

        let dto = build_status_dto_with_service_state(&context, ServiceStateDto::Running)
            .expect("must build");
        let login_link = dto
            .login_link
            .expect("a valid webui-token file must produce a login link");
        assert!(
            login_link.contains("/login?token=reborn-status-test-token-0123456789abcdef"),
            "login_link must carry the token file's contents: {login_link}"
        );
        assert!(
            login_link.starts_with("http://127.0.0.1:3000/"),
            "login_link must use serve's default host:port: {login_link}"
        );
    }

    /// `status --json` must never leak the bearer token embedded in
    /// `login_link`'s `/login?token=<bearer>` query string; the text output
    /// legitimately prints it, only JSON is redacted. Pinned to `Running`
    /// so `login_link` isn't suppressed, defeating the test's premise.
    #[cfg(feature = "webui-v2-beta")]
    #[test]
    fn status_dto_json_excludes_the_login_link_token() {
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        let token = "reborn-status-json-test-token-0123456789abcdef";
        std::fs::write(home.path().join("webui-token"), token).expect("seed webui-token file");

        let dto = build_status_dto_with_service_state(&context, ServiceStateDto::Running)
            .expect("must build");
        assert!(
            dto.login_link.is_some(),
            "sanity: the DTO must actually carry a login_link to make this test meaningful"
        );

        let json = serde_json::to_string(&dto).expect("StatusDto must serialize");
        assert!(
            !json.contains(token),
            "status --json must not leak the webui bearer token: {json}"
        );
        assert!(
            !json.contains("login_link"),
            "status --json must not emit a login_link field at all: {json}"
        );
    }

    /// Clear every Google OAuth input for a test and restore the caller's
    /// exact process environment (including non-UTF-8 values) on drop.
    fn cleared_google_oauth_env() -> Vec<crate::runtime::test_env::EnvGuard> {
        [
            "IRONCLAW_REBORN_GOOGLE_CLIENT_ID",
            "IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI",
            "IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET",
            "IRONCLAW_REBORN_GOOGLE_HOSTED_DOMAIN_HINT",
            "GOOGLE_CLIENT_ID",
            "GOOGLE_OAUTH_REDIRECT_URI",
            "GOOGLE_CLIENT_SECRET",
            "GOOGLE_ALLOWED_HD",
        ]
        .into_iter()
        .map(crate::runtime::test_env::EnvGuard::clear)
        .collect()
    }

    #[test]
    fn status_dto_google_oauth_degraded_none_when_unconfigured() {
        let _guard = crate::runtime::test_env::lock_runtime_env();
        let _env = cleared_google_oauth_env();

        let (_tmp, context) = RebornCliContext::test_context();
        let dto = build_status_dto(&context).expect("must build");
        assert!(
            dto.google_oauth_degraded.is_none(),
            "no Google OAuth vars set at all must not report a degraded state: {:?}",
            dto.google_oauth_degraded
        );
    }

    /// Caller-level proof that the complete status path does not open the
    /// secret store. A directory at the database path is a deliberate
    /// tripwire: any database open fails, while public-field diagnosis must
    /// still succeed.
    #[cfg(feature = "libsql")]
    #[test]
    fn status_dto_complete_google_config_does_not_open_secret_store() {
        let _guard = crate::runtime::test_env::lock_runtime_env();
        let _env = cleared_google_oauth_env();
        let (_tmp, context) = RebornCliContext::test_context();
        let config = context.boot_config();
        std::fs::create_dir_all(config.home().path()).expect("create reborn home");
        std::fs::write(
            config.home().config_file_path(),
            r#"[google]
client_id = "client.apps.googleusercontent.com"
redirect_uri = "http://127.0.0.1:3000/oauth/google/callback"
"#,
        )
        .expect("write Google config");
        let storage_root = crate::runtime::local_runtime_storage_root(config, config.profile());
        let db_path = ironclaw_reborn_composition::local_dev_db_path(&storage_root);
        std::fs::create_dir_all(&db_path).expect("create database-path tripwire");

        let dto = build_status_dto_with_service_state(&context, ServiceStateDto::Unknown)
            .expect("complete status path must not open the secret store");

        assert!(
            dto.google_oauth_degraded.is_none(),
            "complete public config must not be reported as degraded"
        );
        assert!(
            db_path.is_dir(),
            "status must leave the secret-store tripwire untouched"
        );
    }

    #[test]
    fn status_dto_reports_google_oauth_degraded_when_client_id_present_but_redirect_uri_missing() {
        let _guard = crate::runtime::test_env::lock_runtime_env();
        let _env = cleared_google_oauth_env();
        let _client_id = crate::runtime::test_env::EnvGuard::set(
            "IRONCLAW_REBORN_GOOGLE_CLIENT_ID",
            "reborn-client.apps.googleusercontent.com",
        );

        let (_tmp, context) = RebornCliContext::test_context();
        let dto = build_status_dto(&context).expect("must build");
        let degraded = dto
            .google_oauth_degraded
            .expect("client_id-without-redirect_uri must surface as a degraded status line");
        assert!(
            degraded.contains("redirect_uri"),
            "status line must name the missing key: {degraded}"
        );
        assert!(
            degraded.contains("config set google.redirect_uri"),
            "status line must include the fix command: {degraded}"
        );
    }

    #[test]
    fn status_dto_reports_google_oauth_degraded_when_redirect_uri_present_but_client_id_missing() {
        let _guard = crate::runtime::test_env::lock_runtime_env();
        let _env = cleared_google_oauth_env();
        let _redirect_uri = crate::runtime::test_env::EnvGuard::set(
            "IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI",
            "http://127.0.0.1:3000/api/reborn/product-auth/oauth/google/callback",
        );

        let (_tmp, context) = RebornCliContext::test_context();
        let dto = build_status_dto(&context).expect("must build");
        let degraded = dto
            .google_oauth_degraded
            .expect("redirect_uri-without-client_id must surface as a degraded status line");
        assert!(
            degraded.contains("client_id"),
            "status line must name the missing key: {degraded}"
        );
        assert!(
            degraded.contains("config set google.client_id"),
            "status line must include the fix command: {degraded}"
        );
    }

    #[test]
    fn convert_component_status_failed_maps_correctly() {
        let status = RebornRuntimeComponentStatus::Failed("db connection refused".to_string());
        let result = convert_component_status(&status);
        match result {
            ComponentStatus::Failed { reason } => {
                assert_eq!(reason, "db connection refused");
            }
            ComponentStatus::Initialized => panic!("expected Failed variant"),
        }
    }

    // ── service state: `status` tells the truth (bug fix) ──────────

    /// `Stopped`/`NotInstalled` must always override login_link/login_note;
    /// `Running`/`Unknown` must pass them through unchanged.
    #[test]
    fn apply_service_suppression_overrides_only_when_known_and_not_running() {
        let link = Some("http://127.0.0.1:3000/login?token=t".to_string());
        let note = Some("env token active".to_string());

        for state in [ServiceStateDto::Stopped, ServiceStateDto::NotInstalled] {
            let (suppressed_link, suppressed_note) =
                apply_service_suppression(state, link.clone(), note.clone());
            assert_eq!(
                suppressed_link, None,
                "a known not-running service must suppress login_link ({state:?})"
            );
            assert_eq!(
                suppressed_note.as_deref(),
                Some(SERVICE_NOT_RUNNING_LOGIN_NOTE),
                "a known not-running service must carry the restart guidance ({state:?})"
            );
        }

        for state in [ServiceStateDto::Running, ServiceStateDto::Unknown] {
            let (passthrough_link, passthrough_note) =
                apply_service_suppression(state, link.clone(), note.clone());
            assert_eq!(
                passthrough_link, link,
                "Running/Unknown must not touch login_link ({state:?})"
            );
            assert_eq!(
                passthrough_note, note,
                "Running/Unknown must not touch login_note ({state:?})"
            );
        }
    }

    /// `status --json` must serialize `service` as snake_case, matching this
    /// file's other status/doctor enums (`CheckCategory`, `CheckOutcome`,
    /// `ComponentStatus`) rather than the odd kebab-case one out.
    #[test]
    fn status_dto_json_serializes_service_state_as_snake_case() {
        let (_tmp, context) = RebornCliContext::test_context();
        for (state, expected) in [
            (ServiceStateDto::Running, "\"service\":\"running\""),
            (ServiceStateDto::Stopped, "\"service\":\"stopped\""),
            (
                ServiceStateDto::NotInstalled,
                "\"service\":\"not_installed\"",
            ),
            (ServiceStateDto::Unknown, "\"service\":\"unknown\""),
        ] {
            let dto = build_status_dto_with_service_state(&context, state).expect("must build");
            let json = serde_json::to_string(&dto).expect("StatusDto must serialize");
            assert!(json.contains(expected), "json: {json}");
        }
    }

    /// The text renderer must print a `service:` line for every state,
    /// same vocabulary as `service status`, plus `unknown` when detection
    /// wasn't possible.
    #[test]
    fn status_text_renders_service_line_for_every_state() {
        let (_tmp, context) = RebornCliContext::test_context();
        for (state, expected_value) in [
            (ServiceStateDto::Running, "running"),
            (ServiceStateDto::Stopped, "stopped"),
            (ServiceStateDto::NotInstalled, "not installed"),
            (ServiceStateDto::Unknown, "unknown"),
        ] {
            let dto = build_status_dto_with_service_state(&context, state).expect("must build");
            let mut buf = Vec::new();
            dto.render_text_to(&mut buf).expect("render must succeed");
            let text = String::from_utf8(buf).expect("render output must be UTF-8");
            // contains()-based rather than an exact-column-spacing match: the
            // `kv` column width is a formatting detail this test shouldn't
            // pin, only that the `service:` line carries the right value.
            assert!(
                text.lines()
                    .any(|line| line.trim_start().starts_with("service:")
                        && line.contains(expected_value)),
                "expected a `service:` line containing `{expected_value}`, got:\n{text}"
            );
        }
    }

    /// Once the service state is known not-running, text output must show
    /// restart guidance instead of a (necessarily stale) login link.
    #[cfg(feature = "webui-v2-beta")]
    #[test]
    fn status_text_suppresses_login_link_and_shows_restart_guidance_when_service_stopped() {
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        std::fs::write(
            home.path().join("webui-token"),
            "reborn-status-suppression-test-0123456789abcdef",
        )
        .expect("seed webui-token file");

        let dto = build_status_dto_with_service_state(&context, ServiceStateDto::Stopped)
            .expect("must build");
        assert!(
            dto.login_link.is_none(),
            "a stopped service must suppress the login link even though a valid token file exists"
        );
        assert_eq!(
            dto.login_note.as_deref(),
            Some(SERVICE_NOT_RUNNING_LOGIN_NOTE)
        );

        let mut buf = Vec::new();
        dto.render_text_to(&mut buf).expect("render must succeed");
        let text = String::from_utf8(buf).expect("render output must be UTF-8");
        assert!(!text.contains("login_link:"));
        assert!(text.contains(SERVICE_NOT_RUNNING_LOGIN_NOTE));
    }
}
