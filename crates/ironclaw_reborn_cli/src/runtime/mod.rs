use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::time::Duration;
use std::{future::Future, thread};

use anyhow::Context;
#[cfg(feature = "webui-v2-beta")]
use ironclaw_reborn_composition::host_api::{AgentId, TenantId, UserId};
#[cfg(feature = "webui-v2-beta")]
use ironclaw_reborn_composition::{
    LocalTriggerAccessReconciliation, LocalTriggerAccessRole, LocalTriggerAccessSource,
    open_local_trigger_access_store,
};
use ironclaw_reborn_composition::{
    OAuthClientConfig, PollSettings, RebornBuildInput, RebornCompositionProfile,
    RebornLocalRuntimeProfileOptions, RebornRuntimeIdentity, RebornRuntimeInput,
    TurnRunnerSettings, build_reborn_runtime, local_runtime_build_input_with_options,
};
use ironclaw_reborn_config::{
    REBORN_PROFILE_ENV, RebornBootConfig, RebornProfile, seed_default_config_file_if_missing,
};
use secrecy::SecretString;
use tokio_util::sync::CancellationToken;

use crate::context::RebornCliContext;

#[cfg(test)]
mod test_env;
mod trigger_poller;

use trigger_poller::trigger_poller_settings;

pub(crate) fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::fmt;
    let filter = EnvFilter::try_from_env("IRONCLAW_REBORN_LOG").unwrap_or_else(|_| {
        EnvFilter::new("info,ironclaw_reborn=info,ironclaw_reborn_composition=info")
    });
    let _ = fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

pub(crate) fn block_on_cli<F, T, E>(future: F) -> anyhow::Result<T>
where
    F: Future<Output = Result<T, E>> + Send + 'static,
    T: Send + 'static,
    E: Into<anyhow::Error> + Send + 'static,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        return thread::spawn(move || block_on_cli_future(future))
            .join()
            .map_err(|_| anyhow::anyhow!("CLI async task thread panicked"))?;
    }
    block_on_cli_future(future)
}

fn block_on_cli_future<F, T, E>(future: F) -> anyhow::Result<T>
where
    F: Future<Output = Result<T, E>>,
    E: Into<anyhow::Error>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(future).map_err(Into::into)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RuntimeInputOptions {
    pub(crate) confirm_host_access: bool,
}

pub(crate) fn execute(
    context: RebornCliContext,
    message: Option<String>,
    options: RuntimeInputOptions,
) -> anyhow::Result<()> {
    let runtime_input =
        build_runtime_input_with_options(context.boot_config(), RuntimeInputCaller::Run, options)?;
    seed_default_config_file_if_missing(&context.boot_config().home().config_file_path())
        .map_err(anyhow::Error::from)?;
    let boot_config = context.boot_config().clone();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let runtime_input =
            with_run_local_trigger_fire_access_checker(runtime_input, &boot_config).await?;
        let runtime = build_reborn_runtime(runtime_input).await?;
        print_runtime_banner(&boot_config);

        let conversation = runtime.new_conversation().await?;
        let cancellation = install_ctrl_c_cancellation();

        let outcome = if let Some(text) = message {
            send_once(&runtime, &conversation, &text, cancellation).await
        } else {
            run_repl_loop(&runtime, &conversation, cancellation).await
        };

        runtime.shutdown().await?;
        outcome
    })?;
    Ok(())
}

async fn with_run_local_trigger_fire_access_checker(
    runtime_input: RebornRuntimeInput,
    config: &RebornBootConfig,
) -> anyhow::Result<RebornRuntimeInput> {
    #[cfg(not(feature = "webui-v2-beta"))]
    {
        let _ = config;
        Ok(runtime_input)
    }

    #[cfg(feature = "webui-v2-beta")]
    {
        if !runtime_input.trigger_poller.enabled {
            return Ok(runtime_input);
        }

        let config_file = read_config_file(config)?;
        let tenant_id = TenantId::new(&runtime_input.identity.tenant_id).with_context(|| {
            format!(
                "[identity].tenant `{}` is invalid",
                runtime_input.identity.tenant_id
            )
        })?;
        let user_id = UserId::new(default_owner_id(config_file.as_ref()))
            .context("[identity].default_owner is invalid")?;
        let agent_id = AgentId::new(&runtime_input.identity.agent_id).with_context(|| {
            format!(
                "[identity].default_agent `{}` is invalid",
                runtime_input.identity.agent_id
            )
        })?;
        let user_store_path = config
            .home()
            .path()
            .join("local-dev")
            .join("reborn-local-dev.db");
        let access_store = open_local_trigger_access_store(&user_store_path)
            .await
            .context("failed to initialize local trigger-fire access store for `run`")?;
        let user_ids = [user_id];
        access_store
            .reconcile_local_access(LocalTriggerAccessReconciliation {
                tenant_id: &tenant_id,
                user_ids: &user_ids,
                agent_id: Some(&agent_id),
                project_id: None,
                role: LocalTriggerAccessRole::Owner,
                source: LocalTriggerAccessSource::LocalDevRunBootstrap,
            })
            .await
            .context("failed to reconcile local trigger-fire access for `run`")?;

        Ok(runtime_input.with_trigger_fire_access_checker(access_store))
    }
}

fn print_runtime_banner(config: &RebornBootConfig) {
    eprintln!("ironclaw-reborn: runtime started");
    eprintln!("  profile     : {}", config.profile());
    eprintln!("  reborn_home : {}", config.home().path().display());
    eprintln!();
}

async fn send_once(
    runtime: &ironclaw_reborn_composition::RebornRuntime,
    conversation: &ironclaw_reborn_composition::ConversationId,
    text: &str,
    cancellation: CancellationToken,
) -> anyhow::Result<()> {
    let reply = runtime
        .send_user_message_with_cancellation(conversation, text, cancellation)
        .await?;
    if !reply.is_successful_final_reply() {
        anyhow::bail!(
            "reborn run did not produce an assistant reply (status={:?}, run_id={})",
            reply.status,
            reply.run_id
        );
    }
    print_reply(&reply);
    Ok(())
}

async fn run_repl_loop(
    runtime: &ironclaw_reborn_composition::RebornRuntime,
    conversation: &ironclaw_reborn_composition::ConversationId,
    cancellation: CancellationToken,
) -> anyhow::Result<()> {
    let stdin_is_tty = std::io::stdin().is_terminal();
    if stdin_is_tty {
        eprintln!("(repl) type a message and press enter; Ctrl-D to exit");
    }
    let stdin = tokio::io::stdin();
    let reader = tokio::io::BufReader::new(stdin);
    use tokio::io::AsyncBufReadExt;
    let mut lines = reader.lines();

    loop {
        if stdin_is_tty {
            // Prompt to stderr so stdout stays clean for piping.
            eprint!("> ");
            let _ = std::io::stderr().flush();
        }
        tokio::select! {
            line = lines.next_line() => {
                match line? {
                    Some(text) if text.trim().is_empty() => continue,
                    Some(text) if is_exit_command(&text) => return Ok(()),
                    Some(text) if is_help_command(&text) => {
                        print_repl_help();
                        continue;
                    }
                    Some(text) => {
                        match runtime
                            .send_user_message_with_cancellation(
                                conversation,
                                &text,
                                cancellation.clone(),
                            )
                            .await
                        {
                            Ok(reply) if reply.is_successful_final_reply() => print_reply(&reply),
                            Ok(reply) if stdin_is_tty => print_reply(&reply),
                            Ok(reply) => {
                                anyhow::bail!(
                                    "reborn run did not produce an assistant reply (status={:?}, run_id={})",
                                    reply.status,
                                    reply.run_id
                                );
                            }
                            Err(error) if stdin_is_tty => {
                                eprintln!("error: {error}");
                                if cancellation.is_cancelled() {
                                    return Ok(());
                                }
                            }
                            Err(error) => return Err(error.into()),
                        }
                    }
                    None => {
                        if stdin_is_tty {
                            eprintln!();
                        }
                        return Ok(());
                    }
                }
            }
            _ = cancellation.cancelled() => {
                eprintln!();
                eprintln!("(repl) caught ctrl-c, shutting down");
                return Ok(());
            }
        }
    }
}

fn is_exit_command(text: &str) -> bool {
    matches!(text.trim(), "/exit" | "/quit")
}

fn is_help_command(text: &str) -> bool {
    text.trim() == "/help"
}

fn print_repl_help() {
    eprintln!("Reborn REPL commands:");
    eprintln!("  /help  Show this help");
    eprintln!("  /exit  Exit the REPL");
    eprintln!("  /quit  Exit the REPL");
}

fn print_reply(reply: &ironclaw_reborn_composition::AssistantReply) {
    match reply.text.as_deref() {
        Some(text) => println!("{text}"),
        None => eprintln!(
            "(no assistant text; status={:?}, run_id={})",
            reply.status, reply.run_id
        ),
    }
}

fn install_ctrl_c_cancellation() -> CancellationToken {
    let cancellation = CancellationToken::new();
    let ctrl_c_cancellation = cancellation.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            ctrl_c_cancellation.cancel();
        }
    });
    cancellation
}

/// Which subcommand is asking for the runtime input. Used to decide
/// which `[identity]` / `[…]` config sections are legitimate vs.
/// "parsed but not wired" — the runtime slice today does not honor
/// `[identity].default_project`, but the `serve` subcommand stamps it
/// onto every authenticated WebUI caller and therefore consumes it
/// directly. Without this discriminator the shared `build_runtime_input`
/// would reject `serve` configs that legitimately set
/// `default_project`. See the `reject_unsupported_runtime_sections`
/// branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeInputCaller {
    Run,
    Serve,
}

#[cfg(test)]
pub(crate) fn build_runtime_input(
    config: &RebornBootConfig,
    caller: RuntimeInputCaller,
) -> anyhow::Result<RebornRuntimeInput> {
    build_runtime_input_with_options(config, caller, RuntimeInputOptions::default())
}

pub(crate) fn build_runtime_input_with_options(
    config: &RebornBootConfig,
    caller: RuntimeInputCaller,
    options: RuntimeInputOptions,
) -> anyhow::Result<RebornRuntimeInput> {
    let runtime_services = build_services_input_with_options(config, caller, options)?;

    #[allow(unused_mut)]
    let mut runtime_input = RebornRuntimeInput::from_services(runtime_services.services_input)
        .with_runner_settings(runner_settings(runtime_services.config_file.as_ref())?)
        .with_trigger_poller_settings(trigger_poller_settings(
            runtime_services.config_file.as_ref(),
        )?)
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(200),
            max_total: Duration::from_secs(180),
        })
        .with_identity(runtime_identity(runtime_services.config_file.as_ref()))
        .with_regex_skill_activation_enabled(regex_skill_activation_enabled(
            runtime_services.config_file.as_ref(),
        ));

    #[cfg(feature = "root-llm-provider")]
    {
        match ironclaw_reborn_composition::resolve_reborn_runtime_llm(
            config,
            runtime_services.config_file.as_ref(),
        )? {
            Some(llm) => {
                tracing::debug!(
                    provider_id = %llm.provider_id(),
                    model = %llm.model(),
                    "resolved LLM selection for Reborn runtime"
                );
                runtime_input = runtime_input.with_resolved_llm(llm);
            }
            None => {
                tracing::warn!(
                    "no LLM selection configured; set `[llm.default]` in {} or configure \
                     LLM_BACKEND / provider environment variables. Runs will fail until an \
                     LLM is wired.",
                    config.home().config_file_path().display()
                );
            }
        }
    }

    Ok(runtime_input)
}

pub(crate) struct RuntimeServicesInput {
    pub(crate) services_input: RebornBuildInput,
    config_file: Option<ironclaw_reborn_config::RebornConfigFile>,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedGoogleOAuthConfig {
    pub(crate) client: OAuthClientConfig,
    pub(crate) hosted_domain_hint: Option<String>,
}

pub(crate) fn build_services_input_with_options(
    config: &RebornBootConfig,
    caller: RuntimeInputCaller,
    options: RuntimeInputOptions,
) -> anyhow::Result<RuntimeServicesInput> {
    // Read the operator's boot TOML if present. Missing file is OK
    // (operator may not have run `ironclaw-reborn config init` yet);
    // sparse fields are OK (each absent field falls back to the
    // CLI-shaped default baked into composition).
    let config_file = read_config_file(config)?;

    let owner_id = default_owner_id(config_file.as_ref());

    let profile = effective_profile(config, config_file.as_ref())?;
    reject_unsupported_runtime_sections(config_file.as_ref(), caller, profile)?;
    let mut services_input = match profile {
        RebornProfile::LocalDev | RebornProfile::LocalDevYolo => {
            let local_dev_root: PathBuf = config.home().path().join("local-dev");
            let workspace_root = std::env::current_dir()
                .context("failed to resolve current directory for local-dev workspace")?;
            let mut services_input = local_runtime_build_input_with_options(
                composition_profile(profile),
                owner_id,
                local_dev_root,
                RebornLocalRuntimeProfileOptions {
                    confirm_host_access: options.confirm_host_access,
                },
            )
            .with_context(|| {
                format!(
                    "ironclaw-reborn run currently supports profile=local-dev or profile=local-dev-yolo; \
                     got profile={profile}."
                )
            })?
            .with_local_dev_workspace_root(workspace_root);
            if services_input.requires_local_dev_confirmed_host_home_root() {
                let host_home_root =
                    confirmed_host_home_root(options).context("local-dev-yolo host access")?;
                services_input =
                    services_input.with_local_dev_confirmed_host_home_root(host_home_root);
            }
            services_input
        }
        RebornProfile::Production | RebornProfile::MigrationDryRun => {
            // MigrationDryRun needs production storage handles so follow-up migration
            // code can inspect durable schema state; this branch only constructs
            // those handles and does not execute migration writes.
            build_production_services_input(profile, owner_id, config_file.as_ref())?
        }
    };
    if let Some(ResolvedGoogleOAuthConfig {
        client,
        hosted_domain_hint: _hosted_domain_hint,
    }) = resolve_google_oauth_config_from_env()?
    {
        services_input = services_input.with_google_oauth_backend(client);
    }

    Ok(RuntimeServicesInput {
        services_input,
        config_file,
    })
}

#[cfg(feature = "postgres")]
fn build_production_services_input(
    profile: RebornProfile,
    owner_id: &str,
    config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
) -> anyhow::Result<RebornBuildInput> {
    RebornBuildInput::postgres_from_config_and_env(
        composition_profile(profile),
        owner_id,
        config_file,
    )
    .map_err(anyhow::Error::from)
}
#[cfg(not(feature = "postgres"))]
fn build_production_services_input(
    profile: RebornProfile,
    _owner_id: &str,
    _config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
) -> anyhow::Result<RebornBuildInput> {
    anyhow::bail!(
        "profile={profile} requires a binary built with the `postgres` feature for production \
         storage; the default PostgreSQL URL env var is IRONCLAW_REBORN_POSTGRES_URL"
    )
}

pub(crate) fn resolve_google_oauth_config_from_env()
-> anyhow::Result<Option<ResolvedGoogleOAuthConfig>> {
    resolve_google_oauth_config(optional_nonempty_env)
}

fn resolve_google_oauth_config(
    mut lookup: impl FnMut(&str) -> Option<String>,
) -> anyhow::Result<Option<ResolvedGoogleOAuthConfig>> {
    let reborn_client_id = lookup("IRONCLAW_REBORN_GOOGLE_CLIENT_ID");
    let reborn_redirect_uri = lookup("IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI");
    let reborn_client_secret = lookup("IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET");
    let reborn_hosted_domain_hint = lookup("IRONCLAW_REBORN_GOOGLE_HOSTED_DOMAIN_HINT");
    let legacy_client_id = lookup("GOOGLE_CLIENT_ID");
    let legacy_client_secret = lookup("GOOGLE_CLIENT_SECRET");
    let legacy_redirect_uri = lookup("GOOGLE_OAUTH_REDIRECT_URI");
    let legacy_hosted_domain_hint = lookup("GOOGLE_ALLOWED_HD");

    if reborn_client_id.is_none()
        && reborn_redirect_uri.is_none()
        && reborn_client_secret.is_none()
        && reborn_hosted_domain_hint.is_none()
        && legacy_client_id.is_none()
        && legacy_client_secret.is_none()
        && legacy_redirect_uri.is_none()
        && legacy_hosted_domain_hint.is_none()
    {
        return Ok(None);
    }

    let client_id = reborn_client_id
        .or(legacy_client_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "IRONCLAW_REBORN_GOOGLE_CLIENT_ID or GOOGLE_CLIENT_ID is required for Google OAuth setup"
            )
        })?;
    let redirect_uri = reborn_redirect_uri.or(legacy_redirect_uri).ok_or_else(|| {
        anyhow::anyhow!(
            "IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI or GOOGLE_OAUTH_REDIRECT_URI is required for Google OAuth setup"
        )
    })?;
    let client_secret = reborn_client_secret
        .or(legacy_client_secret)
        .map(SecretString::from);
    if client_secret.is_none() {
        tracing::warn!(
            target = "ironclaw::reborn::cli::google_oauth",
            "Google OAuth setup config has no client secret; token exchange will use public-client PKCE",
        );
    }
    let hosted_domain_hint = reborn_hosted_domain_hint.or(legacy_hosted_domain_hint);
    let mut client = OAuthClientConfig::new(client_id, redirect_uri, client_secret)
        .context("invalid Google OAuth client configuration")?;
    if let Some(hosted_domain_hint) = hosted_domain_hint.clone() {
        client = client.with_hosted_domain_hint(hosted_domain_hint);
    }

    Ok(Some(ResolvedGoogleOAuthConfig {
        client,
        hosted_domain_hint,
    }))
}

/// Read an env var with lenient presence semantics: unset OR
/// present-but-blank both collapse to `None`. Used for optional-config
/// callers (OAuth client overrides, etc.) where a blank slot is benign.
///
/// **Not** for operator-control knobs like `IRONCLAW_TRIGGER_POLLER_*` —
/// those use a strict-presence variant in the `trigger_poller` submodule,
/// which treats a present-but-blank value as a fatal misconfiguration.
fn optional_nonempty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn default_owner_id(
    config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
) -> &str {
    config_file
        .and_then(|file| file.identity.as_ref())
        .and_then(|identity| identity.default_owner.as_deref())
        .unwrap_or("reborn-cli")
}

fn confirmed_host_home_root(options: RuntimeInputOptions) -> anyhow::Result<PathBuf> {
    debug_assert!(options.confirm_host_access);
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .context("HOME or USERPROFILE must be set")
}

fn composition_profile(profile: RebornProfile) -> RebornCompositionProfile {
    match profile {
        RebornProfile::LocalDev => RebornCompositionProfile::LocalDev,
        RebornProfile::LocalDevYolo => RebornCompositionProfile::LocalDevYolo,
        RebornProfile::Production => RebornCompositionProfile::Production,
        RebornProfile::MigrationDryRun => RebornCompositionProfile::MigrationDryRun,
    }
}

pub(crate) fn read_config_file(
    config: &RebornBootConfig,
) -> anyhow::Result<Option<ironclaw_reborn_config::RebornConfigFile>> {
    use ironclaw_reborn_config::RebornConfigFile;
    let path = config.home().config_file_path();
    let file = RebornConfigFile::load(&path).map_err(anyhow::Error::from)?;
    if let Some(parsed) = &file {
        tracing::debug!(
            path = %path.display(),
            api_version = ?parsed.api_version,
            "loaded boot config TOML"
        );
    }
    Ok(file)
}

// CLI-local operator config only. Product/WebUI identity must come from
// trusted host installation/binding resolution, not inbound payloads.
fn runtime_identity(
    config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
) -> RebornRuntimeIdentity {
    let default = RebornRuntimeIdentity::reborn_cli();
    let Some(identity) = config_file.and_then(|file| file.identity.as_ref()) else {
        return default;
    };

    RebornRuntimeIdentity {
        tenant_id: identity
            .tenant
            .clone()
            .unwrap_or_else(|| default.tenant_id.clone()),
        agent_id: identity
            .default_agent
            .clone()
            .unwrap_or_else(|| default.agent_id.clone()),
        source_binding_id: default.source_binding_id,
        reply_target_binding_id: default.reply_target_binding_id,
    }
}

fn regex_skill_activation_enabled(
    config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
) -> bool {
    config_file
        .and_then(|file| file.skills.as_ref())
        .and_then(|skills| skills.regex_activation_enabled)
        .unwrap_or(true)
}

pub(crate) fn effective_profile(
    config: &RebornBootConfig,
    config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
) -> anyhow::Result<RebornProfile> {
    // Env wins over file. `RebornBootConfig` already parsed/validated env,
    // so if the variable is present we keep that value.
    if std::env::var_os(REBORN_PROFILE_ENV).is_some() {
        return Ok(config.profile());
    }

    let Some(profile) = config_file
        .and_then(|file| file.boot.as_ref())
        .and_then(|boot| boot.profile.as_deref())
    else {
        return Ok(config.profile());
    };

    profile.parse::<RebornProfile>().map_err(|error| {
        anyhow::anyhow!("config file [boot].profile `{profile}` is invalid: {error}")
    })
}

fn reject_unsupported_runtime_sections(
    config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
    caller: RuntimeInputCaller,
    profile: RebornProfile,
) -> anyhow::Result<()> {
    let Some(file) = config_file else {
        return Ok(());
    };

    // `[identity].default_project` is parsed but not yet wired into the
    // generic runtime slice — `run` / `repl` would silently drop the value,
    // so we fail-loud. The `serve` subcommand DOES consume it (stamped onto
    // every `WebUiAuthenticatedCaller`), so for that caller the field is
    // supported, not "parsed but not wired".
    if let Some(identity) = file.identity.as_ref()
        && identity.default_project.is_some()
        && caller != RuntimeInputCaller::Serve
    {
        anyhow::bail!(
            "config file [identity] field default_project is parsed but not wired in this runtime slice; \
             leave it commented until project-scope wiring lands"
        );
    }

    let mut sections = Vec::new();
    if file.policy.is_some()
        && !matches!(
            profile,
            RebornProfile::Production | RebornProfile::MigrationDryRun
        )
    {
        sections.push("[policy]");
    }
    if file.drivers.is_some() {
        sections.push("[drivers]");
    }
    if file.harness.is_some() {
        sections.push("[harness]");
    }
    if sections.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "config file section(s) {} are parsed but not wired in this runtime slice; \
             leave them commented until epic #3036 substrate lands",
            sections.join(", ")
        )
    }
}

fn runner_settings(
    config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
) -> anyhow::Result<TurnRunnerSettings> {
    let mut settings = TurnRunnerSettings::default();
    if let Some(runner) = config_file.and_then(|file| file.runner.as_ref()) {
        if let Some(secs) = runner.heartbeat_interval_secs {
            if secs == 0 {
                anyhow::bail!(
                    "config file [runner].heartbeat_interval_secs must be greater than 0"
                );
            }
            settings.heartbeat_interval = Duration::from_secs(secs);
        }
        if let Some(ms) = runner.poll_interval_ms {
            if ms == 0 {
                anyhow::bail!("config file [runner].poll_interval_ms must be greater than 0");
            }
            settings.poll_interval = Duration::from_millis(ms);
        }
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ironclaw_reborn_composition::RebornCompositionProfile;
    #[cfg(feature = "webui-v2-beta")]
    use ironclaw_reborn_composition::{LocalTriggerAccessRole, LocalTriggerAccessSource};
    use ironclaw_reborn_config::RebornBootConfig;

    use super::test_env::{EnvGuard, lock_trigger_env};
    #[cfg(feature = "webui-v2-beta")]
    use super::with_run_local_trigger_fire_access_checker;
    use super::{
        RuntimeInputCaller, RuntimeInputOptions, block_on_cli, build_runtime_input,
        build_runtime_input_with_options, resolve_google_oauth_config,
    };

    fn clear_trigger_poller_env() -> (EnvGuard, EnvGuard) {
        (
            EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_ENABLED"),
            EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS"),
        )
    }

    #[cfg(feature = "postgres")]
    fn clear_reborn_postgres_tls_env() -> (EnvGuard, EnvGuard) {
        (
            EnvGuard::clear("DATABASE_SSLMODE"),
            EnvGuard::clear("IRONCLAW_REBORN_ALLOW_REMOTE_POSTGRES_CLEAR_TEXT"),
        )
    }

    #[tokio::test]
    async fn block_on_cli_can_run_inside_existing_tokio_runtime() {
        let value = block_on_cli(async { Ok::<_, anyhow::Error>(42) }).expect("block future");

        assert_eq!(value, 42);
    }

    #[test]
    fn build_runtime_input_maps_configured_cli_identity() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        std::fs::write(
            reborn_home.join("config.toml"),
            r#"
[identity]
tenant = "custom-tenant"
default_agent = "custom-agent"
default_owner = "custom-owner"
"#,
        )
        .expect("write config");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.into_os_string()),
            None,
            None,
            None,
        )
        .expect("boot config");

        let runtime_input =
            build_runtime_input(&config, RuntimeInputCaller::Run).expect("runtime input");

        assert_eq!(runtime_input.identity.tenant_id, "custom-tenant");
        assert_eq!(runtime_input.identity.agent_id, "custom-agent");
        assert_eq!(runtime_input.identity.source_binding_id, "reborn-cli");
        assert_eq!(runtime_input.identity.reply_target_binding_id, "reborn-cli");
    }

    #[test]
    fn build_runtime_input_maps_regex_skill_activation_config() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        std::fs::write(
            reborn_home.join("config.toml"),
            r#"
[skills]
regex_activation_enabled = false
"#,
        )
        .expect("write config");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.into_os_string()),
            None,
            None,
            None,
        )
        .expect("boot config");

        let runtime_input =
            build_runtime_input(&config, RuntimeInputCaller::Run).expect("runtime input");

        assert!(!runtime_input.regex_skill_activation_enabled);
    }

    #[test]
    fn build_runtime_input_rejects_local_dev_yolo_without_host_access_confirmation() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.into_os_string()),
            None,
            None,
            Some("local-dev-yolo".into()),
        )
        .expect("boot config");

        let error = match build_runtime_input(&config, RuntimeInputCaller::Run) {
            Ok(_) => panic!("local-dev-yolo requires confirmation"),
            Err(error) => error,
        };

        assert!(format!("{error:#}").contains("requires explicit disclosure acknowledgement"));
    }

    #[test]
    fn build_runtime_input_accepts_confirmed_local_dev_yolo_profile() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.into_os_string()),
            None,
            None,
            Some("local-dev-yolo".into()),
        )
        .expect("boot config");

        let runtime_input = build_runtime_input_with_options(
            &config,
            RuntimeInputCaller::Run,
            RuntimeInputOptions {
                confirm_host_access: true,
            },
        )
        .expect("runtime input");
        assert!(runtime_input.grants_trusted_laptop_access());
        let services = runtime_input.services.expect("services input");
        let policy = services.runtime_policy().expect("runtime policy");

        assert_eq!(services.profile(), RebornCompositionProfile::LocalDevYolo);
        assert_eq!(
            policy.filesystem_backend.as_str(),
            "host_workspace_and_home"
        );
        assert_eq!(policy.secret_mode.as_str(), "inherited_env");
    }

    #[cfg(feature = "postgres")]
    fn boot_config_with_config_toml(
        profile: &str,
        config_toml: &str,
    ) -> (tempfile::TempDir, RebornBootConfig) {
        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        std::fs::write(reborn_home.join("config.toml"), config_toml).expect("write config");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.into_os_string()),
            None,
            None,
            Some(profile.into()),
        )
        .expect("boot config");
        (temp, config)
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_for_local_dev_rejects_policy_section() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let (_temp, config) = boot_config_with_config_toml(
            "local-dev",
            r#"
[policy]
deployment_mode = "hosted_multi_tenant"
default_profile = "secure_default"
"#,
        );

        let err = build_runtime_input(&config, RuntimeInputCaller::Run)
            .err()
            .expect("local-dev must reject policy section");

        assert!(
            err.to_string().contains("[policy]"),
            "error must mention policy section, got: {err:#}"
        );
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_requires_storage_section() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let _postgres_url = EnvGuard::clear("IRONCLAW_REBORN_POSTGRES_URL");
        let _secret_master_key = EnvGuard::clear("IRONCLAW_REBORN_SECRET_MASTER_KEY");

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.into_os_string()),
            None,
            None,
            Some("production".into()),
        )
        .expect("boot config");

        let err = build_runtime_input(&config, RuntimeInputCaller::Run)
            .err()
            .expect("production requires explicit storage config");

        assert!(
            err.to_string().contains("[storage]"),
            "error must mention storage config, got: {err:#}"
        );
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_requires_postgres_url_env_value() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let _postgres_url = EnvGuard::clear("IRONCLAW_REBORN_POSTGRES_URL");
        let _secret_master_key = EnvGuard::clear("IRONCLAW_REBORN_SECRET_MASTER_KEY");

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        std::fs::write(
            reborn_home.join("config.toml"),
            r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"

[policy]
deployment_mode = "hosted_multi_tenant"
default_profile = "secure_default"
"#,
        )
        .expect("write config");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.into_os_string()),
            None,
            None,
            Some("production".into()),
        )
        .expect("boot config");

        let err = build_runtime_input(&config, RuntimeInputCaller::Run)
            .err()
            .expect("missing Postgres URL env must fail closed");

        assert!(
            err.to_string().contains("IRONCLAW_REBORN_POSTGRES_URL"),
            "error must mention missing env var name, got: {err:#}"
        );
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_storage_section_missing_backend_field() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let _postgres_url = EnvGuard::clear("IRONCLAW_REBORN_POSTGRES_URL");
        let _secret_master_key = EnvGuard::clear("IRONCLAW_REBORN_SECRET_MASTER_KEY");
        let (_temp, config) = boot_config_with_config_toml(
            "production",
            r#"
[storage]
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"
"#,
        );

        let err = build_runtime_input(&config, RuntimeInputCaller::Run)
            .err()
            .expect("missing backend must fail closed");
        assert!(
            err.to_string().contains("backend"),
            "error must mention missing backend field, got: {err:#}"
        );
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_requires_policy_section() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let _postgres_url = EnvGuard::set(
            "IRONCLAW_REBORN_POSTGRES_URL",
            "postgres://event_user:RAW_PASSWORD_SENTINEL_3162@db.example.com/events?sslmode=require",
        );
        let _secret_master_key = EnvGuard::set(
            "IRONCLAW_REBORN_SECRET_MASTER_KEY",
            "test-secret-master-key",
        );
        let (_temp, config) = boot_config_with_config_toml(
            "production",
            r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"
"#,
        );

        let err = build_runtime_input(&config, RuntimeInputCaller::Run)
            .err()
            .expect("production requires policy config");

        assert!(
            err.to_string().contains("[policy]"),
            "error must mention policy config, got: {err:#}"
        );
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_rejects_invalid_policy_deployment_mode() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let _postgres_url = EnvGuard::set(
            "IRONCLAW_REBORN_POSTGRES_URL",
            "postgres://event_user:RAW_PASSWORD_SENTINEL_3162@db.example.com/events?sslmode=require",
        );
        let _secret_master_key = EnvGuard::set(
            "IRONCLAW_REBORN_SECRET_MASTER_KEY",
            "test-secret-master-key",
        );
        let (_temp, config) = boot_config_with_config_toml(
            "production",
            r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"

[policy]
deployment_mode = "not_a_deployment"
default_profile = "secure_default"
"#,
        );

        let err = build_runtime_input(&config, RuntimeInputCaller::Run)
            .err()
            .expect("invalid deployment mode must fail closed");

        assert!(
            format!("{err:#}").contains("deployment_mode"),
            "error must mention deployment_mode, got: {err:#}"
        );
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_rejects_invalid_policy_default_profile() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let _postgres_url = EnvGuard::set(
            "IRONCLAW_REBORN_POSTGRES_URL",
            "postgres://event_user:RAW_PASSWORD_SENTINEL_3162@db.example.com/events?sslmode=require",
        );
        let _secret_master_key = EnvGuard::set(
            "IRONCLAW_REBORN_SECRET_MASTER_KEY",
            "test-secret-master-key",
        );
        let (_temp, config) = boot_config_with_config_toml(
            "production",
            r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"

[policy]
deployment_mode = "hosted_multi_tenant"
default_profile = "not_a_profile"
"#,
        );

        let err = build_runtime_input(&config, RuntimeInputCaller::Run)
            .err()
            .expect("invalid default profile must fail closed");

        assert!(
            format!("{err:#}").contains("default_profile"),
            "error must mention default_profile, got: {err:#}"
        );
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_rejects_unsupported_backend() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let (_temp, config) = boot_config_with_config_toml(
            "production",
            r#"
[storage]
backend = "libsql"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"
"#,
        );

        let err = build_runtime_input(&config, RuntimeInputCaller::Run)
            .err()
            .expect("unsupported backend must fail closed");
        assert!(
            err.to_string().contains("postgres") && err.to_string().contains("libsql"),
            "error must mention supported and bad backend values, got: {err:#}"
        );
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_rejects_whitespace_only_postgres_url() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let _postgres_url = EnvGuard::set("IRONCLAW_REBORN_POSTGRES_URL", "   ");
        let _secret_master_key =
            EnvGuard::set("IRONCLAW_REBORN_SECRET_MASTER_KEY", "test-master-key");
        let (_temp, config) = boot_config_with_config_toml(
            "production",
            r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"
"#,
        );

        let err = build_runtime_input(&config, RuntimeInputCaller::Run)
            .err()
            .expect("whitespace-only URL env must fail closed");
        assert!(
            err.to_string().contains("empty"),
            "error must mention empty URL env var, got: {err:#}"
        );
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_preserves_whitespace_secret_master_key() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let _postgres_url = EnvGuard::set(
            "IRONCLAW_REBORN_POSTGRES_URL",
            "postgres://localhost/ironclaw_reborn_cli_test",
        );
        let _secret_master_key = EnvGuard::set("IRONCLAW_REBORN_SECRET_MASTER_KEY", "   ");
        let (_temp, config) = boot_config_with_config_toml(
            "production",
            r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"

[policy]
deployment_mode = "hosted_multi_tenant"
default_profile = "secure_default"
"#,
        );

        let runtime_input =
            build_runtime_input(&config, RuntimeInputCaller::Run).expect("runtime input");
        let services = runtime_input.services.expect("services input");
        assert_eq!(services.profile(), RebornCompositionProfile::Production);
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_uses_custom_url_env_name() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let _default_postgres_url = EnvGuard::clear("IRONCLAW_REBORN_POSTGRES_URL");
        let _custom_postgres_url = EnvGuard::set(
            "IRONCLAW_REBORN_CUSTOM_POSTGRES_URL",
            "postgres://localhost/ironclaw_reborn_cli_test",
        );
        let _secret_master_key =
            EnvGuard::set("IRONCLAW_REBORN_SECRET_MASTER_KEY", "test-master-key");
        let (_temp, config) = boot_config_with_config_toml(
            "production",
            r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_CUSTOM_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"

[policy]
deployment_mode = "hosted_multi_tenant"
default_profile = "secure_default"
"#,
        );

        let runtime_input =
            build_runtime_input(&config, RuntimeInputCaller::Run).expect("runtime input");
        let services = runtime_input.services.expect("services input");
        assert_eq!(services.profile(), RebornCompositionProfile::Production);
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_constructs_migration_dry_run_services_input() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let _postgres_url = EnvGuard::set(
            "IRONCLAW_REBORN_POSTGRES_URL",
            "postgres://localhost/ironclaw_reborn_cli_test",
        );
        let _secret_master_key =
            EnvGuard::set("IRONCLAW_REBORN_SECRET_MASTER_KEY", "test-master-key");
        let (_temp, config) = boot_config_with_config_toml(
            "migration-dry-run",
            r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"

[policy]
deployment_mode = "hosted_multi_tenant"
default_profile = "secure_default"
"#,
        );

        let runtime_input =
            build_runtime_input(&config, RuntimeInputCaller::Run).expect("runtime input");
        let services = runtime_input.services.expect("services input");
        assert_eq!(
            services.profile(),
            RebornCompositionProfile::MigrationDryRun
        );
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_requires_secret_master_key_env_value() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let _postgres_url = EnvGuard::set(
            "IRONCLAW_REBORN_POSTGRES_URL",
            "postgres://event_user:RAW_PASSWORD_SENTINEL_3162@db.example.com/events?sslmode=require",
        );
        let _secret_master_key = EnvGuard::clear("IRONCLAW_REBORN_SECRET_MASTER_KEY");

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        std::fs::write(
            reborn_home.join("config.toml"),
            r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"

[policy]
deployment_mode = "hosted_multi_tenant"
default_profile = "secure_default"
"#,
        )
        .expect("write config");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.into_os_string()),
            None,
            None,
            Some("production".into()),
        )
        .expect("boot config");

        let err = build_runtime_input(&config, RuntimeInputCaller::Run)
            .err()
            .expect("missing secret master key env must fail closed");
        let rendered = format!("{err:#}");

        assert!(
            rendered.contains("IRONCLAW_REBORN_SECRET_MASTER_KEY"),
            "error must mention missing secret master key env var, got: {rendered}"
        );
        assert!(!rendered.contains("RAW_PASSWORD_SENTINEL_3162"));
        assert!(!rendered.contains("postgres://"));
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_rejects_remote_postgres_sslmode_disable_redacted() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let (_database_sslmode, _allow_cleartext) = clear_reborn_postgres_tls_env();
        let _postgres_url = EnvGuard::set(
            "IRONCLAW_REBORN_POSTGRES_URL",
            "postgres://event_user:RAW_PASSWORD_SENTINEL_3162@db.example.com/events?sslmode=disable",
        );
        let _secret_master_key =
            EnvGuard::set("IRONCLAW_REBORN_SECRET_MASTER_KEY", "test-master-key");

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        std::fs::write(
            reborn_home.join("config.toml"),
            r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"

[policy]
deployment_mode = "hosted_multi_tenant"
default_profile = "secure_default"
"#,
        )
        .expect("write config");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.into_os_string()),
            None,
            None,
            Some("production".into()),
        )
        .expect("boot config");

        let err = build_runtime_input(&config, RuntimeInputCaller::Run)
            .err()
            .expect("sslmode=disable must fail closed before connecting");
        let rendered = format!("{err:#}");

        assert!(
            rendered.contains("sslmode=require") && rendered.contains("sslmode=disable"),
            "error should explain TLS requirement, got: {rendered}"
        );
        assert!(!rendered.contains("RAW_PASSWORD_SENTINEL_3162"));
        assert!(!rendered.contains("postgres://"));
        assert!(!rendered.contains("db.example.com"));
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_rejects_database_sslmode_disable_without_opt_in() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let _database_sslmode = EnvGuard::set("DATABASE_SSLMODE", "Disable");
        let _allow_cleartext = EnvGuard::clear("IRONCLAW_REBORN_ALLOW_REMOTE_POSTGRES_CLEAR_TEXT");
        let _postgres_url = EnvGuard::set(
            "IRONCLAW_REBORN_POSTGRES_URL",
            "postgres://event_user:RAW_PASSWORD_SENTINEL_3162@db.example.com/events?sslmode=require",
        );
        let _secret_master_key =
            EnvGuard::set("IRONCLAW_REBORN_SECRET_MASTER_KEY", "test-master-key");
        let (_temp, config) = boot_config_with_config_toml(
            "production",
            r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"

[policy]
deployment_mode = "hosted_multi_tenant"
default_profile = "secure_default"
"#,
        );

        let err = build_runtime_input(&config, RuntimeInputCaller::Run)
            .err()
            .expect("DATABASE_SSLMODE=disable must fail without the Reborn opt-in");
        let rendered = format!("{err:#}");

        assert!(
            rendered.contains("sslmode=disable"),
            "error should mention rejected sslmode, got: {rendered}"
        );
        assert!(!rendered.contains("RAW_PASSWORD_SENTINEL_3162"));
        assert!(!rendered.contains("postgres://"));
        assert!(!rendered.contains("db.example.com"));
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_allows_database_sslmode_disable_with_opt_in() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let _database_sslmode = EnvGuard::set("DATABASE_SSLMODE", "DISABLE");
        let _allow_cleartext =
            EnvGuard::set("IRONCLAW_REBORN_ALLOW_REMOTE_POSTGRES_CLEAR_TEXT", "On");
        let _postgres_url = EnvGuard::set(
            "IRONCLAW_REBORN_POSTGRES_URL",
            "postgres://event_user:RAW_PASSWORD_SENTINEL_3162@db.example.com/events?sslmode=require",
        );
        let _secret_master_key =
            EnvGuard::set("IRONCLAW_REBORN_SECRET_MASTER_KEY", "test-master-key");
        let (_temp, config) = boot_config_with_config_toml(
            "production",
            r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"

[policy]
deployment_mode = "hosted_multi_tenant"
default_profile = "secure_default"
"#,
        );

        let runtime_input =
            build_runtime_input(&config, RuntimeInputCaller::Run).expect("runtime input");
        let services = runtime_input.services.expect("services input");
        assert_eq!(services.profile(), RebornCompositionProfile::Production);
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_rejects_invalid_cleartext_opt_in() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let _database_sslmode = EnvGuard::set("DATABASE_SSLMODE", "disable");
        let _allow_cleartext = EnvGuard::set(
            "IRONCLAW_REBORN_ALLOW_REMOTE_POSTGRES_CLEAR_TEXT",
            "enabled",
        );
        let _postgres_url = EnvGuard::set(
            "IRONCLAW_REBORN_POSTGRES_URL",
            "postgres://event_user:RAW_PASSWORD_SENTINEL_3162@db.example.com/events?sslmode=require",
        );
        let _secret_master_key =
            EnvGuard::set("IRONCLAW_REBORN_SECRET_MASTER_KEY", "test-master-key");
        let (_temp, config) = boot_config_with_config_toml(
            "production",
            r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"

[policy]
deployment_mode = "hosted_multi_tenant"
default_profile = "secure_default"
"#,
        );

        let err = build_runtime_input(&config, RuntimeInputCaller::Run)
            .err()
            .expect("invalid cleartext opt-in must fail loudly");
        let rendered = format!("{err:#}");

        assert!(rendered.contains("IRONCLAW_REBORN_ALLOW_REMOTE_POSTGRES_CLEAR_TEXT"));
        assert!(rendered.contains("true"));
        assert!(rendered.contains("false"));
        assert!(!rendered.contains("RAW_PASSWORD_SENTINEL_3162"));
        assert!(!rendered.contains("postgres://"));
        assert!(!rendered.contains("db.example.com"));
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_accepts_verify_full_database_sslmode() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let _database_sslmode = EnvGuard::set("DATABASE_SSLMODE", "verify-full");
        let _allow_cleartext = EnvGuard::clear("IRONCLAW_REBORN_ALLOW_REMOTE_POSTGRES_CLEAR_TEXT");
        let _postgres_url = EnvGuard::set(
            "IRONCLAW_REBORN_POSTGRES_URL",
            "postgres://event_user:RAW_PASSWORD_SENTINEL_3162@db.example.com/events?sslmode=require",
        );
        let _secret_master_key =
            EnvGuard::set("IRONCLAW_REBORN_SECRET_MASTER_KEY", "test-master-key");
        let (_temp, config) = boot_config_with_config_toml(
            "production",
            r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"

[policy]
deployment_mode = "hosted_multi_tenant"
default_profile = "secure_default"
"#,
        );

        let runtime_input =
            build_runtime_input(&config, RuntimeInputCaller::Run).expect("runtime input");
        let services = runtime_input.services.expect("services input");
        assert_eq!(services.profile(), RebornCompositionProfile::Production);
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_constructs_postgres_services_input() {
        let lock = lock_trigger_env();
        let (enabled, interval) = clear_trigger_poller_env();
        let (database_sslmode, allow_cleartext) = clear_reborn_postgres_tls_env();
        let postgres_url = EnvGuard::set(
            "IRONCLAW_REBORN_POSTGRES_URL",
            "postgres://event_user:RAW_PASSWORD_SENTINEL_3162@db.example.com/events?sslmode=require",
        );
        let secret_master_key =
            EnvGuard::set("IRONCLAW_REBORN_SECRET_MASTER_KEY", "test-master-key");

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        std::fs::write(
            reborn_home.join("config.toml"),
            r#"
[identity]
default_owner = "prod-owner"

[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"

[policy]
deployment_mode = "hosted_multi_tenant"
default_profile = "secure_default"
"#,
        )
        .expect("write config");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.into_os_string()),
            None,
            None,
            Some("production".into()),
        )
        .expect("boot config");

        let runtime_input =
            build_runtime_input(&config, RuntimeInputCaller::Run).expect("runtime input");
        let services = runtime_input.services.expect("services input");

        assert_eq!(services.profile(), RebornCompositionProfile::Production);
        assert_eq!(services.owner_id(), "prod-owner");
        let runtime_policy = services
            .runtime_policy()
            .expect("production CLI input wires runtime policy");
        assert_eq!(runtime_policy.deployment.as_str(), "hosted_multi_tenant");
        assert_eq!(runtime_policy.resolved_profile.as_str(), "secure_default");

        drop(postgres_url);
        drop(secret_master_key);
        drop(interval);
        drop(enabled);
        drop(allow_cleartext);
        drop(database_sslmode);
        drop(lock);
    }

    // Regression for the review point that `serve` rejected legitimate
    // `[identity].default_project` configs at runtime-input build time
    // because the unsupported-section check was shared with `run` / `repl`.
    // `serve` consumes the value, `run` does not — the discriminator
    // ensures both branches do the right thing.
    #[test]
    fn build_runtime_input_for_run_rejects_default_project() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        std::fs::write(
            reborn_home.join("config.toml"),
            r#"
[identity]
default_project = "project-alpha"
"#,
        )
        .expect("write config");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.into_os_string()),
            None,
            None,
            None,
        )
        .expect("boot config");

        let err = build_runtime_input(&config, RuntimeInputCaller::Run)
            .err()
            .expect("run must reject default_project");
        assert!(
            err.to_string().contains("default_project"),
            "error must mention the rejected field, got: {err}",
        );
    }

    #[test]
    fn build_runtime_input_for_run_rejects_default_project_when_trigger_poller_enabled() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        std::fs::write(
            reborn_home.join("config.toml"),
            r#"
[identity]
default_project = "project-alpha"

[trigger_poller]
enabled = true
"#,
        )
        .expect("write config");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.into_os_string()),
            None,
            None,
            None,
        )
        .expect("boot config");

        let err = build_runtime_input(&config, RuntimeInputCaller::Run)
            .err()
            .expect("run must reject default_project even when trigger poller is enabled");
        assert!(
            err.to_string().contains("default_project"),
            "error must mention the rejected field, got: {err}",
        );
    }

    #[cfg(feature = "webui-v2-beta")]
    #[allow(clippy::await_holding_lock, reason = "serializes env guards")]
    #[tokio::test]
    async fn run_trigger_poller_bootstrap_seeds_local_access_checker() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        std::fs::write(
            reborn_home.join("config.toml"),
            r#"
[identity]
tenant = "run-trigger-tenant"
default_owner = "run-trigger-user"
default_agent = "run-trigger-agent"

[trigger_poller]
enabled = true
"#,
        )
        .expect("write config");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.into_os_string()),
            None,
            None,
            None,
        )
        .expect("boot config");
        let runtime_input =
            build_runtime_input(&config, RuntimeInputCaller::Run).expect("runtime input");

        let tenant_id = ironclaw_reborn_composition::host_api::TenantId::new("run-trigger-tenant")
            .expect("tenant id");
        let user_id = ironclaw_reborn_composition::host_api::UserId::new("run-trigger-user")
            .expect("user id");
        let stale_user_id = ironclaw_reborn_composition::host_api::UserId::new("run-trigger-stale")
            .expect("stale user id");
        let agent_id = ironclaw_reborn_composition::host_api::AgentId::new("run-trigger-agent")
            .expect("agent id");
        let project_id =
            ironclaw_reborn_composition::host_api::ProjectId::new("run-trigger-project")
                .expect("project id");
        let user_store_path = config
            .home()
            .path()
            .join("local-dev")
            .join("reborn-local-dev.db");
        let access_store =
            ironclaw_reborn_composition::open_local_trigger_access_store(&user_store_path)
                .await
                .expect("open local trigger access store");
        access_store
            .seed_local_access(ironclaw_reborn_composition::LocalTriggerAccessSeed {
                tenant_id: &tenant_id,
                user_id: &stale_user_id,
                agent_id: Some(&agent_id),
                project_id: None,
                role: LocalTriggerAccessRole::Owner,
                source: LocalTriggerAccessSource::LocalDevRunBootstrap,
            })
            .await
            .expect("seed stale run trigger access");

        let runtime_input = with_run_local_trigger_fire_access_checker(runtime_input, &config)
            .await
            .expect("bootstrap run trigger fire access checker");

        let checker = runtime_input
            .trigger_fire_access_checker
            .expect("checker is wired");
        let allowed = checker
            .check_trigger_fire_access(ironclaw_reborn_composition::TriggerFireAccessCheck {
                tenant_id: tenant_id.clone(),
                creator_user_id: user_id,
                agent_id: Some(agent_id.clone()),
                project_id: None,
                trigger_id: ironclaw_reborn_composition::TriggerId::new(),
                fire_slot: chrono::Utc::now(),
            })
            .await
            .expect("check run trigger fire access");
        assert_eq!(
            allowed,
            ironclaw_reborn_composition::TriggerFireAccessDecision::Allowed
        );

        let project_scoped_decision = checker
            .check_trigger_fire_access(ironclaw_reborn_composition::TriggerFireAccessCheck {
                tenant_id: tenant_id.clone(),
                creator_user_id: ironclaw_reborn_composition::host_api::UserId::new(
                    "run-trigger-user",
                )
                .expect("user id"),
                agent_id: Some(agent_id.clone()),
                project_id: Some(project_id.clone()),
                trigger_id: ironclaw_reborn_composition::TriggerId::new(),
                fire_slot: chrono::Utc::now(),
            })
            .await
            .expect("check project-scoped run trigger fire access");
        assert_eq!(
            project_scoped_decision,
            ironclaw_reborn_composition::TriggerFireAccessDecision::Denied {
                reason: "trigger creator does not have active local access for this scope"
                    .to_string(),
            }
        );

        let stale_decision = checker
            .check_trigger_fire_access(ironclaw_reborn_composition::TriggerFireAccessCheck {
                tenant_id,
                creator_user_id: stale_user_id,
                agent_id: Some(agent_id),
                project_id: None,
                trigger_id: ironclaw_reborn_composition::TriggerId::new(),
                fire_slot: chrono::Utc::now(),
            })
            .await
            .expect("check stale run trigger fire access");
        assert_eq!(
            stale_decision,
            ironclaw_reborn_composition::TriggerFireAccessDecision::Denied {
                reason: "trigger creator does not have active local access for this scope"
                    .to_string(),
            }
        );
    }

    #[test]
    fn build_runtime_input_for_serve_accepts_default_project() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        std::fs::write(
            reborn_home.join("config.toml"),
            r#"
[identity]
default_project = "project-alpha"
"#,
        )
        .expect("write config");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.into_os_string()),
            None,
            None,
            None,
        )
        .expect("boot config");

        let _runtime_input = build_runtime_input(&config, RuntimeInputCaller::Serve)
            .expect("serve must accept default_project");
    }

    #[test]
    fn build_runtime_input_maps_trigger_poller_enabled_config() {
        let _lock = lock_trigger_env();
        let (_enabled, _interval) = clear_trigger_poller_env();

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        std::fs::write(
            reborn_home.join("config.toml"),
            r#"
[trigger_poller]
enabled = true
poll_interval_secs = 42
"#,
        )
        .expect("write config");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.into_os_string()),
            None,
            None,
            None,
        )
        .expect("boot config");

        let input = build_runtime_input(&config, RuntimeInputCaller::Run).expect("runtime input");

        assert!(
            input.trigger_poller.enabled,
            "[trigger_poller] enabled=true in config must reach runtime_input.trigger_poller.enabled"
        );
        assert_eq!(
            input.trigger_poller.worker.poll_interval,
            std::time::Duration::from_secs(42),
            "config poll_interval_secs must reach worker.poll_interval"
        );
    }

    #[test]
    fn build_runtime_input_env_enables_trigger_poller_with_no_config_section() {
        // No [trigger_poller] in config; env var enables → input.trigger_poller.enabled must be true.
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::set("IRONCLAW_TRIGGER_POLLER_ENABLED", "true");
        let _interval = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS");

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        // No config.toml written → no [trigger_poller] section at all.

        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.to_string_lossy().to_string().into()),
            None,
            None,
            None,
        )
        .expect("boot config");

        let input = build_runtime_input(&config, RuntimeInputCaller::Run).expect("runtime input");

        assert!(
            input.trigger_poller.enabled,
            "IRONCLAW_TRIGGER_POLLER_ENABLED=true must reach input.trigger_poller.enabled through build_runtime_input"
        );
    }

    #[test]
    fn build_runtime_input_env_interval_overrides_config_interval() {
        // Config says interval=15s, env says interval=45s → env must win at the caller boundary.
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_ENABLED");
        let _interval = EnvGuard::set("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS", "45");

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        std::fs::write(
            reborn_home.join("config.toml"),
            r#"
[trigger_poller]
enabled = true
poll_interval_secs = 15
"#,
        )
        .expect("write config");

        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.to_string_lossy().to_string().into()),
            None,
            None,
            None,
        )
        .expect("boot config");

        let input = build_runtime_input(&config, RuntimeInputCaller::Run).expect("runtime input");

        assert_eq!(
            input.trigger_poller.worker.poll_interval,
            std::time::Duration::from_secs(45),
            "env IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS=45 must override config poll_interval_secs=15 through build_runtime_input"
        );
    }

    #[test]
    fn build_runtime_input_rejects_invalid_trigger_poller_enabled_env() {
        // Invalid env value (`yes`) must error out through build_runtime_input,
        // not slip through to the runtime input. Closes the caller-level gap
        // for the error path; previous tests covered only happy/override paths.
        let _lock = lock_trigger_env();
        let _enabled = EnvGuard::set("IRONCLAW_TRIGGER_POLLER_ENABLED", "yes");
        let _interval = EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS");

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");

        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.to_string_lossy().to_string().into()),
            None,
            None,
            None,
        )
        .expect("boot config");

        let err = match build_runtime_input(&config, RuntimeInputCaller::Run) {
            Ok(_) => panic!(
                "invalid IRONCLAW_TRIGGER_POLLER_ENABLED must propagate as Err through build_runtime_input"
            ),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("IRONCLAW_TRIGGER_POLLER_ENABLED"),
            "caller-level error must surface the env var name, got: {err}",
        );
    }

    #[test]
    fn resolve_google_oauth_config_returns_none_when_no_vars_set() {
        let config =
            resolve_google_oauth_config(|_| None).expect("empty env should not fail setup");

        assert!(config.is_none());
    }

    #[test]
    fn resolve_google_oauth_config_errors_when_client_id_missing() {
        let vars = HashMap::from([(
            "IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI",
            "http://127.0.0.1:3000/api/reborn/product-auth/oauth/google/callback",
        )]);

        let error =
            resolve_google_oauth_config(|name| vars.get(name).map(|value| value.to_string()))
                .expect_err("redirect-only Google OAuth config must fail closed");

        assert!(error.to_string().contains("GOOGLE_CLIENT_ID"));
    }

    #[test]
    fn resolve_google_oauth_config_prefers_reborn_prefixed_vars() {
        let vars = HashMap::from([
            (
                "IRONCLAW_REBORN_GOOGLE_CLIENT_ID",
                "reborn-client.apps.googleusercontent.com",
            ),
            (
                "IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET",
                "reborn-client-secret",
            ),
            (
                "IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI",
                "http://127.0.0.1:3000/api/reborn/product-auth/oauth/google/callback",
            ),
            (
                "IRONCLAW_REBORN_GOOGLE_HOSTED_DOMAIN_HINT",
                "reborn.example.com",
            ),
            (
                "GOOGLE_CLIENT_ID",
                "legacy-client.apps.googleusercontent.com",
            ),
            ("GOOGLE_CLIENT_SECRET", "legacy-client-secret"),
            (
                "GOOGLE_OAUTH_REDIRECT_URI",
                "http://127.0.0.1:3000/legacy/callback",
            ),
            ("GOOGLE_ALLOWED_HD", "legacy.example.com"),
        ]);

        let config =
            resolve_google_oauth_config(|name| vars.get(name).map(|value| value.to_string()))
                .expect("Google OAuth config")
                .expect("configured Google OAuth");

        assert_eq!(
            config.client.client_id.as_str(),
            "reborn-client.apps.googleusercontent.com"
        );
        assert_eq!(
            config.client.redirect_uri.as_str(),
            "http://127.0.0.1:3000/api/reborn/product-auth/oauth/google/callback"
        );
        assert!(config.client.client_secret.is_some());
        assert_eq!(
            config.hosted_domain_hint.as_deref(),
            Some("reborn.example.com")
        );
    }

    #[test]
    fn resolve_google_oauth_config_uses_legacy_client_vars_as_configuration_signal() {
        let vars = HashMap::from([
            (
                "GOOGLE_CLIENT_ID",
                "legacy-client.apps.googleusercontent.com",
            ),
            ("GOOGLE_CLIENT_SECRET", "legacy-client-secret"),
        ]);

        let error =
            resolve_google_oauth_config(|name| vars.get(name).map(|value| value.to_string()))
                .expect_err("legacy client vars without redirect URI must not be ignored");

        assert!(error.to_string().contains("GOOGLE_OAUTH_REDIRECT_URI"));
    }
}
