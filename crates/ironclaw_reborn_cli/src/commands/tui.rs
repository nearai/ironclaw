//! `ironclaw-reborn tui` — launches the ratatui thin client against a
//! running (or auto-spawned) `ironclaw-reborn serve`. See
//! `docs/plans/2026-07-15-reborn-tui-service-install-design.md` Part B.

use std::env;
use std::net::IpAddr;
use std::str::FromStr;

use anyhow::anyhow;
use clap::Args;
use ironclaw_reborn_tui::{TuiConfig, run_tui};

use crate::commands::serve::{DEFAULT_ENV_TOKEN_VAR, DEFAULT_SERVE_HOST, DEFAULT_SERVE_PORT};
use crate::context::RebornCliContext;

#[derive(Debug, Args)]
pub(crate) struct TuiCommand {
    /// Base URL of an already-running `ironclaw-reborn serve`. When
    /// omitted, resolved the same way `serve` resolves its own listener
    /// (CLI flag > `[webui]` config > compiled default), then auto-spawned
    /// if unreachable.
    #[arg(long)]
    base_url: Option<String>,
}

impl TuiCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let boot_config = context.boot_config();
        let config_file =
            ironclaw_reborn_config::RebornConfigFile::load(&boot_config.home().config_file_path())
                .map_err(anyhow::Error::from)?;
        let webui_section = config_file.as_ref().and_then(|f| f.webui.as_ref());

        let base_url = match self.base_url {
            Some(url) => url,
            None => {
                let host: IpAddr = webui_section
                    .and_then(|s| s.listen_host.as_deref())
                    .map(IpAddr::from_str)
                    .transpose()
                    .map_err(|err| anyhow!("[webui].listen_host invalid: {err}"))?
                    .unwrap_or_else(|| IpAddr::from_str(DEFAULT_SERVE_HOST).expect("literal"));
                let port = webui_section
                    .and_then(|s| s.listen_port)
                    .unwrap_or(DEFAULT_SERVE_PORT);
                format!("http://{host}:{port}")
            }
        };

        let env_token_var = webui_section
            .and_then(|s| s.env_token_var.as_deref())
            .unwrap_or(DEFAULT_ENV_TOKEN_VAR);
        let token = env::var(env_token_var).map_err(|_| {
            anyhow!(
                "{env_token_var} must be set to the WebChat v2 bearer token (same variable `serve` reads)."
            )
        })?;

        let spawn = Some(to_process_invocation(
            crate::serve_invocation::serve_invocation()?,
        ));

        crate::runtime::block_on_cli(async move {
            run_tui(TuiConfig {
                base_url,
                token,
                spawn,
            })
            .await
        })
    }
}

fn to_process_invocation(
    inv: crate::serve_invocation::ServeInvocation,
) -> ironclaw_reborn_tui::spawn::ProcessInvocation {
    ironclaw_reborn_tui::spawn::ProcessInvocation {
        exe: inv.exe,
        args: inv.args,
        env: inv.env,
    }
}
