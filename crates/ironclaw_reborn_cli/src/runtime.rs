use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::time::Duration;
use std::{future::Future, thread};

use anyhow::Context;
use ironclaw_reborn_composition::{
    OAuthClientConfig, PollSettings, RebornBuildInput, RebornCompositionProfile,
    RebornLocalRuntimeProfileOptions, RebornRuntimeIdentity, RebornRuntimeInput,
    TriggerPollerSettings, TurnRunnerSettings, build_reborn_runtime,
    local_runtime_build_input_with_options,
};
use ironclaw_reborn_config::{REBORN_PROFILE_ENV, RebornBootConfig, RebornProfile};
use secrecy::SecretString;
use tokio_util::sync::CancellationToken;

use crate::context::RebornCliContext;

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

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let runtime = build_reborn_runtime(runtime_input).await?;
        print_runtime_banner(context.boot_config());

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

    reject_unsupported_runtime_sections(config_file.as_ref(), caller)?;

    let owner_id = default_owner_id(config_file.as_ref());

    let local_dev_root: PathBuf = config.home().path().join("local-dev");

    let workspace_root = std::env::current_dir()
        .context("failed to resolve current directory for local-dev workspace")?;
    let profile = effective_profile(config, config_file.as_ref())?;
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
                     got profile={profile}. Production wiring lands in a follow-up slice."
        )
    })?
    .with_local_dev_workspace_root(workspace_root);
    if services_input.requires_local_dev_confirmed_host_home_root() {
        let host_home_root =
            confirmed_host_home_root(options).context("local-dev-yolo host access")?;
        services_input = services_input.with_local_dev_confirmed_host_home_root(host_home_root);
    }
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
) -> anyhow::Result<()> {
    let Some(file) = config_file else {
        return Ok(());
    };

    // `[identity].default_project` is parsed but not yet wired into
    // the generic runtime slice — `run` / `repl` would silently drop
    // the value, so we fail-loud. The `serve` subcommand DOES consume
    // it (stamped onto every `WebUiAuthenticatedCaller`), so for that
    // caller the field is supported, not "parsed but not wired".
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
    if file.policy.is_some() {
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
/// 3. Compiled default — `TriggerPollerSettings::default()` (disabled, all limits
///    at the `ironclaw_triggers` crate defaults).
///
/// V1 invariant: `max_concurrent_fires_per_trigger` must be exactly 1. Passing
/// any other value (via config or, were an env override ever added, env) returns
/// an error rather than silently breaking per-trigger serialisation.
fn trigger_poller_settings(
    config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
) -> anyhow::Result<TriggerPollerSettings> {
    // Layer 3: compiled default (disabled).
    let mut settings = TriggerPollerSettings::default();

    // Layer 2: config-file [trigger_poller] section.
    if let Some(section) = config_file.and_then(|file| file.trigger_poller.as_ref()) {
        if let Some(enabled) = section.enabled {
            settings.enabled = enabled;
        }

        // Build a mutable worker config to apply section overrides.
        let mut worker = settings.worker;

        if let Some(secs) = section.poll_interval_secs {
            if secs == 0 {
                anyhow::bail!(
                    "config file [trigger_poller].poll_interval_secs must be greater than 0"
                );
            }
            worker.poll_interval = Duration::from_secs(secs);
        }

        if let Some(fires) = section.fires_per_tick {
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
            settings.startup_jitter_max = Duration::from_secs(jitter_secs);
        }

        if let Some(jitter_secs) = section.tick_jitter_max_secs {
            settings.tick_jitter_max = Duration::from_secs(jitter_secs);
        }

        settings.worker = worker;
    }

    // Layer 1: environment variable overrides.
    if let Some(raw) = optional_nonempty_env("IRONCLAW_TRIGGER_POLLER_ENABLED") {
        match raw.to_ascii_lowercase().as_str() {
            "1" | "true" => settings.enabled = true,
            "0" | "false" => settings.enabled = false,
            other => anyhow::bail!(
                "IRONCLAW_TRIGGER_POLLER_ENABLED must be one of 1, true, 0, false (got {other:?})"
            ),
        }
    }

    if let Some(raw) = optional_nonempty_env("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS") {
        let secs: u64 = raw.parse().map_err(|_| {
            anyhow::anyhow!(
                "IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS must be a positive integer, got {raw:?}"
            )
        })?;
        if secs == 0 {
            anyhow::bail!("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS must be greater than 0");
        }
        settings.worker.poll_interval = Duration::from_secs(secs);
    }

    Ok(settings)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ironclaw_reborn_composition::RebornCompositionProfile;
    use ironclaw_reborn_config::RebornBootConfig;

    use super::{
        RuntimeInputCaller, RuntimeInputOptions, block_on_cli, build_runtime_input,
        build_runtime_input_with_options, resolve_google_oauth_config,
    };

    #[tokio::test]
    async fn block_on_cli_can_run_inside_existing_tokio_runtime() {
        let value = block_on_cli(async { Ok::<_, anyhow::Error>(42) }).expect("block future");

        assert_eq!(value, 42);
    }

    #[test]
    fn build_runtime_input_maps_configured_cli_identity() {
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

    // Regression for the review point that `serve` rejected legitimate
    // `[identity].default_project` configs at runtime-input build time
    // because the unsupported-section check was shared with `run` / `repl`.
    // `serve` consumes the value, `run` does not — the discriminator
    // ensures both branches do the right thing.
    #[test]
    fn build_runtime_input_for_run_rejects_default_project() {
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
    fn build_runtime_input_for_serve_accepts_default_project() {
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

    // --- trigger_poller_settings tests ---

    use super::trigger_poller_settings;
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
        // No config file, no env → disabled with zero jitter.
        let settings = trigger_poller_settings(None).expect("default trigger poller settings");

        assert!(!settings.enabled, "default must be disabled");
        assert_eq!(settings.startup_jitter_max, Duration::ZERO);
        assert_eq!(settings.tick_jitter_max, Duration::ZERO);
    }

    #[test]
    fn trigger_poller_settings_config_enabled_maps_worker_fields() {
        let section = TriggerPollerConfigSection {
            enabled: Some(true),
            poll_interval_secs: Some(15),
            fires_per_tick: Some(50),
            max_concurrent_fires_per_trigger: Some(1),
            startup_jitter_max_secs: Some(3),
            tick_jitter_max_secs: Some(7),
        };
        let config = make_config_with_trigger_poller(section);

        let settings =
            trigger_poller_settings(Some(&config)).expect("trigger poller settings from config");

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

        let err = trigger_poller_settings(Some(&config))
            .expect_err("max_concurrent_fires_per_trigger=2 must be rejected");

        assert!(
            err.to_string().contains("max_concurrent_fires_per_trigger"),
            "error must mention the field, got: {err}",
        );
    }

    // NOTE: tests that mutate real process env vars are inherently racy when
    // run in parallel with other tests in the same process. We follow the
    // pattern used elsewhere in this crate: set the variable, run the
    // assertion, then always restore the original value in a defer-style
    // guard so a panicking assertion cannot leave the env dirty.
    #[test]
    fn trigger_poller_settings_env_enabled_overrides_config_disabled() {
        // Config says disabled; env says enabled — env must win.
        let section = TriggerPollerConfigSection {
            enabled: Some(false),
            ..Default::default()
        };
        let config = make_config_with_trigger_poller(section);

        let key = "IRONCLAW_TRIGGER_POLLER_ENABLED";
        let prior = std::env::var(key).ok();
        // Safety: single-threaded test binary segment; restored below.
        unsafe { std::env::set_var(key, "true") };
        let result = trigger_poller_settings(Some(&config));
        match prior {
            Some(v) => unsafe { std::env::set_var(key, v) },
            None => unsafe { std::env::remove_var(key) },
        }

        let settings = result.expect("env override should succeed");
        assert!(
            settings.enabled,
            "IRONCLAW_TRIGGER_POLLER_ENABLED=true must override config enabled=false"
        );
    }
}
