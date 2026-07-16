//! `ironclaw-reborn tui` — launches the ratatui thin client against a
//! running (or auto-spawned) `ironclaw-reborn serve`. See
//! `docs/plans/2026-07-15-reborn-tui-service-install-design.md` Part B.

use std::env;
use std::net::IpAddr;
use std::path::Path;
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
                    .unwrap_or_else(|| {
                        IpAddr::from_str(DEFAULT_SERVE_HOST).expect(
                            "DEFAULT_SERVE_HOST is a crate-local literal that parses as IpAddr",
                        )
                    });
                let port = webui_section
                    .and_then(|s| s.listen_port)
                    .unwrap_or(DEFAULT_SERVE_PORT);
                format!("http://{host}:{port}")
            }
        };

        let env_token_var = webui_section
            .and_then(|s| s.env_token_var.as_deref())
            .unwrap_or(DEFAULT_ENV_TOKEN_VAR);
        let token = resolve_tui_token(env_token_var, boot_config.home().path())?;

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

/// Resolve `tui`'s bearer token through the same precedence and entropy
/// validation `serve` uses (`crate::webui_token::resolve_webui_token`):
/// `env_token_var` if set and non-empty, else the `onboard`-provisioned
/// `<reborn_home>/webui-token` fallback file, else a fail-closed error
/// naming both. A tiny impure wrapper around the pure resolver so the
/// real env-reading call site (not just the resolver) is covered by a
/// test.
fn resolve_tui_token(env_token_var: &str, reborn_home: &Path) -> anyhow::Result<String> {
    crate::webui_token::resolve_webui_token(
        env_token_var,
        env::var(env_token_var).ok().as_deref(),
        reborn_home,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_TOKEN: &str = "reborn-smoke-test-token-0123456789abcdef"; // 40 bytes
    const ENV_VAR: &str = "IRONCLAW_REBORN_TUI_TEST_TOKEN_VAR";

    #[test]
    fn resolve_tui_token_falls_back_to_home_file_when_env_unset() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        // SAFETY: `_lock` serializes against every other env-mutating test
        // in this crate's shared test binary.
        unsafe { std::env::remove_var(ENV_VAR) };

        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            crate::webui_token::webui_token_file_path(dir.path()),
            VALID_TOKEN,
        )
        .expect("seed token file");

        let token =
            resolve_tui_token(ENV_VAR, dir.path()).expect("tui must resolve token from file");
        assert_eq!(token, VALID_TOKEN);
    }

    #[test]
    fn resolve_tui_token_prefers_env_when_set() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        // SAFETY: see above.
        unsafe { std::env::set_var(ENV_VAR, VALID_TOKEN) };

        let dir = tempfile::tempdir().expect("tempdir");
        let token = resolve_tui_token(ENV_VAR, dir.path()).expect("env value should resolve");
        assert_eq!(token, VALID_TOKEN);

        // SAFETY: see above.
        unsafe { std::env::remove_var(ENV_VAR) };
    }
}
