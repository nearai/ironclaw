use std::env;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, anyhow};
use clap::Args;
use ironclaw_reborn_composition::{
    RebornReadiness, RebornWebuiBundle, WebuiServeConfig, build_reborn_runtime,
    build_webui_services, webui_v2_app,
};
use ironclaw_reborn_webui_ingress::{
    EnvBearerAuthenticator, RebornWebuiServeOptions, serve_webui_v2,
};
use secrecy::SecretString;

use crate::context::RebornCliContext;

const DEFAULT_SERVE_HOST: &str = "127.0.0.1";
const DEFAULT_SERVE_PORT: u16 = 3000;
const DEFAULT_ENV_TOKEN_VAR: &str = "IRONCLAW_REBORN_WEBUI_TOKEN";
const DEFAULT_ENV_USER_ID_VAR: &str = "IRONCLAW_REBORN_WEBUI_USER_ID";

#[derive(Debug, Args)]
pub(crate) struct ServeCommand {
    /// Host interface for the Reborn WebChat v2 HTTP listener.
    /// Overrides `[webui].listen_host` from the boot config file.
    #[arg(long, default_value = DEFAULT_SERVE_HOST)]
    host: IpAddr,

    /// Port for the Reborn WebChat v2 HTTP listener. `0` lets the
    /// kernel pick a free port (useful for tests). Overrides
    /// `[webui].listen_port` from the boot config file.
    #[arg(long, default_value_t = DEFAULT_SERVE_PORT)]
    port: u16,
}

impl ServeCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        // Build the runtime config from the operator's TOML.
        let runtime_input = crate::runtime::build_runtime_input(context.boot_config())?;
        let boot_config = context.boot_config();
        let config_file =
            ironclaw_reborn_config::RebornConfigFile::load(&boot_config.home().config_file_path())
                .map_err(anyhow::Error::from)?;

        // Tenant id is host-trusted (operator-owned config), never
        // browser-influenced. Falls back to the same default the CLI's
        // `run` command uses.
        let tenant_raw = config_file
            .as_ref()
            .and_then(|file| file.identity.as_ref())
            .and_then(|identity| identity.tenant.as_deref())
            .unwrap_or("reborn-cli");
        let tenant_id = ironclaw_reborn_composition::host_api::TenantId::new(tenant_raw)
            .map_err(|err| anyhow!("[identity].tenant `{tenant_raw}` is invalid: {err}"))?;

        // Resolve env-bearer authenticator from the env-var names the
        // operator declared in `[webui]`. Values themselves are env-only
        // (the `secrets_guard` check rejects inline secrets at config
        // parse).
        let webui_section = config_file.as_ref().and_then(|file| file.webui.as_ref());
        let env_token_var = webui_section
            .and_then(|section| section.env_token_var.as_deref())
            .unwrap_or(DEFAULT_ENV_TOKEN_VAR);
        let env_user_id_var = webui_section
            .and_then(|section| section.env_user_id_var.as_deref())
            .unwrap_or(DEFAULT_ENV_USER_ID_VAR);

        let token_value = env::var(env_token_var).map_err(|_| {
            anyhow!(
                "{env_token_var} must be set to the WebChat v2 bearer token. \
                 Override the variable name via `[webui].env_token_var` in {}.",
                boot_config.home().config_file_path().display(),
            )
        })?;
        let user_id_raw = env::var(env_user_id_var).map_err(|_| {
            anyhow!(
                "{env_user_id_var} must be set to the UserId an env-bearer-authenticated caller maps to. \
                 Override the variable name via `[webui].env_user_id_var` in {}.",
                boot_config.home().config_file_path().display(),
            )
        })?;
        let user_id = ironclaw_reborn_composition::host_api::UserId::new(&user_id_raw)
            .map_err(|err| anyhow!("{env_user_id_var} value `{user_id_raw}` is invalid: {err}"))?;

        let authenticator = Arc::new(EnvBearerAuthenticator::new(
            SecretString::from(token_value),
            user_id,
        )?);

        // Resolve listen address. CLI flags WIN over config file (consistent
        // with `compiled defaults < config file < env vars < CLI flags`).
        let host = if matches_cli_default_host(self.host) {
            webui_section
                .and_then(|section| section.listen_host.as_deref())
                .map(IpAddr::from_str)
                .transpose()
                .map_err(|err| anyhow!("[webui].listen_host invalid: {err}"))?
                .unwrap_or(self.host)
        } else {
            self.host
        };
        let port = if self.port == DEFAULT_SERVE_PORT {
            webui_section
                .and_then(|section| section.listen_port)
                .unwrap_or(self.port)
        } else {
            self.port
        };
        let listen_addr = SocketAddr::new(host, port);

        // CORS allow-origin list. Empty = fail-closed on every
        // cross-origin preflight; operators MUST opt in to the
        // specific origins the host installation actually serves.
        let allowed_origins_raw = webui_section
            .and_then(|section| section.allowed_origins.as_ref())
            .cloned()
            .unwrap_or_default();
        let allowed_origins = WebuiServeConfig::parse_allowed_origins(&allowed_origins_raw)
            .map_err(|err| anyhow!("[webui].allowed_origins parse failure: {err}"))?;

        let csp_override = webui_section.and_then(|section| section.csp_header_override.as_deref());

        let max_body_bytes_fallback = webui_section
            .and_then(|section| section.max_body_bytes_fallback)
            .map(|raw| {
                if raw == 0 {
                    Err(anyhow!("[webui].max_body_bytes_fallback must be > 0"))
                } else {
                    usize::try_from(raw)
                        .map_err(|_| anyhow!("[webui].max_body_bytes_fallback exceeds usize"))
                }
            })
            .transpose()?;

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("failed to build tokio runtime for `serve`")?;

        rt.block_on(async move {
            let runtime = build_reborn_runtime(runtime_input)
                .await
                .context("failed to assemble Reborn runtime for `serve`")?;
            let bundle: RebornWebuiBundle = build_webui_services(&runtime, None)?;

            print_serve_banner(
                listen_addr,
                env_token_var,
                env_user_id_var,
                &allowed_origins_raw,
                &bundle.readiness,
            );

            let mut serve_config = WebuiServeConfig::new(tenant_id, authenticator, allowed_origins);
            if let Some(value) = csp_override {
                serve_config = serve_config
                    .with_csp_header_str(value)
                    .map_err(|err| anyhow!("[webui].csp_header_override invalid: {err}"))?;
            }
            if let Some(value) = max_body_bytes_fallback {
                serve_config = serve_config.with_max_body_bytes(value);
            }
            let router =
                webui_v2_app(bundle, serve_config).context("failed to compose v2 Router")?;

            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
            tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    tracing::info!(
                        target = "ironclaw::reborn::cli::serve",
                        "ctrl-c received; signalling WebChat v2 graceful shutdown",
                    );
                    let _ = shutdown_tx.send(());
                }
            });

            let serve_result = serve_webui_v2(RebornWebuiServeOptions {
                addr: listen_addr,
                router,
                shutdown: shutdown_rx,
                bound_addr_tx: None,
            })
            .await;

            // Always drain the Reborn runtime, even on serve error, so
            // background tasks and turn-runner state shut down cleanly.
            let shutdown_result = runtime.shutdown().await;
            serve_result.context("WebChat v2 serve loop failed")?;
            shutdown_result.context("Reborn runtime shutdown failed")?;
            Ok::<(), anyhow::Error>(())
        })?;

        Ok(())
    }
}

fn matches_cli_default_host(value: IpAddr) -> bool {
    value == IpAddr::from_str(DEFAULT_SERVE_HOST).expect("crate-local literal parses") // safety: DEFAULT_SERVE_HOST is a compile-time `&'static str` known to parse as IpAddr; failure here would be a build-time bug
}

fn print_serve_banner(
    listen_addr: SocketAddr,
    env_token_var: &str,
    env_user_id_var: &str,
    allowed_origins: &[String],
    readiness: &RebornReadiness,
) {
    eprintln!("ironclaw-reborn: WebChat v2 listener");
    eprintln!("  binary    : ironclaw-reborn");
    eprintln!("  version   : {}", env!("CARGO_PKG_VERSION"));
    eprintln!("  listen    : http://{listen_addr}");
    eprintln!("  auth      : env-bearer (token ${env_token_var}, user ${env_user_id_var})");
    if allowed_origins.is_empty() {
        eprintln!("  cors      : fail-closed (no allowed origins configured)");
    } else {
        eprintln!(
            "  cors      : {} origin(s) ({})",
            allowed_origins.len(),
            allowed_origins.join(", "),
        );
    }
    eprintln!("  readiness : {readiness:?}");
    eprintln!();
}
