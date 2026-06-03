use std::env;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, anyhow};
use clap::Args;
use ironclaw_reborn_composition::{
    GoogleOAuthRouteConfig, RebornBuildInput, RebornReadiness, RebornRuntimeIdentity,
    RebornRuntimeInput, RebornWebuiBundle, WebuiServeConfig, build_reborn_runtime,
    build_webui_services, webui_v2_app_with_lifecycle,
};
use ironclaw_reborn_config::IdentitySection;
use ironclaw_reborn_webui_ingress::{
    EnvBearerAuthenticator, RebornWebuiServeOptions, serve_webui_v2,
};
use secrecy::SecretString;

use crate::context::RebornCliContext;
use crate::runtime::{RuntimeInputOptions, resolve_google_oauth_config_from_env};

const DEFAULT_SERVE_HOST: &str = "127.0.0.1";
const DEFAULT_SERVE_PORT: u16 = 3000;
const DEFAULT_ENV_TOKEN_VAR: &str = "IRONCLAW_REBORN_WEBUI_TOKEN";
const DEFAULT_ENV_USER_ID_VAR: &str = "IRONCLAW_REBORN_WEBUI_USER_ID";

#[derive(Debug, Args)]
pub(crate) struct ServeCommand {
    /// Host interface for the Reborn WebChat v2 HTTP listener.
    /// Overrides `[webui].listen_host` from the boot config file.
    /// Default (when neither is set) is `127.0.0.1`.
    //
    // Stored as `Option<IpAddr>` (no clap default) so the precedence
    // chain `CLI > config > constant default` can be resolved
    // explicitly. A clap default would conflate "operator passed
    // 127.0.0.1 explicitly" with "operator omitted the flag", which
    // would incorrectly let a config-supplied 0.0.0.0 win over an
    // explicit --host 127.0.0.1.
    #[arg(long)]
    host: Option<IpAddr>,

    /// Port for the Reborn WebChat v2 HTTP listener. `0` lets the
    /// kernel pick a free port (useful for tests). Overrides
    /// `[webui].listen_port` from the boot config file. Default
    /// (when neither is set) is 3000.
    #[arg(long)]
    port: Option<u16>,

    /// Confirm trusted-laptop host filesystem access for local-dev-yolo.
    #[arg(long = "confirm-host-access")]
    confirm_host_access: bool,
}

impl ServeCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        crate::runtime::init_tracing();

        // Build the runtime config from the operator's TOML. Built first so
        // the local-dev-yolo host-access disclosure gate fires before any
        // WebUI env-var resolution below; the owner is aligned to the
        // authenticated WebUI user once it is resolved (see `with_owner_id`).
        let runtime_input = crate::runtime::build_runtime_input_with_options(
            context.boot_config(),
            crate::runtime::RuntimeInputCaller::Serve,
            RuntimeInputOptions {
                confirm_host_access: self.confirm_host_access,
            },
        )?;
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

        // Resolve trusted host-installation default agent/project from
        // `[identity]`. The v2 facade builds `ThreadScope` from
        // `caller.agent_id` on every mutation and read, so an absent
        // default_agent here means every authenticated request would
        // still 400. Mirror the same fallback rule the `run` command
        // uses: identity.default_agent or composition's default.
        let identity_section = config_file.as_ref().and_then(|file| file.identity.as_ref());

        // Pin the runtime owner to the authenticated WebUI user so the
        // turn-runner loop host reads thread context from the same
        // `owners/<user>` subtree the v2 facade wrote to. Without this the
        // runtime owner stays at `[identity].default_owner` (a different
        // identity source) and every turn fails with `UnknownThread`.
        let runtime_owner = resolve_webui_runtime_owner(identity_section, &user_id_raw)?;
        let mut runtime_input = runtime_input.with_owner_id(runtime_owner);
        let default_agent_raw =
            resolve_webui_default_agent(identity_section, &runtime_input.identity);
        let default_agent_id =
            ironclaw_reborn_composition::host_api::AgentId::new(&default_agent_raw).map_err(
                |err| anyhow!("[identity].default_agent `{default_agent_raw}` is invalid: {err}"),
            )?;
        let default_project_id = identity_section
            .and_then(|identity| identity.default_project.as_deref())
            .map(ironclaw_reborn_composition::host_api::ProjectId::new)
            .transpose()
            .map_err(|err| anyhow!("[identity].default_project is invalid: {err}"))?;

        // Resolve listen address with explicit precedence:
        //   CLI flag (Some(...)) > config file > compile-time default.
        // Both `host` and `port` are `Option<>` in the clap struct so
        // we can distinguish "operator omitted the flag" from "operator
        // passed the default value explicitly".
        let host: IpAddr = if let Some(value) = self.host {
            value
        } else if let Some(raw) = webui_section.and_then(|s| s.listen_host.as_deref()) {
            IpAddr::from_str(raw)
                .map_err(|err| anyhow!("[webui].listen_host `{raw}` invalid: {err}"))?
        } else {
            IpAddr::from_str(DEFAULT_SERVE_HOST)
                .expect("DEFAULT_SERVE_HOST is a crate-local literal that parses as IpAddr") // safety: crate-local const known to be valid
        };
        // `port = 0` would tell the OS to pick a free port — useful
        // when invoked from a test harness with `--port 0`, but in a
        // config file it produces a running server whose real bound
        // port is never reported back to the operator (the banner
        // prints `:0`). Allow `--port 0` from the CLI flag, reject
        // `0` from `[webui].listen_port`.
        let port: u16 = if let Some(value) = self.port {
            value
        } else if let Some(value) = webui_section.and_then(|s| s.listen_port) {
            if value == 0 {
                anyhow::bail!(
                    "[webui].listen_port = 0 from config is not supported: the OS would pick \
                     an ephemeral port and the startup banner cannot report it. Set a fixed \
                     port in config, or pass `--port 0` on the CLI when you genuinely want \
                     an ephemeral port (the banner output is still :0 in that case — the \
                     bound address is only useful when consumed through a test harness)."
                );
            }
            value
        } else {
            DEFAULT_SERVE_PORT
        };
        // Canonical host for WS same-origin check (defense against
        // reverse-proxy passthrough-Host attacks). Validate as
        // `host` or `host:port` — refuse multi-segment paths or
        // scheme prefixes which would silently never match Origin.
        let canonical_host = webui_section
            .and_then(|section| section.canonical_host.as_deref())
            .map(|raw| -> anyhow::Result<String> {
                if raw.is_empty() {
                    anyhow::bail!("[webui].canonical_host must not be empty");
                }
                if raw.contains("://") {
                    anyhow::bail!(
                        "[webui].canonical_host `{raw}` must be `host` or `host:port`, \
                         not a scheme-qualified URL",
                    );
                }
                if raw.contains('/') {
                    anyhow::bail!("[webui].canonical_host `{raw}` must not contain `/`",);
                }
                Ok(raw.to_string())
            })
            .transpose()?;

        let listen_addr = SocketAddr::new(host, port);
        reject_non_loopback_privileged_local_runtime(host, &runtime_input)?;
        if let Some(callback_origin) =
            webui_oauth_callback_origin(listen_addr, canonical_host.as_deref())
        {
            let services = runtime_input.services.take().ok_or_else(|| {
                anyhow!("WebChat v2 serve requires Reborn runtime services before OAuth wiring")
            })?;
            runtime_input.services = Some(
                with_notion_dcr_oauth_backend(services, &callback_origin)
                    .context("failed to configure Notion DCR OAuth for WebChat v2")?,
            );
        } else {
            tracing::warn!(
                target = "ironclaw::reborn::cli::serve",
                %listen_addr,
                "Notion DCR OAuth is not configured because the WebChat v2 listener origin is not a stable loopback HTTP origin"
            );
        }

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

        // Loud warning when binding to a non-loopback interface. The
        // env-bearer authenticator is fine for trusted operator-only
        // deployments, but a public listener with a single env-token
        // is a foot-gun. Operators can silence by setting
        // `--host 0.0.0.0` explicitly (we don't have a "yes I mean
        // it" flag yet — this is purely an attention nudge).
        if !host.is_loopback() {
            eprintln!(
                "WARNING: WebChat v2 listener will bind to non-loopback address {host}. \
                 The default env-bearer authenticator is intended for single-operator \
                 deployments; review your auth config before exposing this to a network."
            );
        }
        // Also emit a structured log so operators with log aggregation
        // see the same signal.
        if !host.is_loopback() {
            tracing::warn!(
                target = "ironclaw::reborn::cli::serve",
                %host,
                "binding WebChat v2 listener on a non-loopback interface",
            );
        }

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

            let mut serve_config = WebuiServeConfig::new(tenant_id, authenticator, allowed_origins)
                .with_default_agent_id(default_agent_id);
            if let Some(project_id) = default_project_id {
                serve_config = serve_config.with_default_project_id(project_id);
            }
            if let Some(google_oauth) = resolve_google_oauth_config_from_env()
                .context("failed to resolve Google OAuth setup config for WebUI")?
            {
                let mut route_config = GoogleOAuthRouteConfig::new(
                    google_oauth.client.client_id.as_str(),
                    google_oauth.client.redirect_uri.as_str(),
                )
                .context("invalid Google OAuth route config for WebUI")?;
                if let Some(hosted_domain_hint) = google_oauth.hosted_domain_hint {
                    route_config = route_config
                        .with_hosted_domain_hint(hosted_domain_hint)
                        .context("invalid Google OAuth hosted-domain hint for WebUI")?;
                }
                serve_config = serve_config.with_google_oauth(route_config);
            }
            if let Some(value) = csp_override {
                serve_config = serve_config
                    .with_csp_header_str(value)
                    .map_err(|err| anyhow!("[webui].csp_header_override invalid: {err}"))?;
            }
            if let Some(value) = max_body_bytes_fallback {
                serve_config = serve_config.with_max_body_bytes(value);
            }
            if let Some(host) = canonical_host {
                serve_config = serve_config.with_canonical_host(host);
            }
            let webui_app = webui_v2_app_with_lifecycle(bundle, serve_config)
                .context("failed to compose v2 Router")?;
            let (router, public_route_drains) = webui_app.into_parts();

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

            // Always drain public route mounts before shutting down the
            // Reborn runtime. Protocol webhooks such as Slack can ACK a
            // request before product workflow dispatch completes, so their
            // route-owned work must finish after ingress stops accepting new
            // requests but before shared runtime services are torn down.
            public_route_drains.drain().await;

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

fn reject_non_loopback_privileged_local_runtime(
    host: IpAddr,
    runtime_input: &RebornRuntimeInput,
) -> anyhow::Result<()> {
    if host.is_loopback() || !runtime_input.grants_trusted_laptop_access() {
        return Ok(());
    }

    anyhow::bail!(
        "`ironclaw-reborn serve` refuses non-loopback listener {host} because the selected \
         runtime policy grants trusted-laptop host access (host-home filesystem, local host \
         process, direct network, inherited environment). Bind to a loopback host such as \
         127.0.0.1 or ::1, or choose a less privileged profile."
    );
}

fn with_notion_dcr_oauth_backend(
    services: RebornBuildInput,
    callback_origin: &str,
) -> anyhow::Result<RebornBuildInput> {
    // Provider-visible DCR client display name shown during Notion OAuth consent.
    services
        .with_notion_dcr_oauth_backend(callback_origin, "Ironclaw")
        .map_err(|error| anyhow!("Notion DCR OAuth backend rejected callback origin: {error}"))
}

fn webui_oauth_callback_origin(
    listen_addr: SocketAddr,
    canonical_host: Option<&str>,
) -> Option<String> {
    if let Some(host) = canonical_host {
        return Some(format!(
            "{}://{}",
            callback_origin_scheme(host),
            canonical_host_for_origin_url(host)
        ));
    }

    let port = listen_addr.port();
    if port == 0 {
        return None;
    }
    match listen_addr.ip() {
        IpAddr::V4(host) if host.is_unspecified() => Some(format!("http://localhost:{port}")),
        IpAddr::V6(host) if host.is_unspecified() => Some(format!("http://localhost:{port}")),
        IpAddr::V4(host) if host.is_loopback() => Some(format!("http://{host}:{port}")),
        IpAddr::V6(host) if host.is_loopback() => Some(format!("http://[{host}]:{port}")),
        _ => None,
    }
}

fn callback_origin_scheme(host: &str) -> &'static str {
    if canonical_host_is_loopback(host) {
        "http"
    } else {
        "https"
    }
}

fn canonical_host_is_loopback(host: &str) -> bool {
    let host_name = canonical_host_name(host);
    host_name == "localhost"
        || host_name
            .parse::<IpAddr>()
            .is_ok_and(|host| host.is_loopback())
}

fn canonical_host_for_origin_url(host: &str) -> String {
    if host.starts_with('[') {
        return host.to_string();
    }
    if matches!(host.parse::<IpAddr>(), Ok(IpAddr::V6(_))) {
        return format!("[{host}]");
    }
    host.to_string()
}

fn canonical_host_name(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split_once(']').map(|(host, _)| host).unwrap_or(host);
    }
    if host.parse::<IpAddr>().is_ok() {
        return host;
    }
    host.split_once(':').map(|(host, _)| host).unwrap_or(host)
}

fn resolve_webui_default_agent(
    identity_section: Option<&IdentitySection>,
    runtime_identity: &RebornRuntimeIdentity,
) -> String {
    identity_section
        .and_then(|identity| identity.default_agent.clone())
        .unwrap_or_else(|| runtime_identity.agent_id.clone())
}

/// Resolve the owner the Reborn runtime must run under for the WebChat v2
/// serve path.
///
/// The v2 facade writes and reads threads under a `ThreadScope` whose
/// `owner_user_id` is the authenticated WebUI user, while the turn-runner
/// loop host reads thread context under the runtime's composition owner. If
/// those two identities diverge, `ThreadScope::to_resource_scope` resolves a
/// different `/tenants/<t>/users/<u>/` MountView for the read than the write,
/// so the loop host silently looks in the wrong `owners/<user>` subtree and
/// every turn fails with `UnknownThread` -> `HostUnavailable { Prompt }`.
///
/// The runtime owner is therefore pinned to the authenticated WebUI user. A
/// `[identity].default_owner` that contradicts that user is rejected loudly
/// rather than silently producing thread-invisible turns.
fn resolve_webui_runtime_owner(
    identity_section: Option<&IdentitySection>,
    webui_user_id: &str,
) -> anyhow::Result<String> {
    if let Some(configured) =
        identity_section.and_then(|identity| identity.default_owner.as_deref())
        && configured != webui_user_id
    {
        return Err(anyhow!(
            "[identity].default_owner `{configured}` must match the WebChat v2 \
             authenticated user `{webui_user_id}`. A mismatch makes every thread \
             created through the WebUI invisible to the turn runner, because the \
             loop host reads thread context under the runtime owner, not the WebUI \
             user. Remove `[identity].default_owner` or set it to `{webui_user_id}`."
        ));
    }
    Ok(webui_user_id.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webui_default_agent_falls_back_to_runtime_identity() {
        let runtime_identity = RebornRuntimeIdentity::reborn_cli();

        assert_eq!(
            resolve_webui_default_agent(None, &runtime_identity),
            "reborn-cli-agent"
        );
    }

    #[test]
    fn webui_default_agent_uses_config_override() {
        let runtime_identity = RebornRuntimeIdentity::reborn_cli();
        let identity = IdentitySection {
            default_agent: Some("configured-agent".to_string()),
            ..IdentitySection::default()
        };

        assert_eq!(
            resolve_webui_default_agent(Some(&identity), &runtime_identity),
            "configured-agent"
        );
    }

    #[test]
    fn webui_runtime_owner_defaults_to_authenticated_user() {
        // With no `[identity].default_owner`, the runtime owner must be the
        // authenticated WebUI user so the turn-runner loop host reads thread
        // context from the same `owners/<user>` subtree the v2 facade wrote.
        assert_eq!(
            resolve_webui_runtime_owner(None, "local-user").unwrap(),
            "local-user"
        );
    }

    #[test]
    fn webui_runtime_owner_accepts_matching_config_owner() {
        let identity = IdentitySection {
            default_owner: Some("local-user".to_string()),
            ..IdentitySection::default()
        };

        assert_eq!(
            resolve_webui_runtime_owner(Some(&identity), "local-user").unwrap(),
            "local-user"
        );
    }

    #[test]
    fn webui_runtime_owner_rejects_divergent_config_owner() {
        // A configured owner that differs from the authenticated WebUI user is
        // the bug class that silently made every thread invisible: the facade
        // writes under `owners/local-user` while the loop host reads under
        // `owners/reborn-cli`. Fail loud at startup instead.
        let identity = IdentitySection {
            default_owner: Some("reborn-cli".to_string()),
            ..IdentitySection::default()
        };

        let error = resolve_webui_runtime_owner(Some(&identity), "local-user")
            .expect_err("divergent owner must be rejected");
        let message = error.to_string();
        assert!(message.contains("reborn-cli"), "message: {message}");
        assert!(message.contains("local-user"), "message: {message}");
    }

    #[test]
    fn webui_oauth_callback_origin_uses_loopback_http() {
        assert_eq!(
            webui_oauth_callback_origin(SocketAddr::from(([127, 0, 0, 1], 3000)), None).as_deref(),
            Some("http://127.0.0.1:3000")
        );
    }

    #[test]
    fn webui_oauth_callback_origin_maps_unspecified_bind_to_localhost() {
        assert_eq!(
            webui_oauth_callback_origin(SocketAddr::from(([0, 0, 0, 0], 3000)), None).as_deref(),
            Some("http://localhost:3000")
        );
    }

    #[test]
    fn webui_oauth_callback_origin_brackets_ipv6_loopback() {
        let listen_addr = SocketAddr::new(IpAddr::from_str("::1").unwrap(), 3000);

        assert_eq!(
            webui_oauth_callback_origin(listen_addr, None).as_deref(),
            Some("http://[::1]:3000")
        );
    }

    #[test]
    fn webui_oauth_callback_origin_skips_unstable_or_non_loopback_origin() {
        assert_eq!(
            webui_oauth_callback_origin(SocketAddr::from(([127, 0, 0, 1], 0)), None),
            None
        );
        assert_eq!(
            webui_oauth_callback_origin(SocketAddr::from(([192, 168, 1, 42], 3000)), None),
            None
        );
    }

    #[test]
    fn webui_oauth_callback_origin_uses_https_canonical_host() {
        assert_eq!(
            webui_oauth_callback_origin(
                SocketAddr::from(([0, 0, 0, 0], 3000)),
                Some("app.example.com"),
            )
            .as_deref(),
            Some("https://app.example.com")
        );
    }

    #[test]
    fn webui_oauth_callback_origin_uses_http_for_loopback_canonical_host() {
        assert_eq!(
            webui_oauth_callback_origin(
                SocketAddr::from(([0, 0, 0, 0], 3000)),
                Some("127.0.0.1:3000"),
            )
            .as_deref(),
            Some("http://127.0.0.1:3000")
        );
    }

    #[test]
    fn webui_oauth_callback_origin_brackets_ipv6_canonical_host() {
        assert_eq!(
            webui_oauth_callback_origin(SocketAddr::from(([0, 0, 0, 0], 3000)), Some("::1"))
                .as_deref(),
            Some("http://[::1]")
        );
    }

    #[tokio::test]
    async fn webui_serve_wires_notion_dcr_into_runtime_services() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services_input = with_notion_dcr_oauth_backend(
            RebornBuildInput::local_dev("notion-dcr-owner", dir.path().join("local-dev")),
            "http://127.0.0.1:3000",
        )
        .expect("notion dcr wiring");
        let services = ironclaw_reborn_composition::build_reborn_services(services_input)
            .await
            .expect("reborn services build");

        assert!(
            services
                .product_auth
                .as_ref()
                .and_then(|product_auth| product_auth.as_auth_challenge_provider())
                .is_some(),
            "serve wiring must expose the DCR-backed auth challenge provider"
        );
    }

    #[tokio::test]
    async fn webui_serve_wires_notion_dcr_with_canonical_host_origin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services_input = with_notion_dcr_oauth_backend(
            RebornBuildInput::local_dev("notion-dcr-owner", dir.path().join("local-dev")),
            webui_oauth_callback_origin(
                SocketAddr::from(([0, 0, 0, 0], 3000)),
                Some("app.example.com"),
            )
            .as_deref()
            .expect("canonical callback origin"),
        )
        .expect("notion dcr wiring");
        let services = ironclaw_reborn_composition::build_reborn_services(services_input)
            .await
            .expect("reborn services build");

        assert!(
            services
                .product_auth
                .as_ref()
                .and_then(|product_auth| product_auth.as_auth_challenge_provider())
                .is_some(),
            "serve wiring must expose the DCR-backed auth challenge provider"
        );
    }
}
