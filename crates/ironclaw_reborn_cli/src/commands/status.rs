use clap::Args;
use ironclaw_reborn_composition::{
    RebornRuntimeComponentStatus, reborn_model_slot_names, reborn_runtime_readiness_snapshot,
};

use crate::context::RebornCliContext;
use crate::dto::{ComponentStatus, DriversSnapshot, FilePresence, StatusDto};
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
    let (login_link, login_note) = resolve_login_link_and_note(home, &config_path_for_webui_lookup);

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
    })
}

/// `status` reprints the CLI-token login link `onboard` originally printed
/// — the returning-user story: `sessionStorage` is per-browser-session, so
/// a closed browser needs a fresh link, and `status` is the way to get one
/// without rerunning `onboard`. Reuses the shared
/// `webui_token::resolve_login_link_announcement` resolver (also used by
/// `commands::onboard`) rather than re-deriving the host:port/token
/// construction here — see this repo's shared-resolver convention for
/// auth-adjacent links.
///
/// Returns `(login_link, login_note)`, mutually exclusive: a file-sourced
/// token yields `(Some(link), None)`; an active env var yields
/// `(None, Some(note))` — printing the file-token link in that case would
/// advertise a route `serve` doesn't mount for an env-sourced token (see
/// `commands::serve::execute`'s `cli_login_mount` condition). Neither source
/// available yet yields `(None, None)`.
#[cfg(feature = "webui-v2-beta")]
fn resolve_login_link_and_note(
    home: &ironclaw_reborn_config::RebornHome,
    config_path: &std::path::Path,
) -> (Option<String>, Option<String>) {
    let config_file = ironclaw_reborn_config::RebornConfigFile::load(config_path)
        .ok()
        .flatten();
    match crate::webui_token::resolve_login_link_announcement(home, config_file.as_ref()) {
        crate::webui_token::LoginLinkAnnouncement::Link(link) => (Some(link), None),
        crate::webui_token::LoginLinkAnnouncement::EnvTokenActive { env_var_name } => (
            None,
            Some(format!(
                "{env_var_name} is set; serve authenticates with that env token directly (no \
                 login link — the CLI-token login route only mounts for a file-sourced token)"
            )),
        ),
        crate::webui_token::LoginLinkAnnouncement::Unavailable => (None, None),
    }
}

#[cfg(not(feature = "webui-v2-beta"))]
fn resolve_login_link_and_note(
    _home: &ironclaw_reborn_config::RebornHome,
    _config_path: &std::path::Path,
) -> (Option<String>, Option<String>) {
    (None, None)
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
        if let Some(login_link) = &self.login_link {
            kv(w, "login_link", login_link)?;
        }
        if let Some(login_note) = &self.login_note {
            kv(w, "login_note", login_note)?;
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

    /// RED (B4 step 6): `status` must reprint the same CLI-token login link
    /// `onboard` printed — the returning-user story (see `resolve_login_link`'s
    /// doc: `sessionStorage` is per-browser-session, so a closed browser needs
    /// a fresh link without rerunning `onboard`).
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

        let dto = build_status_dto(&context).expect("must build");
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

    /// Security regression: `status --json` (`serde_json::to_string` over
    /// `StatusDto`) must never leak the bearer token embedded in
    /// `login_link`'s `/login?token=<bearer>` query string. The human
    /// `status` text output legitimately prints it (see
    /// `render_text_to` above); only the JSON/diagnostic path is redacted.
    #[cfg(feature = "webui-v2-beta")]
    #[test]
    fn status_dto_json_excludes_the_login_link_token() {
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        let token = "reborn-status-json-test-token-0123456789abcdef";
        std::fs::write(home.path().join("webui-token"), token).expect("seed webui-token file");

        let dto = build_status_dto(&context).expect("must build");
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
}
