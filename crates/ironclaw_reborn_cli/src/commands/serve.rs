use std::env;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, anyhow};
use clap::Args;
#[cfg(feature = "openai-compat-beta")]
use ironclaw_reborn_composition::build_openai_compat_route_mount;
#[cfg(not(feature = "slack-v2-host-beta"))]
use ironclaw_reborn_composition::build_webui_services;
use ironclaw_reborn_composition::host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_reborn_composition::{
    GoogleOAuthRouteConfig, LocalTriggerAccessReconciliation, LocalTriggerAccessRole,
    LocalTriggerAccessSource, RebornBuildInput, RebornReadiness, RebornRuntimeIdentity,
    RebornRuntimeInput, RebornWebuiBundle, WebuiAuthenticator, WebuiServeConfig,
    build_reborn_runtime, open_local_trigger_access_store, webui_v2_app_with_lifecycle,
};
#[cfg(feature = "slack-v2-host-beta")]
use ironclaw_reborn_composition::{
    SlackOperatorRouteVisibility, build_slack_host_beta_runtime_mounts,
    build_webui_services_with_slack_host_beta_mounts,
};
use ironclaw_reborn_config::{IdentitySection, RebornProfile, seed_default_config_file_if_missing};
use ironclaw_reborn_webui_ingress::{
    DeferredWebuiRouterHandle, EnvBearerAuthenticator, RebornWebuiServeError,
    RebornWebuiServeOptions, deferred_webui_v2_startup_router, serve_webui_v2,
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
        let tenant_id = TenantId::new(tenant_raw)
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
        let user_id = UserId::new(&user_id_raw)
            .map_err(|err| anyhow!("{env_user_id_var} value `{user_id_raw}` is invalid: {err}"))?;

        // Keep a copy of the operator secret to key the SSO session-token
        // HMAC before the value is moved into the env-bearer authenticator.
        // Held as `SecretString` so it is redacted in `Debug`/logs and
        // zeroed on drop — it doubles as the session-signing key. Capture
        // its byte length first (for the SSO entropy floor below) since the
        // value is consumed here.
        let token_byte_len = token_value.len();
        let session_signing_secret = SecretString::from(token_value.clone());
        let env_authenticator: Arc<dyn WebuiAuthenticator> = Arc::new(EnvBearerAuthenticator::new(
            SecretString::from(token_value),
            user_id.clone(),
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
        // Carry the boot config so the WebUI facade can compose the operator
        // LLM-config settings service over `providers.json` / `config.toml`.
        #[cfg(feature = "root-llm-provider")]
        {
            runtime_input = runtime_input.with_boot_config(boot_config.clone());
        }
        let default_agent_raw =
            resolve_webui_default_agent(identity_section, &runtime_input.identity);
        let default_agent_id = AgentId::new(&default_agent_raw).map_err(|err| {
            anyhow!("[identity].default_agent `{default_agent_raw}` is invalid: {err}")
        })?;
        let default_project_id = identity_section
            .and_then(|identity| identity.default_project.as_deref())
            .map(ProjectId::new)
            .transpose()
            .map_err(|err| anyhow!("[identity].default_project is invalid: {err}"))?;
        if let Some(project_id) = default_project_id.clone() {
            runtime_input = runtime_input.with_default_project_id(project_id);
        }
        let slack_host_beta_config = crate::commands::serve_slack::resolve_slack_config_for_serve(
            config_file.as_ref().and_then(|file| file.slack.as_ref()),
            &tenant_id,
            &default_agent_id,
            default_project_id.as_ref(),
            &user_id,
            &boot_config.home().config_file_path(),
        )?;
        #[cfg(not(feature = "slack-v2-host-beta"))]
        let _ = slack_host_beta_config;

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
        let callback_origin =
            webui_notion_dcr_callback_origin(listen_addr, canonical_host.as_deref())?;
        if let Some(callback_origin) = callback_origin {
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

        // WebChat v2 SSO login startup config (providers + base URL +
        // cleartext guard). Resolved here so misconfiguration fails fast
        // before the runtime is built; the DB-backed user directory and
        // the login wiring are assembled inside the async runtime below,
        // because opening the libSQL user store is async.
        let sso_startup = crate::commands::serve_sso::sso_startup_config_from_env(listen_addr)?;
        // When SSO is enabled this same token keys the stateless session
        // HMAC, so a weak value becomes an OFFLINE forgery target: an
        // attacker who completes one legitimate login holds a
        // `{payload}.{hmac}` pair and can brute-force a low-entropy key
        // locally, then mint a session for any user/tenant. Pre-SSO the
        // token only ever gated an online, rate-limited bearer guess.
        // Require real entropy; fail closed rather than warn.
        if sso_startup.is_some() && token_byte_len < 32 {
            return Err(anyhow!(
                "{env_token_var} is also the WebChat SSO session-signing key and must be at \
                 least 32 bytes of high-entropy random material when an SSO provider is \
                 configured (it signs stateless, user-visible session tokens). The current \
                 value is {token_byte_len} bytes — generate one with e.g. `openssl rand -hex 32`."
            ));
        }
        // Sidecar DB used by the local-runtime trigger-fire access checker. It
        // backs the local trigger-fire
        // access store used to seed default-user and SSO-user trigger access;
        // canonical identity itself lives on the runtime's scoped filesystem,
        // not in this file.
        let profile = crate::runtime::effective_profile(boot_config, config_file.as_ref())?;
        let user_store_path = crate::runtime::local_runtime_storage_root(boot_config, profile)
            .join("reborn-local-dev.db");
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
        seed_default_config_file_if_missing(&context.boot_config().home().config_file_path())
            .map_err(anyhow::Error::from)?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("failed to build tokio runtime for `serve`")?;

        rt.block_on(async move {
            runtime_input = with_local_trigger_fire_access_checker(
                runtime_input,
                &user_store_path,
                &tenant_id,
                &user_id,
                &default_agent_id,
                default_project_id.as_ref(),
            )
            .await?;

            let startup_serve = if profile == RebornProfile::HostedSingleTenant {
                Some(start_hosted_single_tenant_startup_listener(listen_addr).await?)
            } else {
                None
            };

            let runtime = build_reborn_runtime(runtime_input)
                .await
                .context("failed to assemble Reborn runtime for `serve`")?;
            #[cfg(feature = "slack-v2-host-beta")]
            let slack_mounts = if let Some(slack_config) = slack_host_beta_config {
                match build_slack_host_beta_runtime_mounts(&runtime, slack_config)
                    .await
                    .context("failed to compose Slack host-beta routes")
                {
                    Ok(mounts) => Some(mounts),
                    Err(error) => {
                        let shutdown_result = runtime.shutdown().await;
                        if let Err(shutdown_error) = shutdown_result {
                            return Err(error.context(format!(
                                "runtime shutdown after Slack route composition failure also failed: {shutdown_error}"
                            )));
                        }
                        return Err(error);
                    }
                }
            } else {
                None
            };
            #[cfg(feature = "slack-v2-host-beta")]
            let operator_route_visibility = if sso_startup.is_none() {
                SlackOperatorRouteVisibility::Visible
            } else {
                SlackOperatorRouteVisibility::Hidden
            };
            #[cfg(feature = "slack-v2-host-beta")]
            let bundle: RebornWebuiBundle = build_webui_services_with_slack_host_beta_mounts(
                &runtime,
                None,
                slack_mounts.as_ref(),
                operator_route_visibility,
            )?;
            #[cfg(not(feature = "slack-v2-host-beta"))]
            let bundle: RebornWebuiBundle = build_webui_services(&runtime, None)?;
            #[cfg(feature = "openai-compat-beta")]
            let openai_compat_mount = build_openai_compat_route_mount(
                &runtime,
                tenant_id.clone(),
                default_agent_id.clone(),
                default_project_id.clone(),
            )
            .await
            .context("failed to compose OpenAI-compatible Reborn routes")?;

            // Open the canonical Reborn identity resolver on the runtime's
            // existing substrate handle (the same `reborn-local-dev.db` the
            // runtime owns) rather than opening a second handle to the file.
            // Only SSO-enabled WebUI needs it: an env-bearer-only deployment
            // resolves its single configured user without any identity store,
            // so skip opening (and its legacy migration) when SSO is disabled
            // — otherwise a disabled-SSO deployment could fail startup on an
            // unused identity backend. `None` also covers the case where the
            // runtime carries no local-runtime substrate; the auth surface
            // fails closed when SSO is configured but no resolver is available.
            let identity_resolver = if sso_startup.is_some() {
                match runtime.open_reborn_identity_resolver(&tenant_id).await {
                    Some(result) => {
                        Some(result.context("failed to initialize the Reborn identity resolver")?)
                    }
                    None => None,
                }
            } else {
                None
            };

            // Assemble the WebChat v2 auth surface (authenticator + optional
            // public login mount). The auth/identity module owns the
            // signed-session wiring; `serve` supplies host config, the
            // runtime-owned identity resolver, and the local trigger-access
            // bootstrap that seeds an admitted SSO user's trigger access on
            // login.
            let crate::commands::webui_auth::WebuiAuthSurface {
                authenticator,
                public_mount,
            } = crate::commands::webui_auth::build_webui_auth_surface(
                sso_startup,
                identity_resolver,
                tenant_id.clone(),
                session_signing_secret,
                env_authenticator,
                Some(
                    crate::commands::webui_auth::LocalTriggerAccessBootstrapConfig {
                        access_store_path: user_store_path.clone(),
                        tenant_id: tenant_id.clone(),
                        agent_id: default_agent_id.clone(),
                        project_id: default_project_id.clone(),
                    },
                ),
            )
            .await?;

            print_serve_banner(
                listen_addr,
                env_token_var,
                env_user_id_var,
                &allowed_origins_raw,
                &bundle.readiness,
            );

            let mut serve_config = WebuiServeConfig::new(tenant_id, authenticator, allowed_origins)
                .with_default_agent_id(default_agent_id.clone());
            if let Some(project_id) = default_project_id.clone() {
                serve_config = serve_config.with_default_project_id(project_id);
            }
            #[cfg(feature = "openai-compat-beta")]
            {
                serve_config = serve_config.with_protected_route_mount(openai_compat_mount);
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
            #[cfg(feature = "slack-v2-host-beta")]
            if let Some(slack_mounts) = slack_mounts {
                serve_config = serve_config
                    .with_public_route_mount(slack_mounts.events)
                    .with_slack_personal_binding_pairing(slack_mounts.personal_binding_pairing)
                    .with_slack_channel_routes(slack_mounts.channel_routes);
            }
            // Public NEAR AI login callback route (token redirect target). Built
            // from the runtime's LLM seam; absent when no LLM was wired.
            #[cfg(feature = "root-llm-provider")]
            if let Some(nearai_mount) = runtime.nearai_login_callback_mount() {
                serve_config = serve_config.with_public_route_mount(nearai_mount);
            }
            if let Some(mount) = public_mount {
                serve_config = serve_config.with_public_route_mount(mount);
            }
            let webui_app = webui_v2_app_with_lifecycle(bundle, serve_config)
                .context("failed to compose v2 Router")?;
            let (router, public_route_drains) = webui_app.into_parts();

            let serve_result = if let Some(startup_serve) = startup_serve {
                startup_serve
                    .ready_handle
                    .publish_ready_router(router)
                    .context("failed to publish ready WebChat v2 router")?;
                startup_serve
                    .serve_task
                    .await
                    .context("hosted single-tenant startup WebChat v2 serve task failed to join")?
            } else {
                serve_webui_v2(RebornWebuiServeOptions {
                    addr: listen_addr,
                    router,
                    shutdown: webui_ctrl_c_shutdown(),
                    bound_addr_tx: None,
                })
                .await
            };

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

struct StartupServe {
    ready_handle: DeferredWebuiRouterHandle,
    serve_task: tokio::task::JoinHandle<Result<(), RebornWebuiServeError>>,
}

async fn start_hosted_single_tenant_startup_listener(
    listen_addr: SocketAddr,
) -> anyhow::Result<StartupServe> {
    let (router, ready_handle) = deferred_webui_v2_startup_router();
    let (bound_tx, bound_rx) = tokio::sync::oneshot::channel();
    let serve_task = tokio::spawn(async move {
        serve_webui_v2(RebornWebuiServeOptions {
            addr: listen_addr,
            router,
            shutdown: webui_ctrl_c_shutdown(),
            bound_addr_tx: Some(bound_tx),
        })
        .await
    });

    match bound_rx.await {
        Ok(bound) => {
            tracing::info!(
                target = "ironclaw::reborn::cli::serve",
                %bound,
                "hosted single-tenant WebChat v2 startup listener is serving healthchecks before runtime assembly"
            );
        }
        Err(_) => {
            let serve_result = serve_task
                .await
                .context("hosted single-tenant startup WebChat v2 serve task failed to join")?;
            serve_result.context("hosted single-tenant startup WebChat v2 serve loop failed")?;
            anyhow::bail!("hosted single-tenant startup listener exited before binding");
        }
    }

    Ok(StartupServe {
        ready_handle,
        serve_task,
    })
}

fn webui_ctrl_c_shutdown() -> tokio::sync::oneshot::Receiver<()> {
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
    shutdown_rx
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

fn webui_notion_dcr_callback_origin(
    listen_addr: SocketAddr,
    canonical_host: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let public_base_url = crate::commands::serve_sso::webui_public_base_url_from_env()
        .context("invalid hosted WebUI OAuth base URL from IRONCLAW_REBORN_WEBUI_BASE_URL")?;
    crate::commands::serve_sso::validate_webui_public_base_url(
        public_base_url.as_deref(),
        listen_addr,
    )
    .context("invalid hosted WebUI OAuth base URL from IRONCLAW_REBORN_WEBUI_BASE_URL")?;
    Ok(webui_oauth_callback_origin(
        listen_addr,
        public_base_url.as_deref(),
        canonical_host,
    ))
}

fn webui_oauth_callback_origin(
    listen_addr: SocketAddr,
    public_base_url: Option<&str>,
    canonical_host: Option<&str>,
) -> Option<String> {
    if let Some(base_url) = public_base_url {
        let base_url = base_url.trim().trim_end_matches('/');
        if base_url.is_empty() {
            return None;
        }
        if crate::commands::serve_sso::is_cleartext_http_scheme(base_url)
            && !listen_addr.ip().is_loopback()
        {
            return None;
        }
        return Some(base_url.to_string());
    }
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

async fn with_local_trigger_fire_access_checker(
    runtime_input: RebornRuntimeInput,
    user_store_path: &std::path::Path,
    tenant_id: &TenantId,
    user_id: &UserId,
    default_agent_id: &AgentId,
    default_project_id: Option<&ProjectId>,
) -> anyhow::Result<RebornRuntimeInput> {
    if !runtime_input.trigger_poller.enabled {
        return Ok(runtime_input);
    }

    let access_store = open_local_trigger_access_store(user_store_path)
        .await
        .context("failed to initialize local trigger-fire access store")?;
    let user_ids = [user_id.clone()];
    access_store
        .reconcile_local_access(LocalTriggerAccessReconciliation {
            tenant_id,
            user_ids: &user_ids,
            agent_id: Some(default_agent_id),
            project_id: default_project_id,
            role: LocalTriggerAccessRole::Owner,
            source: LocalTriggerAccessSource::LocalDevEnvBootstrap,
        })
        .await
        .context("failed to reconcile local trigger-fire access")?;
    Ok(runtime_input.with_trigger_fire_access_checker(access_store))
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

    const WEBUI_BASE_URL_ENV: &str = "IRONCLAW_REBORN_WEBUI_BASE_URL";

    fn clear_webui_env() {
        // SAFETY: tests are serialized by `WEBUI_BASE_URL_ENV_LOCK`; no other
        // thread reads or writes this env var while the guard is held.
        unsafe { std::env::remove_var(WEBUI_BASE_URL_ENV) };
    }

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

    #[tokio::test]
    async fn trigger_poller_disabled_does_not_wire_local_access_checker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tenant_id = TenantId::new("serve-trigger-disabled-tenant").expect("tenant id");
        let user_id = UserId::new("serve-trigger-disabled-user").expect("user id");
        let agent_id = AgentId::new("serve-trigger-disabled-agent").expect("agent id");
        let runtime_input = RebornRuntimeInput::from_services(RebornBuildInput::local_dev(
            "serve-trigger-owner",
            dir.path().join("runtime"),
        ));
        let missing_store_path = dir.path().join("missing").join("reborn-local-dev.db");

        let runtime_input = with_local_trigger_fire_access_checker(
            runtime_input,
            &missing_store_path,
            &tenant_id,
            &user_id,
            &agent_id,
            None,
        )
        .await
        .expect("disabled trigger poller skips local access store");

        assert!(
            runtime_input.trigger_fire_access_checker.is_none(),
            "disabled trigger poller must not wire a local access checker"
        );
        assert!(
            !missing_store_path.exists(),
            "disabled trigger poller must not create the local access store"
        );
    }

    #[tokio::test]
    async fn trigger_poller_bootstrap_seeds_local_access_checker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tenant_id = TenantId::new("serve-trigger-tenant").expect("tenant id");
        let user_id = UserId::new("serve-trigger-user").expect("user id");
        let stale_user_id = UserId::new("serve-trigger-stale").expect("stale user id");
        let agent_id = AgentId::new("serve-trigger-agent").expect("agent id");
        let project_id = ProjectId::new("serve-trigger-project").expect("project id");
        let user_store_path = dir.path().join("reborn-local-dev.db");
        let access_store =
            ironclaw_reborn_composition::open_local_trigger_access_store(&user_store_path)
                .await
                .expect("open local trigger access store");
        access_store
            .seed_local_access(ironclaw_reborn_composition::LocalTriggerAccessSeed {
                tenant_id: &tenant_id,
                user_id: &stale_user_id,
                agent_id: Some(&agent_id),
                project_id: Some(&project_id),
                role: LocalTriggerAccessRole::Owner,
                source: LocalTriggerAccessSource::LocalDevEnvBootstrap,
            })
            .await
            .expect("seed stale local trigger access");
        let runtime_input =
            RebornRuntimeInput::from_services(RebornBuildInput::local_dev(
                "serve-trigger-owner",
                dir.path().join("runtime"),
            ))
            .with_trigger_poller_settings(
                ironclaw_reborn_composition::TriggerPollerSettings::enabled(),
            );

        let runtime_input = with_local_trigger_fire_access_checker(
            runtime_input,
            &user_store_path,
            &tenant_id,
            &user_id,
            &agent_id,
            Some(&project_id),
        )
        .await
        .expect("bootstrap trigger fire access checker");

        let checker = runtime_input
            .trigger_fire_access_checker
            .expect("checker is wired");
        let decision = checker
            .check_trigger_fire_access(ironclaw_reborn_composition::TriggerFireAccessCheck {
                tenant_id: tenant_id.clone(),
                creator_user_id: user_id,
                agent_id: Some(agent_id.clone()),
                project_id: Some(project_id.clone()),
                trigger_id: ironclaw_reborn_composition::TriggerId::new(),
                fire_slot: chrono::Utc::now(),
            })
            .await
            .expect("check trigger fire access");

        assert_eq!(
            decision,
            ironclaw_reborn_composition::TriggerFireAccessDecision::Allowed
        );

        let stale_decision = checker
            .check_trigger_fire_access(ironclaw_reborn_composition::TriggerFireAccessCheck {
                tenant_id,
                creator_user_id: stale_user_id,
                agent_id: Some(agent_id),
                project_id: Some(project_id),
                trigger_id: ironclaw_reborn_composition::TriggerId::new(),
                fire_slot: chrono::Utc::now(),
            })
            .await
            .expect("check stale trigger fire access");

        assert_eq!(
            stale_decision,
            ironclaw_reborn_composition::TriggerFireAccessDecision::Denied {
                reason: "trigger creator does not have active local access for this scope"
                    .to_string(),
            }
        );
    }

    #[tokio::test]
    async fn trigger_poller_bootstrap_seeds_no_project_local_access_checker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tenant_id = TenantId::new("serve-trigger-no-project-tenant").expect("tenant id");
        let user_id = UserId::new("serve-trigger-no-project-user").expect("user id");
        let agent_id = AgentId::new("serve-trigger-no-project-agent").expect("agent id");
        let project_id = ProjectId::new("serve-trigger-no-project-project").expect("project id");
        let user_store_path = dir.path().join("reborn-local-dev.db");
        let runtime_input =
            RebornRuntimeInput::from_services(RebornBuildInput::local_dev(
                "serve-trigger-owner",
                dir.path().join("runtime"),
            ))
            .with_trigger_poller_settings(
                ironclaw_reborn_composition::TriggerPollerSettings::enabled(),
            );

        let runtime_input = with_local_trigger_fire_access_checker(
            runtime_input,
            &user_store_path,
            &tenant_id,
            &user_id,
            &agent_id,
            None,
        )
        .await
        .expect("bootstrap trigger fire access checker");

        let checker = runtime_input
            .trigger_fire_access_checker
            .expect("checker is wired");
        let decision = checker
            .check_trigger_fire_access(ironclaw_reborn_composition::TriggerFireAccessCheck {
                tenant_id: tenant_id.clone(),
                creator_user_id: user_id.clone(),
                agent_id: Some(agent_id.clone()),
                project_id: None,
                trigger_id: ironclaw_reborn_composition::TriggerId::new(),
                fire_slot: chrono::Utc::now(),
            })
            .await
            .expect("check trigger fire access");

        assert_eq!(
            decision,
            ironclaw_reborn_composition::TriggerFireAccessDecision::Allowed
        );

        let project_scoped_decision = checker
            .check_trigger_fire_access(ironclaw_reborn_composition::TriggerFireAccessCheck {
                tenant_id,
                creator_user_id: user_id,
                agent_id: Some(agent_id),
                project_id: Some(project_id),
                trigger_id: ironclaw_reborn_composition::TriggerId::new(),
                fire_slot: chrono::Utc::now(),
            })
            .await
            .expect("check project-scoped trigger fire access");

        assert_eq!(
            project_scoped_decision,
            ironclaw_reborn_composition::TriggerFireAccessDecision::Denied {
                reason: "trigger creator does not have active local access for this scope"
                    .to_string(),
            }
        );
    }

    #[test]
    fn webui_oauth_callback_origin_uses_loopback_http() {
        assert_eq!(
            webui_oauth_callback_origin(SocketAddr::from(([127, 0, 0, 1], 3000)), None, None)
                .as_deref(),
            Some("http://127.0.0.1:3000")
        );
    }

    #[test]
    fn webui_oauth_callback_origin_maps_unspecified_bind_to_localhost() {
        assert_eq!(
            webui_oauth_callback_origin(SocketAddr::from(([0, 0, 0, 0], 3000)), None, None)
                .as_deref(),
            Some("http://localhost:3000")
        );
    }

    #[test]
    fn webui_oauth_callback_origin_brackets_ipv6_loopback() {
        let listen_addr = SocketAddr::new(IpAddr::from_str("::1").unwrap(), 3000);

        assert_eq!(
            webui_oauth_callback_origin(listen_addr, None, None).as_deref(),
            Some("http://[::1]:3000")
        );
    }

    #[test]
    fn webui_oauth_callback_origin_skips_unstable_or_non_loopback_origin() {
        assert_eq!(
            webui_oauth_callback_origin(SocketAddr::from(([127, 0, 0, 1], 0)), None, None),
            None
        );
        assert_eq!(
            webui_oauth_callback_origin(SocketAddr::from(([192, 168, 1, 42], 3000)), None, None),
            None
        );
    }

    #[test]
    fn webui_oauth_callback_origin_uses_https_canonical_host() {
        assert_eq!(
            webui_oauth_callback_origin(
                SocketAddr::from(([0, 0, 0, 0], 3000)),
                None,
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
                None,
                Some("127.0.0.1:3000"),
            )
            .as_deref(),
            Some("http://127.0.0.1:3000")
        );
    }

    #[test]
    fn webui_oauth_callback_origin_brackets_ipv6_canonical_host() {
        assert_eq!(
            webui_oauth_callback_origin(SocketAddr::from(([0, 0, 0, 0], 3000)), None, Some("::1"))
                .as_deref(),
            Some("http://[::1]")
        );
    }

    #[test]
    fn webui_oauth_callback_origin_prefers_public_base_url_for_hosted_oauth() {
        assert_eq!(
            webui_oauth_callback_origin(
                SocketAddr::from(([0, 0, 0, 0], 8080)),
                Some("https://app.example.com/"),
                Some("internal.example.com"),
            )
            .as_deref(),
            Some("https://app.example.com")
        );
    }

    #[test]
    fn webui_oauth_callback_origin_rejects_cleartext_public_origin_on_non_loopback() {
        assert_eq!(
            webui_oauth_callback_origin(
                SocketAddr::from(([192, 168, 1, 42], 8080)),
                Some("http://app.example.com/"),
                None,
            ),
            None
        );
    }

    #[test]
    fn webui_oauth_callback_origin_keeps_loopback_http_public_origin() {
        assert_eq!(
            webui_oauth_callback_origin(
                SocketAddr::from(([127, 0, 0, 1], 8080)),
                Some("http://127.0.0.1:8080/"),
                None,
            )
            .as_deref(),
            Some("http://127.0.0.1:8080")
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
                None,
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

    #[tokio::test]
    async fn webui_serve_wires_notion_dcr_with_public_base_url_env_origin() {
        let callback_origin = {
            let _guard = crate::commands::serve_sso::WEBUI_BASE_URL_ENV_LOCK
                .lock()
                .expect("env lock");
            clear_webui_env();
            // SAFETY: serialized by WEBUI_BASE_URL_ENV_LOCK; cleaned up before the guard drops.
            unsafe {
                std::env::set_var(WEBUI_BASE_URL_ENV, " https://configured.example/ ");
            }

            let callback_origin =
                webui_notion_dcr_callback_origin(SocketAddr::from(([0, 0, 0, 0], 8080)), None)
                    .expect("resolve callback origin from env")
                    .expect("public base url env should enable DCR wiring");
            assert_eq!(callback_origin, "https://configured.example");
            clear_webui_env();
            callback_origin
        };

        let dir = tempfile::tempdir().expect("tempdir");
        let services_input = with_notion_dcr_oauth_backend(
            RebornBuildInput::local_dev("notion-dcr-owner", dir.path().join("local-dev")),
            &callback_origin,
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

    #[test]
    fn webui_notion_dcr_callback_origin_rejects_slash_only_public_base_url_env() {
        let _guard = crate::commands::serve_sso::WEBUI_BASE_URL_ENV_LOCK
            .lock()
            .expect("env lock");
        clear_webui_env();
        // SAFETY: serialized by WEBUI_BASE_URL_ENV_LOCK; cleaned up before the guard drops.
        unsafe {
            std::env::set_var(WEBUI_BASE_URL_ENV, "/");
        }

        let error = webui_notion_dcr_callback_origin(SocketAddr::from(([0, 0, 0, 0], 8080)), None)
            .expect_err("slash-only base URL must fail closed");
        assert!(
            error.to_string().contains(WEBUI_BASE_URL_ENV),
            "error should name the invalid env var, got: {error}"
        );

        clear_webui_env();
    }

    #[test]
    fn webui_notion_dcr_callback_origin_rejects_public_cleartext_base_url_env() {
        let _guard = crate::commands::serve_sso::WEBUI_BASE_URL_ENV_LOCK
            .lock()
            .expect("env lock");
        clear_webui_env();
        // SAFETY: serialized by WEBUI_BASE_URL_ENV_LOCK; cleaned up before the guard drops.
        unsafe {
            std::env::set_var(WEBUI_BASE_URL_ENV, "http://configured.example");
        }

        let error = webui_notion_dcr_callback_origin(SocketAddr::from(([0, 0, 0, 0], 8080)), None)
            .expect_err("public cleartext base URL must fail closed");
        let message = error.to_string();
        assert!(
            message.contains(WEBUI_BASE_URL_ENV),
            "error should name the invalid env var, got: {message}"
        );
        assert!(
            message.contains("hosted WebUI OAuth base URL"),
            "error should describe the hosted WebUI OAuth URL, got: {message}"
        );

        clear_webui_env();
    }
}
