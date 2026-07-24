use std::env;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Context, anyhow};
use clap::Args;
use ironclaw_reborn_composition::build_openai_compat_route_mount;
use ironclaw_reborn_composition::build_webui_services;
use ironclaw_reborn_composition::host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, SecretHandle, TenantId, UserId,
};
use ironclaw_reborn_composition::{
    RebornHostBindings, RebornReadiness, RebornRuntimeIdentity, RebornRuntimeInput,
    RebornWebuiBundle, TriggerFireAccessPolicy, build_reborn_runtime,
};
use ironclaw_reborn_config::{IdentitySection, seed_default_config_file_if_missing};
use ironclaw_webui::{
    DeferredWebuiRouterHandle, EnvBearerAuthenticator, RebornWebuiServeError,
    RebornWebuiServeOptions, WebuiAuthenticator, WebuiServeConfig,
    deferred_webui_v2_startup_router, serve_webui_v2, webui_v2_app_with_lifecycle,
};
use secrecy::SecretString;

use crate::context::RebornCliContext;
use crate::runtime::RuntimeInputOptions;

// pub(crate): reused by onboard's finale login-link print (same default host:port).
pub(crate) const DEFAULT_SERVE_HOST: &str = "127.0.0.1";
pub(crate) const DEFAULT_SERVE_PORT: u16 = 3000;
// pub(crate): reused by onboard/status for `env_token_is_active` (webui_token.rs).
pub(crate) const DEFAULT_ENV_TOKEN_VAR: &str = "IRONCLAW_REBORN_WEBUI_TOKEN";
const DEFAULT_ENV_USER_ID_VAR: &str = "IRONCLAW_REBORN_WEBUI_USER_ID";
/// Lifetime of the one-time API bearer minted when an admin creates a user. A
/// year: this is a long-lived programmatic credential, not a browser session.
const ADMIN_API_TOKEN_LIFETIME_DAYS: i64 = 365;

/// Read an env var, distinguishing "unset" from "set but not valid UTF-8".
///
/// `std::env::var(name).ok()` collapses both `VarError::NotPresent` and
/// `VarError::NotUnicode` to `None` — which for the WebChat v2 bearer
/// token env var is dangerous: an operator whose token value got mangled
/// into invalid UTF-8 (a shell/CI export bug, a truncated byte sequence)
/// would silently fall through to the `<reborn_home>/webui-token` file
/// credential instead of failing loudly. Only `NotPresent` means "treat
/// as unset"; `NotUnicode` is a real configuration error and must
/// propagate with context naming the variable.
///
/// pub(crate): shared with `webui_token::env_token_is_active` so both
/// checks (token source vs. login-link gating) never drift.
pub(crate) fn present_unicode_env_var(name: &str) -> anyhow::Result<Option<String>> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(raw)) => Err(anyhow!(
            "{name} is set but is not valid UTF-8 ({} raw bytes); refusing to silently treat it \
             as unset, which would otherwise fall through to the WebChat v2 token file credential.",
            raw.as_encoded_bytes().len()
        )),
    }
}

/// Mints the admin-created-user API bearer over a signed session store. The
/// store is deterministic in its signing key (operator secret + tenant), so a
/// token minted here validates under the SSO login surface's own store.
struct SignedSessionTokenMinter {
    session_store: Arc<ironclaw_webui::SignedTokenSessionStore>,
}

#[async_trait::async_trait]
impl ironclaw_reborn_composition::AdminApiTokenMinter for SignedSessionTokenMinter {
    async fn mint(&self, tenant: &TenantId, user_id: &UserId) -> Result<SecretString, String> {
        // `false`: this session is for the admin-created `user_id`, not the
        // operator. Stamping `true` would let any admin-created user (even
        // Member-role) bypass `require_operator_webui_config` — a distinct
        // per-user RBAC axis from the single-box operator capability.
        self.session_store
            .create_session(
                tenant.clone(),
                user_id.clone(),
                chrono::Duration::days(ADMIN_API_TOKEN_LIFETIME_DAYS),
                false,
            )
            .await
            .map_err(|error| error.to_string())
    }
}

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
        let built = crate::runtime::build_runtime_input_with_options(
            context.boot_config(),
            crate::runtime::RuntimeInputCaller::Serve,
            RuntimeInputOptions {
                confirm_host_access: self.confirm_host_access,
            },
        )?;
        let runtime_input = built.inner;
        let boot_config = context.boot_config();
        let config_file =
            ironclaw_reborn_config::RebornConfigFile::load(&boot_config.home().config_file_path())
                .map_err(anyhow::Error::from)?;
        if let Some(file) = config_file.as_ref() {
            reject_legacy_slack_config(file, &boot_config.home().config_file_path())?;
        }

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

        // Precedence: `env_token_var` (if set and non-empty), else
        // `<reborn_home>/webui-token` (the `onboard`-provisioned fallback a
        // service-installed serve, whose unit environment carries only
        // HOME/PROFILE, still needs — see `serve_invocation.rs`). Also
        // enforces the >=32-byte entropy floor (this token doubles as the
        // session-signing HMAC key — see the comment near
        // `session_signing_secret` below) so a weak or missing token fails
        // closed here rather than starting the server and having it reject
        // the value opaquely.
        let resolved_token = crate::webui_token::resolve_webui_token(
            env_token_var,
            present_unicode_env_var(env_token_var)?.as_deref(),
            boot_config.home().path(),
        )?;
        let webui_token_source = resolved_token.source;
        let token_value = resolved_token.value;
        let user_id_raw = resolve_webui_user_id_raw(env_user_id_var, config_file.as_ref())?;
        let user_id = UserId::new(&user_id_raw)
            .map_err(|err| anyhow!("{env_user_id_var} value `{user_id_raw}` is invalid: {err}"))?;

        // Keep a copy of the operator secret to key the SSO session-token
        // HMAC before the value is moved into the env-bearer authenticator.
        // Held as `SecretString` so it is redacted in `Debug`/logs and
        // zeroed on drop — it doubles as the session-signing key.
        // `resolve_webui_token` above already enforced the >=32-byte
        // entropy floor this key needs, regardless of which of its two
        // sources (env var or `<reborn_home>/webui-token`) produced it.
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
        // Admin user-management: mint the one-time API bearer on user create via
        // a signed session store built from the same operator secret + tenant as
        // the SSO login surface. The store is stateless and deterministic in its
        // signing key, so this sibling instance (built before the login surface)
        // mints tokens that validate under the login surface's own store.
        let admin_session_store =
            ironclaw_webui::signed_session_store(&session_signing_secret, &tenant_id);
        // Cloned for the CLI-token-login mount, built later once `sso_enabled`
        // is known — same operator secret + tenant, so it validates identically.
        let cli_login_session_store = admin_session_store.clone();
        runtime_input =
            runtime_input.with_admin_api_token_minter(Arc::new(SignedSessionTokenMinter {
                session_store: admin_session_store,
            }));
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
            webui_product_auth_callback_origin(listen_addr, canonical_host.as_deref())?;
        if let Some(callback_origin) = callback_origin {
            let services = runtime_input.services.take().ok_or_else(|| {
                anyhow!("WebChat v2 serve requires Reborn runtime services before OAuth wiring")
            })?;
            runtime_input.services = Some(
                with_product_auth_callback_origin(services, &callback_origin)
                    .context("failed to configure product-auth OAuth for WebChat v2")?,
            );
        } else {
            tracing::warn!(
                target = "ironclaw::reborn::cli::serve",
                %listen_addr,
                "product-auth OAuth is not configured because the WebChat v2 listener origin is not a stable loopback HTTP origin"
            );
        }

        // WebChat v2 SSO login startup config (providers + base URL +
        // cleartext guard). Resolved here so misconfiguration fails fast
        // before the runtime is built; the DB-backed user directory and
        // the login wiring are assembled inside the async runtime below,
        // because opening the libSQL user store is async.
        let sso_startup = crate::commands::serve_sso::sso_startup_config_from_env(listen_addr)?;
        // This token keys the stateless session HMAC, so a weak value would be
        // an OFFLINE forgery target: an attacker who obtains one legitimate
        // `{payload}.{hmac}` session pair could brute-force a low-entropy key
        // locally, then mint a session for any user/tenant. Two paths mint
        // such user-visible session tokens, so the entropy floor is
        // unconditional:
        //   - SSO login (`sso_startup`) signs a session on every login, and
        //   - admin user-management (wired above via
        //     `with_admin_api_token_minter`) mints a one-time session bearer
        //     on `POST /admin/users`.
        // The admin minter is always installed, so a signed session token can
        // always be produced regardless of whether SSO is configured.
        // `crate::webui_token::resolve_webui_token` already enforced the
        // >=32-byte floor when `token_value` was resolved above, so no
        // separate check is needed here.
        let profile = crate::runtime::effective_profile(boot_config, config_file.as_ref())?;
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
            // The agent loop executes a deep async dispatch chain (turn runner ->
            // planned driver -> canonical executor -> capability stage -> host
            // dispatch -> first-party tool); a single poll of one capability
            // dispatch consumes ~1.9 MB of stack in debug builds, which overflows
            // the default 2 MB worker thread. Match the 8 MB stack the codebase
            // already uses for deep work (see ironclaw_reborn_cli traces tests and
            // src/cli stack_size sites).
            .thread_stack_size(8 * 1024 * 1024)
            .build()
            .context("failed to build tokio runtime for `serve`")?;

        rt.block_on(async move {
            let trigger_poller_enabled = runtime_input.trigger_poller.enabled;
            let sso_enabled = sso_startup.is_some();
            let startup_serve = if profile.starts_hosted_single_tenant_listener() {
                Some(start_hosted_single_tenant_startup_listener(listen_addr).await?)
            } else {
                None
            };

            // Fire-time trigger access is a config value, not a persisted store
            // (arch-simplification §4.4). When the poller is on, the operator
            // owner may always fire; when SSO is also on, any active tenant
            // member (the users SSO login persists in the identity store) may
            // too — the union the former single trigger-access store expressed.
            runtime_input =
                runtime_input.with_trigger_fire_access_policy(trigger_fire_access_policy(
                    trigger_poller_enabled,
                    &user_id,
                    &default_agent_id,
                    default_project_id.as_ref(),
                ));

            let runtime = build_reborn_runtime(runtime_input)
                .await
                .context("failed to assemble Reborn runtime for `serve`")?;

            // Tenant-shared tool credentials from the environment (#5459):
            // `IRONCLAW_REBORN_DEV_SECRET__<handle>=<value>` pairs, parsed by
            // `dev_secret_seeds_from_env` (see its doc for the contract), are
            // written into the tenant-shared admin-managed scope so a keyed
            // tool (network + `use_secret`) resolves its `InjectSecretOnce`
            // obligation for EVERY user of the tenant — including SSO users
            // who never provisioned it — from one operator-set key. Inert
            // unless the operator sets one; ops/dev path, not per-user setup.
            for (shared_scope, handle, value) in dev_secret_seeds_from_env(
                std::env::vars(),
                &tenant_id,
                &user_id,
                &default_agent_id,
                default_project_id.as_ref(),
            )? {
                let handle_name = handle.as_str().to_string();
                runtime
                    .seed_local_dev_secret(shared_scope, handle, value)
                    .await
                    .map_err(|err| anyhow!("failed to seed dev secret `{handle_name}`: {err}"))?;
                tracing::warn!(
                    target: "ironclaw::reborn::cli",
                    secret_handle = %handle_name,
                    "seeded IRONCLAW_REBORN_DEV_SECRET__ tool credential at the tenant-shared scope"
                );
            }

            let bundle: RebornWebuiBundle = build_webui_services(&runtime, None)?;
            let openai_compat_mount = build_openai_compat_route_mount(
                &runtime,
                tenant_id.clone(),
                default_agent_id.clone(),
                default_project_id.clone(),
            )
            .await
            .context("failed to compose OpenAI-compatible Reborn routes")?;

            // Only SSO-enabled WebUI needs the canonical Reborn identity
            // resolver: an env-bearer-only deployment resolves its single
            // configured user without any identity store, so skip opening (and
            // its legacy migration) when SSO is disabled.
            let identity_resolver = if sso_startup.is_some() {
                Some(
                    runtime
                        .open_reborn_identity_resolver(&tenant_id)
                        .await
                        .context("failed to initialize the Reborn identity resolver")?,
                )
            } else {
                None
            };

            // Cloned before the moves below: the CLI-token-login mount (built
            // after `build_webui_auth_surface`) needs its own tenant id and
            // bearer authenticator, but the originals are moved into the
            // auth-surface call immediately below.
            let cli_login_tenant_id = tenant_id.clone();
            let cli_login_authenticator = Arc::clone(&env_authenticator);

            // Assemble the WebChat v2 auth surface (authenticator + optional
            // public login mount). The auth/identity module owns the
            // signed-session wiring; `serve` supplies host config and the
            // runtime-owned identity resolver. An admitted SSO user's trigger
            // access is no longer separately seeded here — the identity
            // `StoredUser` the login persists IS the membership the fire-time
            // checker reads (arch-simplification §4.4).
            let crate::commands::webui_auth::WebuiAuthSurface {
                authenticator,
                public_mount,
            } = crate::commands::webui_auth::build_webui_auth_surface(
                sso_startup,
                identity_resolver,
                tenant_id.clone(),
                session_signing_secret,
                env_authenticator,
            )
            .await?;

            // CLI-token-login mount (`GET /login?token=`, printed by `onboard`
            // at setup end) — only when SSO is off AND the token came from
            // the FILE, not env:
            // - `build_cli_token_login` mounts its own `POST
            //   /auth/session/exchange` unconditionally; mounting it while
            //   SSO is on would double-register that path and panic at
            //   router-merge time (no shared-ticket-store knob exists).
            // - env-sourced tokens (e.g. Railway-style `IRONCLAW_REBORN_WEBUI_TOKEN`)
            //   must not appear in this route's query string, which flows
            //   through edge/proxy access logs.
            let cli_login_mount = if sso_enabled
                || webui_token_source != crate::webui_token::WebuiTokenSource::File
            {
                None
            } else {
                Some(ironclaw_webui::build_cli_token_login(
                    ironclaw_webui::CliTokenLoginConfig::new(
                        cli_login_tenant_id,
                        cli_login_authenticator,
                        cli_login_session_store,
                    ),
                ))
            };

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
            {
                serve_config = serve_config.with_protected_route_mount(openai_compat_mount);
            }
            // Google/Slack OAuth start + callback run on the generic
            // recipe-driven engine routes; the deployment client material is
            // wired on the build input (`resolve_google_oauth_config_from_env`
            // in runtime/mod.rs) rather than on the serve config.
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
            // Generic extension channel ingress (extension-runtime P4): one
            // mount serves `/webhooks/extensions/{extension_id}/{route_suffix}`
            // for every active extension; the route table follows the active
            // snapshot.
            if let Some(ingress_parts) = runtime.extension_ingress_parts() {
                let ingress_mount =
                    ironclaw_reborn_composition::extension_ingress_route_mount(&ingress_parts)
                        .context("failed to compose the extension ingress route mount")?;
                serve_config = serve_config.with_public_route_mount(ingress_mount);
            }
            // The generic post-OAuth channel identity binding: installed
            // channel extensions bind through generic discovery over the
            // durable installation store; bindings land in the generic
            // channel-identity store.
            if let Some(channel_identity_binding) = runtime.channel_identity_binding_config() {
                serve_config = serve_config.with_channel_identity_binding(channel_identity_binding);
            }
            // Generic WebGeneratedCode pairing routes (mint/status/unpair per
            // extension), riding the shared protected-route seam.
            if let Some(pairing_mount) = runtime.channel_pairing_route_mount() {
                serve_config = serve_config.with_protected_route_mount(pairing_mount);
            }
            // Public NEAR AI login callback route (token redirect target). Built
            // from the runtime's LLM seam; absent when no LLM was wired.
            if let Some(nearai_mount) = runtime.nearai_login_callback_mount() {
                serve_config = serve_config.with_public_route_mount(nearai_mount);
            }
            if let Some(mount) = public_mount {
                serve_config = serve_config.with_public_route_mount(mount);
            }
            if let Some(cli_login_mount) = cli_login_mount {
                serve_config = serve_config.with_public_route_mount(cli_login_mount);
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
                    shutdown: webui_shutdown_signal(),
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
            shutdown: webui_shutdown_signal(),
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

/// Resolve when a shutdown signal arrives: **SIGTERM** (what orchestrators —
/// Railway, Kubernetes, systemd — send on a deploy/restart) or **SIGINT**
/// (Ctrl-C). Handling SIGTERM is what lets the graceful path
/// (`runtime.shutdown()`, including its in-memory turn-state flush) run on a
/// deploy; without it the process is killed on SIGTERM and in-flight turns are
/// lost. On non-unix, only Ctrl-C is available.
async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut sigterm) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = sigterm.recv() => {}
                }
            }
            // If the SIGTERM handler can't be installed, still honor Ctrl-C.
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn webui_shutdown_signal() -> tokio::sync::oneshot::Receiver<()> {
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        tracing::info!(
            target = "ironclaw::reborn::cli::serve",
            "shutdown signal (SIGTERM/SIGINT) received; signalling WebChat v2 graceful shutdown",
        );
        let _ = shutdown_tx.send(());
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
        "`ironclaw serve` refuses non-loopback listener {host} because the selected \
         runtime policy grants trusted-laptop host access (host-home filesystem, local host \
         process, direct network, inherited environment). Bind to a loopback host such as \
         127.0.0.1 or ::1, or choose a less privileged profile."
    );
}

fn with_product_auth_callback_origin(
    services: RebornHostBindings,
    callback_origin: &str,
) -> anyhow::Result<RebornHostBindings> {
    services
        .with_dcr_oauth_callback(callback_origin)
        .map_err(|error| anyhow!("OAuth callback origin rejected: {error}"))
}

fn webui_product_auth_callback_origin(
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

/// Resolve the fire-time trigger access policy for `serve` from the enabled
/// surfaces (arch-simplification §4.4). Poller off → no authorizer. Poller on →
/// the configured operator owner may fire, and any active canonical tenant
/// member may fire.
///
/// Intentional access model — the auth method is not the gate; active canonical
/// membership is. The `TenantMembership` grant is wired unconditionally whenever
/// the poller is on (there is deliberately no `sso_enabled` condition): a
/// member's signed session is equally valid whether it originated from SSO or
/// the administrator user API, so gating on the auth method would wrongly lock
/// out admin-API-authenticated members from firing their own routines. Safety
/// does not rest on the auth method because the grant is enforced by active
/// membership at fire time: `build_reborn_runtime` turns this `TenantMembership`
/// grant into an `IdentityMembershipTriggerFireChecker` that resolves the
/// creator against the canonical identity directory and denies a suspended,
/// wrong-tenant, or unknown creator (see `crate::trigger_fire_access` in
/// `ironclaw_reborn_composition`). A merely-authenticated non-member therefore
/// cannot fire; only an active member can.
fn trigger_fire_access_policy(
    trigger_poller_enabled: bool,
    user_id: &UserId,
    default_agent_id: &AgentId,
    default_project_id: Option<&ProjectId>,
) -> TriggerFireAccessPolicy {
    if !trigger_poller_enabled {
        return TriggerFireAccessPolicy::disabled();
    }
    TriggerFireAccessPolicy::disabled()
        .with_static_owner(
            user_id.clone(),
            default_agent_id.clone(),
            default_project_id.cloned(),
        )
        .with_tenant_membership(default_agent_id.clone(), default_project_id.cloned())
}

/// The legacy `[slack]` setup fields are a retired configuration surface:
/// Slack is configured by installing the Slack extension and completing
/// workspace OAuth in the WebUI (`/extensions`). A populated setup field
/// means the operator is following retired instructions — fail closed with
/// the migration pointer instead of silently ignoring it. `[slack].enabled`
/// is not rejected: the flag is unused, but existing installs may still
/// carry it and must keep booting.
fn reject_legacy_slack_config(
    config_file: &ironclaw_reborn_config::RebornConfigFile,
    config_path: &std::path::Path,
) -> anyhow::Result<()> {
    let Some(slack) = config_file.slack.as_ref() else {
        return Ok(());
    };
    let offending = [
        ("installation_id", slack.installation_id.is_some()),
        ("team_id", slack.team_id.is_some()),
        ("api_app_id", slack.api_app_id.is_some()),
        ("slack_user_id", slack.slack_user_id.is_some()),
        ("user_id", slack.user_id.is_some()),
        (
            "shared_subject_user_id",
            slack.shared_subject_user_id.is_some(),
        ),
        ("channel_routes", !slack.channel_routes.is_empty()),
        ("signing_secret_env", slack.signing_secret_env.is_some()),
        ("bot_token_env", slack.bot_token_env.is_some()),
    ];
    if let Some((field, _)) = offending.iter().find(|(_, set)| *set) {
        anyhow::bail!(
            "`[slack].{field}` in {path} is a retired configuration surface: Slack is \
             configured by installing the Slack extension and completing workspace OAuth \
             in the WebUI (/extensions). Remove the `[slack]` section to continue.",
            field = field,
            path = config_path.display(),
        );
    }
    Ok(())
}

fn resolve_webui_default_agent(
    identity_section: Option<&IdentitySection>,
    runtime_identity: &RebornRuntimeIdentity,
) -> String {
    identity_section
        .and_then(|identity| identity.default_agent.clone())
        .unwrap_or_else(|| runtime_identity.agent_id.clone())
}

/// Resolution: `env_user_id_var` (non-empty) → config `[identity].default_owner`
/// → `"reborn-cli"` (via `crate::runtime::default_owner_id`).
///
/// A service-installed serve with only HOME/PROFILE in its unit env (no
/// per-operator var) must still boot bound to a stable identity rather than
/// hard-failing — see `resolve_webui_runtime_owner` below, same fallback.
///
/// Uses `present_unicode_env_var` so a non-UTF-8 value for `env_user_id_var`
/// propagates as a startup error instead of being silently treated as
/// unset (the same `NotPresent`-vs-`NotUnicode` distinction documented on
/// `present_unicode_env_var`).
fn resolve_webui_user_id_raw(
    env_user_id_var: &str,
    config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
) -> anyhow::Result<String> {
    Ok(present_unicode_env_var(env_user_id_var)?
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| crate::runtime::default_owner_id(config_file).to_string()))
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
    eprintln!("ironclaw: WebChat v2 listener");
    eprintln!("  binary    : ironclaw");
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

/// Parse `IRONCLAW_REBORN_DEV_SECRET__<handle>=<value>` pairs from an
/// environment snapshot into the `(scope, handle, value)` seeds `serve` writes
/// through `RebornRuntime::seed_local_dev_secret` (#5459 tenant-shared tool
/// credentials). The contract, pinned by the unit tests below:
/// - only names carrying the exact `IRONCLAW_REBORN_DEV_SECRET__` prefix
///   participate; every other env var is ignored;
/// - empty values are skipped (an exported-but-blank var is not a secret);
/// - the suffix IS the [`SecretHandle`] and must be handle-legal (lowercase
///   ASCII); an invalid handle — e.g. a conventionally ALL-CAPS suffix — is a
///   hard startup error, never a silent skip;
/// - every seed targets the caller identity's tenant-shared, admin-managed
///   scope (`tenant_shared_managed_scope`), never the caller's own scope.
///
/// Takes the environment as an iterator parameter so tests never read or
/// mutate process-global env.
fn dev_secret_seeds_from_env(
    vars: impl IntoIterator<Item = (String, String)>,
    tenant_id: &TenantId,
    user_id: &UserId,
    default_agent_id: &AgentId,
    default_project_id: Option<&ProjectId>,
) -> anyhow::Result<Vec<(ResourceScope, SecretHandle, String)>> {
    const DEV_SECRET_PREFIX: &str = "IRONCLAW_REBORN_DEV_SECRET__";
    let mut seeds = Vec::new();
    for (name, value) in vars {
        let Some(handle_raw) = name.strip_prefix(DEV_SECRET_PREFIX) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        let handle = SecretHandle::new(handle_raw)
            .map_err(|err| anyhow!("{name}: invalid secret handle `{handle_raw}`: {err}"))?;
        // The caller invocation owner alias (tenant/user/agent/project),
        // mapped to the tenant-shared scope the runtime's InjectSecretOnce
        // resolution falls back to (caller-first, then tenant-shared).
        let owner = ResourceScope {
            tenant_id: tenant_id.clone(),
            user_id: user_id.clone(),
            agent_id: Some(default_agent_id.clone()),
            project_id: default_project_id.cloned(),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        seeds.push((owner.tenant_shared_managed_scope(), handle, value));
    }
    Ok(seeds)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WEBUI_BASE_URL_ENV: &str = "IRONCLAW_REBORN_WEBUI_BASE_URL";

    #[test]
    fn present_unicode_env_var_treats_unset_as_none() {
        let _guard = crate::runtime::test_env::lock_runtime_env();
        const VAR: &str = "IRONCLAW_REBORN_CLI_TEST_ABSENT_VAR";
        // SAFETY: serialized by `lock_runtime_env`; no other thread touches
        // this test-local var name.
        unsafe { std::env::remove_var(VAR) };
        assert_eq!(
            present_unicode_env_var(VAR).expect("unset is not an error"),
            None
        );
    }

    #[test]
    fn present_unicode_env_var_returns_a_present_value() {
        let _guard = crate::runtime::test_env::lock_runtime_env();
        const VAR: &str = "IRONCLAW_REBORN_CLI_TEST_PRESENT_VAR";
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var(VAR, "a-token-value") };
        let result = present_unicode_env_var(VAR);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe { std::env::remove_var(VAR) };
        assert_eq!(
            result.expect("present unicode value is not an error"),
            Some("a-token-value".to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn present_unicode_env_var_propagates_not_unicode_instead_of_treating_it_as_unset() {
        // Before this fix, `env::var(name).ok()` collapsed `NotUnicode`
        // (a real configuration error — the bearer token env var got
        // mangled into invalid UTF-8) into `None`, silently falling
        // through to the WebChat v2 token file credential instead of
        // failing loudly at startup.
        use std::os::unix::ffi::OsStringExt as _;

        let _guard = crate::runtime::test_env::lock_runtime_env();
        const VAR: &str = "IRONCLAW_REBORN_CLI_TEST_NON_UNICODE_VAR";
        let invalid_utf8 = std::ffi::OsString::from_vec(vec![0xFF, 0xFE, 0xFD]);
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var(VAR, &invalid_utf8) };
        let result = present_unicode_env_var(VAR);
        // SAFETY: serialized by `lock_runtime_env`.
        unsafe { std::env::remove_var(VAR) };

        let error = result.expect_err("non-UTF-8 env value must be a real error, not `Ok(None)`");
        let message = error.to_string();
        assert!(
            message.contains(VAR),
            "error must name the variable: {message}"
        );
        assert!(
            message.contains("not valid UTF-8"),
            "error must explain why: {message}"
        );
    }

    fn clear_webui_env() {
        // SAFETY: tests are serialized by `the shared crate process-env lock`; no other
        // thread reads or writes this env var while the guard is held.
        unsafe { std::env::remove_var(WEBUI_BASE_URL_ENV) };
    }

    fn dev_secret_identity() -> (TenantId, UserId, AgentId) {
        (
            TenantId::new("tenant-a").expect("tenant"),
            UserId::new("user-a").expect("user"),
            AgentId::new("agent-a").expect("agent"),
        )
    }

    /// #5499 review finding #5: the `IRONCLAW_REBORN_DEV_SECRET__` serve
    /// bridge itself was untested — a typo'd prefix, a mis-parsed handle, a
    /// non-skipped empty value, or seeding at the caller's own scope instead
    /// of the tenant-shared one would all reach production unseen. The env
    /// snapshot is an iterator parameter, so no process env is touched.
    #[test]
    fn dev_secret_seeds_parse_prefix_skip_empty_and_target_tenant_shared_scope() {
        let (tenant, user, agent) = dev_secret_identity();
        let vars = vec![
            (
                "IRONCLAW_REBORN_DEV_SECRET__market_data_api_key".to_string(),
                "shared-key".to_string(),
            ),
            // Exported-but-blank must be skipped, not seeded as "".
            (
                "IRONCLAW_REBORN_DEV_SECRET__blank_value".to_string(),
                String::new(),
            ),
            // Non-secret env noise must be ignored, including near-misses
            // that share a shorter IRONCLAW_REBORN_ prefix.
            (
                "IRONCLAW_REBORN_WEBUI_BASE_URL".to_string(),
                "http://localhost:8080".to_string(),
            ),
            ("PATH".to_string(), "/usr/bin".to_string()),
        ];

        let seeds =
            dev_secret_seeds_from_env(vars, &tenant, &user, &agent, None).expect("seeds parse");

        assert_eq!(seeds.len(), 1, "exactly the one prefixed non-empty var");
        let (scope, handle, value) = &seeds[0];
        assert_eq!(handle.as_str(), "market_data_api_key");
        assert_eq!(value, "shared-key");
        // The seed targets the tenant-shared admin-managed scope: tenant
        // preserved, user replaced by the wire-stable shared-owner sentinel
        // (hardcoded here as a tripwire — persisted scopes depend on it),
        // sub-user axes dropped. Seeding at the caller's own scope would
        // make the secret invisible to every other user of the tenant.
        assert_eq!(scope.tenant_id, tenant);
        assert_eq!(scope.user_id.as_str(), "__ironclaw_tenant_shared_admin__");
        assert!(
            scope.agent_id.is_none(),
            "shared scope drops the agent axis"
        );
        assert!(
            scope.project_id.is_none(),
            "shared scope drops the project axis"
        );
    }

    /// The env-var suffix IS the secret handle, and handles are
    /// lowercase-only — an ALL-CAPS suffix (the conventional env style) must
    /// fail serve startup loudly instead of silently skipping the seed and
    /// leaving every tenant user gating on AuthRequired.
    #[test]
    fn dev_secret_invalid_handle_is_a_startup_error() {
        let (tenant, user, agent) = dev_secret_identity();
        let vars = vec![(
            "IRONCLAW_REBORN_DEV_SECRET__MARKET_DATA_API_KEY".to_string(),
            "shared-key".to_string(),
        )];

        let error = dev_secret_seeds_from_env(vars, &tenant, &user, &agent, None)
            .expect_err("an invalid handle suffix must be a startup error");

        let message = format!("{error}");
        assert!(
            message.contains("invalid secret handle") && message.contains("MARKET_DATA_API_KEY"),
            "error must name the offending variable: {message}"
        );
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
        let identity = IdentitySection::default().set_default_agent("configured-agent");

        assert_eq!(
            resolve_webui_default_agent(Some(&identity), &runtime_identity),
            "configured-agent"
        );
    }

    const WEBUI_USER_ID_TEST_ENV: &str = "IRONCLAW_REBORN_SERVE_TEST_USER_ID_RAW";

    #[test]
    fn webui_user_id_raw_prefers_a_set_nonempty_env_var() {
        let _guard = crate::runtime::test_env::lock_runtime_env();
        // SAFETY: serialized by the shared crate process-env lock; cleaned up
        // before the guard drops.
        unsafe { std::env::set_var(WEBUI_USER_ID_TEST_ENV, "env-user") };

        let config_file = ironclaw_reborn_config::RebornConfigFile {
            identity: Some(IdentitySection::default().set_default_owner("config-user")),
            ..Default::default()
        };

        assert_eq!(
            resolve_webui_user_id_raw(WEBUI_USER_ID_TEST_ENV, Some(&config_file))
                .expect("valid unicode env value is not an error"),
            "env-user"
        );

        // SAFETY: see above.
        unsafe { std::env::remove_var(WEBUI_USER_ID_TEST_ENV) };
    }

    #[test]
    fn webui_user_id_raw_falls_back_to_config_default_owner_when_env_absent() {
        let _guard = crate::runtime::test_env::lock_runtime_env();
        // SAFETY: serialized by the shared crate process-env lock.
        unsafe { std::env::remove_var(WEBUI_USER_ID_TEST_ENV) };

        let config_file = ironclaw_reborn_config::RebornConfigFile {
            identity: Some(IdentitySection::default().set_default_owner("config-user")),
            ..Default::default()
        };

        assert_eq!(
            resolve_webui_user_id_raw(WEBUI_USER_ID_TEST_ENV, Some(&config_file))
                .expect("absent env value is not an error"),
            "config-user"
        );
    }

    #[test]
    fn webui_user_id_raw_treats_empty_env_var_as_absent() {
        let _guard = crate::runtime::test_env::lock_runtime_env();
        // SAFETY: serialized by the shared crate process-env lock; cleaned up
        // before the guard drops.
        unsafe { std::env::set_var(WEBUI_USER_ID_TEST_ENV, "") };

        let config_file = ironclaw_reborn_config::RebornConfigFile {
            identity: Some(IdentitySection::default().set_default_owner("config-user")),
            ..Default::default()
        };

        assert_eq!(
            resolve_webui_user_id_raw(WEBUI_USER_ID_TEST_ENV, Some(&config_file))
                .expect("empty env value is not an error"),
            "config-user"
        );

        // SAFETY: see above.
        unsafe { std::env::remove_var(WEBUI_USER_ID_TEST_ENV) };
    }

    #[test]
    fn webui_user_id_raw_defaults_to_reborn_cli_when_no_config_or_env() {
        let _guard = crate::runtime::test_env::lock_runtime_env();
        // SAFETY: serialized by the shared crate process-env lock.
        unsafe { std::env::remove_var(WEBUI_USER_ID_TEST_ENV) };

        assert_eq!(
            resolve_webui_user_id_raw(WEBUI_USER_ID_TEST_ENV, None)
                .expect("no config or env is not an error"),
            "reborn-cli"
        );
    }

    #[cfg(unix)]
    #[test]
    fn webui_user_id_raw_propagates_not_unicode_instead_of_treating_it_as_unset() {
        // Mirrors `present_unicode_env_var_propagates_not_unicode_instead_of_treating_it_as_unset`:
        // before this fix, `resolve_webui_user_id_raw` read the env var with
        // `env::var(..).ok()`, which collapsed `VarError::NotUnicode` (a real
        // misconfiguration — the user-id env var got mangled into invalid
        // UTF-8) into `None`, silently falling through to the config/default
        // owner instead of failing loudly at startup.
        use std::os::unix::ffi::OsStringExt as _;

        let _guard = crate::runtime::test_env::lock_runtime_env();
        let invalid_utf8 = std::ffi::OsString::from_vec(vec![0xFF, 0xFE, 0xFD]);
        // SAFETY: serialized by `lock_runtime_env`; restored below.
        unsafe { std::env::set_var(WEBUI_USER_ID_TEST_ENV, &invalid_utf8) };

        let result = resolve_webui_user_id_raw(WEBUI_USER_ID_TEST_ENV, None);

        // SAFETY: serialized by `lock_runtime_env`.
        unsafe { std::env::remove_var(WEBUI_USER_ID_TEST_ENV) };

        let error =
            result.expect_err("non-UTF-8 env value must be a real error, not a silent fallback");
        let message = error.to_string();
        assert!(
            message.contains(WEBUI_USER_ID_TEST_ENV),
            "error must name the variable: {message}"
        );
        assert!(
            message.contains("not valid UTF-8"),
            "error must explain why: {message}"
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
        let identity = IdentitySection::default().set_default_owner("local-user");

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
        let identity = IdentitySection::default().set_default_owner("reborn-cli");

        let error = resolve_webui_runtime_owner(Some(&identity), "local-user")
            .expect_err("divergent owner must be rejected");
        let message = error.to_string();
        assert!(message.contains("reborn-cli"), "message: {message}");
        assert!(message.contains("local-user"), "message: {message}");
    }

    #[test]
    fn serve_startup_rejects_loaded_config_with_legacy_slack_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            r#"
api_version = "ironclaw.runtime/v1"

[slack]
enabled = true
slack_user_id = "U123"
"#,
        )
        .expect("write config");
        let config_file = ironclaw_reborn_config::RebornConfigFile::load(&config_path)
            .expect("config file loads")
            .expect("config exists");

        let error = reject_legacy_slack_config(&config_file, &config_path)
            .expect_err("serve startup must reject legacy Slack config fields");
        let message = error.to_string();

        assert!(
            message.contains("[slack].slack_user_id"),
            "message: {message}"
        );
        assert!(
            message.contains(&config_path.display().to_string()),
            "message: {message}"
        );
        assert!(message.contains("/extensions"), "message: {message}");

        // Boundary: `enabled` alone is not a setup field. It is unused, but
        // existing installs may still carry it — a boot refusal over an
        // inert flag would strand them.
        std::fs::write(
            &config_path,
            "api_version = \"ironclaw.runtime/v1\"\n\n[slack]\nenabled = true\n",
        )
        .expect("write config");
        let config_file = ironclaw_reborn_config::RebornConfigFile::load(&config_path)
            .expect("config file loads")
            .expect("config exists");
        reject_legacy_slack_config(&config_file, &config_path)
            .expect("an inert [slack].enabled must not block startup");
    }

    #[test]
    fn trigger_poller_disabled_yields_empty_access_policy() {
        let user_id = UserId::new("serve-trigger-disabled-user").expect("user id");
        let agent_id = AgentId::new("serve-trigger-disabled-agent").expect("agent id");
        // Poller off: no fire-time authorizer.
        assert_eq!(
            trigger_fire_access_policy(false, &user_id, &agent_id, None),
            TriggerFireAccessPolicy::disabled()
        );
    }

    #[test]
    fn trigger_poller_without_sso_grants_static_owner_and_tenant_membership() {
        let user_id = UserId::new("serve-trigger-user").expect("user id");
        let agent_id = AgentId::new("serve-trigger-agent").expect("agent id");
        let project_id = ProjectId::new("serve-trigger-project").expect("project id");
        // Poller on, SSO off: admin-created signed-session users are still
        // active canonical tenant members and must be able to fire their own
        // routines. Authentication method does not change membership.
        assert_eq!(
            trigger_fire_access_policy(true, &user_id, &agent_id, Some(&project_id)),
            TriggerFireAccessPolicy::disabled()
                .with_static_owner(user_id.clone(), agent_id.clone(), Some(project_id.clone()),)
                .with_tenant_membership(agent_id.clone(), Some(project_id.clone()))
        );
        // No project scope is carried through exactly (not a wildcard).
        assert_eq!(
            trigger_fire_access_policy(true, &user_id, &agent_id, None),
            TriggerFireAccessPolicy::disabled()
                .with_static_owner(user_id, agent_id.clone(), None)
                .with_tenant_membership(agent_id, None)
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
    async fn webui_serve_wires_product_auth_callback_into_runtime_services() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services_input = with_product_auth_callback_origin(
            ironclaw_reborn_composition::local_dev_build_input(
                "oauth-owner",
                dir.path().join("local-dev"),
            ),
            "http://127.0.0.1:3000",
        )
        .expect("product-auth callback wiring");
        let runtime = ironclaw_reborn_composition::build_reborn_runtime(
            ironclaw_reborn_composition::RebornRuntimeInput::from_build_input(services_input),
        )
        .await
        .expect("reborn runtime builds");

        assert!(
            runtime
                .product_auth_for_test()
                .as_auth_challenge_provider()
                .is_some(),
            "serve wiring must expose the DCR-backed auth challenge provider"
        );
        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn webui_serve_wires_product_auth_callback_with_canonical_host_origin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services_input = with_product_auth_callback_origin(
            ironclaw_reborn_composition::local_dev_build_input(
                "oauth-owner",
                dir.path().join("local-dev"),
            ),
            webui_oauth_callback_origin(
                SocketAddr::from(([0, 0, 0, 0], 3000)),
                None,
                Some("app.example.com"),
            )
            .as_deref()
            .expect("canonical callback origin"),
        )
        .expect("product-auth callback wiring");
        let runtime = ironclaw_reborn_composition::build_reborn_runtime(
            ironclaw_reborn_composition::RebornRuntimeInput::from_build_input(services_input),
        )
        .await
        .expect("reborn runtime builds");

        assert!(
            runtime
                .product_auth_for_test()
                .as_auth_challenge_provider()
                .is_some(),
            "serve wiring must expose the DCR-backed auth challenge provider"
        );
        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn webui_serve_wires_product_auth_callback_with_public_base_url_env_origin() {
        let callback_origin = {
            let _guard = crate::runtime::test_env::lock_runtime_env();
            clear_webui_env();
            // SAFETY: serialized by the shared crate process-env lock; cleaned up before the guard drops.
            unsafe {
                std::env::set_var(WEBUI_BASE_URL_ENV, " https://configured.example/ ");
            }

            let callback_origin =
                webui_product_auth_callback_origin(SocketAddr::from(([0, 0, 0, 0], 8080)), None)
                    .expect("resolve callback origin from env")
                    .expect("public base url env should enable DCR wiring");
            assert_eq!(callback_origin, "https://configured.example");
            clear_webui_env();
            callback_origin
        };

        let dir = tempfile::tempdir().expect("tempdir");
        let services_input = with_product_auth_callback_origin(
            ironclaw_reborn_composition::local_dev_build_input(
                "oauth-owner",
                dir.path().join("local-dev"),
            ),
            &callback_origin,
        )
        .expect("product-auth callback wiring");
        let runtime = ironclaw_reborn_composition::build_reborn_runtime(
            ironclaw_reborn_composition::RebornRuntimeInput::from_build_input(services_input),
        )
        .await
        .expect("reborn runtime builds");

        assert!(
            runtime
                .product_auth_for_test()
                .as_auth_challenge_provider()
                .is_some(),
            "serve wiring must expose the DCR-backed auth challenge provider"
        );
        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[test]
    fn webui_product_auth_callback_origin_rejects_slash_only_public_base_url_env() {
        let _guard = crate::runtime::test_env::lock_runtime_env();
        clear_webui_env();
        // SAFETY: serialized by the shared crate process-env lock; cleaned up before the guard drops.
        unsafe {
            std::env::set_var(WEBUI_BASE_URL_ENV, "/");
        }

        let error =
            webui_product_auth_callback_origin(SocketAddr::from(([0, 0, 0, 0], 8080)), None)
                .expect_err("slash-only base URL must fail closed");
        assert!(
            error.to_string().contains(WEBUI_BASE_URL_ENV),
            "error should name the invalid env var, got: {error}"
        );

        clear_webui_env();
    }

    #[test]
    fn webui_product_auth_callback_origin_rejects_public_cleartext_base_url_env() {
        let _guard = crate::runtime::test_env::lock_runtime_env();
        clear_webui_env();
        // SAFETY: serialized by the shared crate process-env lock; cleaned up before the guard drops.
        unsafe {
            std::env::set_var(WEBUI_BASE_URL_ENV, "http://configured.example");
        }

        let error =
            webui_product_auth_callback_origin(SocketAddr::from(([0, 0, 0, 0], 8080)), None)
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
// arch-exempt: large_file, serve composition remains centralized during assembly cleanup, plan #6175
