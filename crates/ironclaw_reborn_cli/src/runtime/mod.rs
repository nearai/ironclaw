use std::io::{IsTerminal, Write};
#[cfg(feature = "webui-v2-beta")]
use std::path::Path;
use std::path::PathBuf;
#[cfg(feature = "webui-v2-beta")]
use std::sync::Arc;
use std::time::Duration;
use std::{future::Future, thread};

use anyhow::Context;
#[cfg(any(feature = "slack-v2-host-beta", test))]
use ironclaw_reborn_composition::OAuthRedirectUri;
#[cfg(feature = "slack-v2-host-beta")]
use ironclaw_reborn_composition::SlackPersonalSetupServiceSlot;
#[cfg(feature = "webui-v2-beta")]
use ironclaw_reborn_composition::host_api::UserId;
use ironclaw_reborn_composition::host_api::{AgentId, TenantId};
#[cfg(feature = "postgres")]
use ironclaw_reborn_composition::hosted_single_tenant_runtime_policy;
use ironclaw_reborn_composition::{
    CredentialRefreshSettings, OAuthClientConfig, OperatorLogLayer, PollSettings, RebornBuildInput,
    RebornCompositionProfile, RebornRuntimeIdentity, RebornRuntimeInput,
    RebornRuntimeProfileOptions, TurnRunnerSettings, build_reborn_runtime,
    local_runtime_build_input_with_options, nearai_mcp_bootstrap_config_from_env,
};
#[cfg(feature = "webui-v2-beta")]
use ironclaw_reborn_composition::{
    LocalTriggerAccessReconciliation, LocalTriggerAccessRole, LocalTriggerAccessSource,
    LocalTriggerAccessStore, local_trigger_access_fire_checker, open_local_trigger_access_store,
};
use ironclaw_reborn_config::{
    REBORN_PROFILE_ENV, RebornBootConfig, RebornProfile, seed_default_config_file_if_missing,
};
use secrecy::SecretString;
use tokio_util::sync::CancellationToken;

use crate::context::RebornCliContext;

// Crate-wide process-env lock lives here (see test_env.rs). `pub(crate)` so
// non-runtime env-mutating tests (e.g. commands::serve_sso) serialize against
// the same mutex — all unit tests link into one binary, so a second, separate
// env lock would not serialize and races the shared process environment
// (#6015).
#[cfg(test)]
pub(crate) mod test_env;
mod trigger_poller;

use trigger_poller::trigger_poller_settings;

pub(crate) fn init_tracing() {
    use tracing_subscriber::Layer;
    use tracing_subscriber::fmt;
    use tracing_subscriber::prelude::*;
    // stderr/fmt layer: operator-facing console output. Stays at `info` by
    // default so `debug!` diagnostics never reach (and corrupt) a REPL/TUI
    // terminal — the repo's logging invariant. Broad env overrides are still
    // guarded from third-party debug floods unless those targets are explicit.
    let stderr_filter = reborn_env_filter(
        "IRONCLAW_REBORN_LOG",
        "info,ironclaw_runner=info,ironclaw_reborn_composition=info",
    );
    // Operator Logs buffer: captures run diagnostics at `debug` for the
    // ironclaw run-path crates so the scoped (thread/run) Logs panel is
    // populated, while those `debug!` events are NOT written to stderr. This is
    // a *separate* per-layer filter, so terminal safety and Logs-panel
    // visibility are decoupled. Broad env overrides are guarded the same way
    // so the browser log buffer is not filled by low-level protocol crates.
    let operator_filter = reborn_env_filter(
        "IRONCLAW_REBORN_OPERATOR_LOG",
        "info,ironclaw_runner=debug,ironclaw_host_runtime=debug",
    );
    let _ = tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(std::io::stderr)
                .with_filter(stderr_filter),
        )
        .with(OperatorLogLayer.with_filter(operator_filter))
        .try_init();
}

const REBORN_NOISY_LOG_TARGETS: &[(&str, &str)] = &[
    ("tokio_postgres", "warn"),
    ("deadpool_postgres", "warn"),
    ("h2", "warn"),
    ("hyper", "warn"),
    ("hyper_util", "warn"),
    ("reqwest", "warn"),
    ("rustls", "warn"),
    ("tower", "warn"),
    ("tower_http", "warn"),
    ("ironclaw_llm", "info"),
];

fn reborn_env_filter(env_key: &str, default_filter: &str) -> tracing_subscriber::EnvFilter {
    let default_filter = protect_reborn_log_filter(default_filter);
    match std::env::var(env_key) {
        Ok(raw_filter) => {
            tracing_subscriber::EnvFilter::try_new(protect_reborn_log_filter(&raw_filter))
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter))
        }
        Err(_) => tracing_subscriber::EnvFilter::new(default_filter),
    }
}

fn protect_reborn_log_filter(raw_filter: &str) -> String {
    let mut filter = if raw_filter.trim().is_empty() {
        "info".to_string()
    } else {
        raw_filter.trim().to_string()
    };
    for (target, level) in REBORN_NOISY_LOG_TARGETS {
        if !log_filter_mentions_target(&filter, target) {
            filter.push(',');
            filter.push_str(target);
            filter.push('=');
            filter.push_str(level);
        }
    }
    filter
}

fn log_filter_mentions_target(filter: &str, target: &str) -> bool {
    let child_prefix = format!("{target}::");
    filter
        .split(',')
        .filter_map(log_directive_target)
        .any(|directive| directive == target || directive.starts_with(&child_prefix))
}

fn log_directive_target(directive: &str) -> Option<&str> {
    let directive = directive.trim();
    if directive.is_empty() {
        return None;
    }
    let target = match directive.split_once('=') {
        Some((target, _level)) => target.trim(),
        None if is_log_level_directive(directive) => return None,
        None => directive,
    };
    let target = target.split_once('[').map_or(target, |(target, _)| target);
    if target.is_empty() || is_log_level_directive(target) {
        None
    } else {
        Some(target)
    }
}

fn is_log_level_directive(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "off" | "error" | "warn" | "info" | "debug" | "trace"
    )
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
        build_runtime_input_with_options(context.boot_config(), RuntimeInputCaller::Run, options)?
            .inner;
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
        let profile = effective_profile(config, config_file.as_ref())?;
        let user_store_path =
            local_runtime_storage_root(config, profile).join("reborn-local-dev.db");
        let access_store =
            open_trigger_access_store_for_profile(&runtime_input, profile, &user_store_path)
                .await?;
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

        Ok(runtime_input
            .with_trigger_fire_access_checker(local_trigger_access_fire_checker(access_store)))
    }
}

#[cfg(feature = "webui-v2-beta")]
pub(crate) async fn open_trigger_access_store_for_profile(
    runtime_input: &RebornRuntimeInput,
    profile: RebornProfile,
    local_store_path: &Path,
) -> anyhow::Result<Arc<dyn LocalTriggerAccessStore>> {
    match profile {
        RebornProfile::HostedSingleTenant => {
            #[cfg(feature = "postgres")]
            {
                let services = runtime_input.services.as_ref().context(
                    "profile=hosted-single-tenant requires runtime services before trigger-fire access can be wired",
                )?;
                let store = services
                    .open_hosted_single_tenant_trigger_access_store()
                    .await
                    .context("failed to initialize hosted trigger-fire access store")?;
                let store: Arc<dyn LocalTriggerAccessStore> = store;
                Ok(store)
            }
            #[cfg(not(feature = "postgres"))]
            {
                let _ = runtime_input;
                let _ = local_store_path;
                anyhow::bail!(
                    "profile=hosted-single-tenant requires the `postgres` feature for trigger-fire access"
                );
            }
        }
        _ => {
            let store = open_local_trigger_access_store(local_store_path)
                .await
                .context("failed to initialize local trigger-fire access store")?;
            let store: Arc<dyn LocalTriggerAccessStore> = store;
            Ok(store)
        }
    }
}

fn print_runtime_banner(config: &RebornBootConfig) {
    eprintln!("ironclaw: runtime started");
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
            "reborn run did not produce an assistant reply\n{}",
            no_assistant_text_message(&reply)
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
                                    "reborn run did not produce an assistant reply\n{}",
                                    no_assistant_text_message(&reply)
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
        None => eprintln!("{}", no_assistant_text_message(reply)),
    }
}

fn no_assistant_text_message(reply: &ironclaw_reborn_composition::AssistantReply) -> String {
    let summary = reply_without_text_summary(reply);
    let failure_category = reply
        .failure_category
        .as_deref()
        .map(|category| format!("\nfailure_category={category}"))
        .unwrap_or_default();
    format!(
        "{summary}{failure_category}\nstatus={:?}; run_id={}",
        reply.status, reply.run_id
    )
}

fn reply_without_text_summary(reply: &ironclaw_reborn_composition::AssistantReply) -> &'static str {
    match reply.status {
        ironclaw_reborn_composition::TurnStatus::Failed
        | ironclaw_reborn_composition::TurnStatus::RecoveryRequired => {
            ironclaw_reborn_composition::reborn_failure_summary_for_category(
                reply.failure_category.as_deref(),
            )
        }
        ironclaw_reborn_composition::TurnStatus::Cancelled => {
            "The run was cancelled before producing a reply."
        }
        ironclaw_reborn_composition::TurnStatus::Completed => {
            "The run completed without producing an assistant reply."
        }
        _ => "The run has not produced an assistant reply yet.",
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
        .map(|b| b.inner)
}

/// Build [`CredentialRefreshSettings`] for the proactive Google OAuth keepalive
/// worker.
///
/// Enabled by default on the local `serve` surface (so refresh tokens stay warm
/// for long-running deployments) and disabled for every other caller, mirroring
/// the trigger-poller default. `IRONCLAW_CREDENTIAL_REFRESH_ENABLED` is an
/// operator override: `1`/`true` forces it on, `0`/`false` is a kill-switch; a
/// present-but-blank value falls through to the caller default.
fn credential_refresh_settings(
    caller: RuntimeInputCaller,
) -> anyhow::Result<CredentialRefreshSettings> {
    let base = if caller == RuntimeInputCaller::Serve {
        CredentialRefreshSettings::enabled()
    } else {
        CredentialRefreshSettings::default()
    };
    apply_credential_refresh_override(base, std::env::var("IRONCLAW_CREDENTIAL_REFRESH_ENABLED"))
}

/// Apply the `IRONCLAW_CREDENTIAL_REFRESH_ENABLED` operator override to a base
/// settings value. Pure (env lookup is passed in) so the override semantics are
/// unit-testable without mutating process-global environment state.
fn apply_credential_refresh_override(
    mut settings: CredentialRefreshSettings,
    raw: Result<String, std::env::VarError>,
) -> anyhow::Result<CredentialRefreshSettings> {
    match raw {
        Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "" => {}
            "1" | "true" => settings.enabled = true,
            "0" | "false" => settings.enabled = false,
            _ => anyhow::bail!(
                "IRONCLAW_CREDENTIAL_REFRESH_ENABLED must be one of 1, true, 0, false"
            ),
        },
        Err(std::env::VarError::NotPresent) => {}
        Err(std::env::VarError::NotUnicode(_)) => {
            anyhow::bail!("IRONCLAW_CREDENTIAL_REFRESH_ENABLED contains non-UTF-8 bytes")
        }
    }

    Ok(settings)
}

/// Resolve the Reborn runtime's default LLM selection, tolerating a
/// required-but-unset API key env var when a key is already durably stored
/// for that provider in the local secret store.
///
/// - Without this, a key provisioned via `onboard` into the encrypted
///   secret store (never written to config.toml/env) would boot `serve`
///   into a fail-closed error, since `apply_startup_stored_llm_key` only
///   runs later once the async runtime is up. Providers with
///   `api_key_required = false` (e.g. `nearai`) never hit this path.
/// - Only `ApiKeyEnvUnset` is treated specially; every other resolution
///   failure surfaces unchanged, as does `ApiKeyEnvUnset` itself when no
///   key is stored — this can only turn a real fix into a successful boot,
///   never mask a misconfiguration.
/// - Scoped to `RuntimeInputCaller::Serve` only: opening the secret store
///   may fall through to the OS keychain (GUI prompt or indefinite block
///   with no GUI session). `onboard` already pays that cost interactively;
///   `serve` is the boot path this fix unblocks. `run` stays fail-fast so
///   a forgotten env var doesn't hang instead of erroring clearly.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn resolve_reborn_runtime_llm_with_stored_key_fallback(
    config: &RebornBootConfig,
    config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
    caller: RuntimeInputCaller,
) -> anyhow::Result<Option<ironclaw_reborn_composition::ResolvedRebornLlm>> {
    let error = match ironclaw_reborn_composition::resolve_reborn_runtime_llm(config, config_file) {
        Ok(resolved) => return Ok(resolved),
        Err(error) => error,
    };
    if caller != RuntimeInputCaller::Serve {
        return Err(error.into());
    }
    let ironclaw_reborn_composition::RebornLlmCatalogError::ApiKeyEnvUnset { ref provider, .. } =
        error
    else {
        return Err(error.into());
    };
    // `ApiKeyEnvUnset` only comes from the config-file-selection branch, so
    // a selection should be present; defensive fallback to original error.
    let Some(selection) = config_file.and_then(|file| file.default_llm_slot()) else {
        return Err(error.into());
    };
    let provider_id = provider.clone();
    let runtime_storage_root = local_runtime_storage_root(config, config.profile());
    // The runtime storage root is only created lazily (onboarding writing a
    // key, or a prior `serve` boot). If it was never created there is
    // definitely no stored key — fail through to the original error instead
    // of letting the secret-store opener fail on a missing directory.
    if !runtime_storage_root.exists() {
        return Err(error.into());
    }
    let has_stored_key = block_on_cli(async move {
        let store = ironclaw_reborn_composition::open_local_dev_secret_store(&runtime_storage_root)
            .await
            .map_err(anyhow::Error::from)?;
        ironclaw_reborn_composition::LlmKeyStore::new(store)
            .exists(&provider_id)
            .await
            .map_err(anyhow::Error::from)
    })?;
    if !has_stored_key {
        return Err(error.into());
    }
    ironclaw_reborn_composition::resolve_llm_selection_allow_missing_key(
        selection,
        Some(config.home().providers_file_path().as_path()),
    )
    .map(ironclaw_reborn_composition::ResolvedRebornLlm::from_llm_config)
    .map(Some)
    .map_err(Into::into)
}

/// Feature-off fallback: without `libsql` there is no local-dev secret store
/// to check, so behavior here is byte-identical to calling
/// `resolve_reborn_runtime_llm` directly — a required-but-unset API key
/// still fails closed with `ApiKeyEnvUnset`.
#[cfg(all(feature = "root-llm-provider", not(feature = "libsql")))]
fn resolve_reborn_runtime_llm_with_stored_key_fallback(
    config: &RebornBootConfig,
    config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
    _caller: RuntimeInputCaller,
) -> anyhow::Result<Option<ironclaw_reborn_composition::ResolvedRebornLlm>> {
    ironclaw_reborn_composition::resolve_reborn_runtime_llm(config, config_file).map_err(Into::into)
}

pub(crate) fn build_runtime_input_with_options(
    config: &RebornBootConfig,
    caller: RuntimeInputCaller,
    options: RuntimeInputOptions,
) -> anyhow::Result<BuiltRuntimeInput> {
    let runtime_services = build_services_input_with_options(config, caller, options)?;
    #[cfg(feature = "slack-v2-host-beta")]
    let slack_personal_lazy_slot = runtime_services.slack_personal_lazy_slot.clone();

    #[allow(unused_mut)]
    let mut runtime_input = RebornRuntimeInput::from_services(runtime_services.services_input)
        .with_runner_settings(runner_settings(runtime_services.config_file.as_ref())?)
        .with_trigger_poller_settings(trigger_poller_settings(
            runtime_services.config_file.as_ref(),
            caller,
        )?)
        .with_credential_refresh_settings(credential_refresh_settings(caller)?)
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
        // The composition runtime cold-boots with a placeholder gateway and
        // needs the boot config to run its initial provider reload. This is
        // required for both `run` and `serve`; without it `run` resolves the
        // configured provider here but still dispatches through `unconfigured`.
        runtime_input = runtime_input.with_boot_config(config.clone());
        match resolve_reborn_runtime_llm_with_stored_key_fallback(
            config,
            runtime_services.config_file.as_ref(),
            caller,
        )? {
            Some(llm) => {
                tracing::debug!(
                    provider_id = %llm.provider_id(),
                    model = %llm.model(),
                    base_url = %llm.base_url().unwrap_or_default(),
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

    Ok(BuiltRuntimeInput {
        inner: runtime_input,
        #[cfg(feature = "slack-v2-host-beta")]
        slack_personal_lazy_slot,
    })
}

pub(crate) struct RuntimeServicesInput {
    pub(crate) services_input: RebornBuildInput,
    #[cfg(feature = "slack-v2-host-beta")]
    pub(crate) slack_personal_lazy_slot: Option<SlackPersonalSetupServiceSlot>,
    config_file: Option<ironclaw_reborn_config::RebornConfigFile>,
}

pub(crate) struct BuiltRuntimeInput {
    pub(crate) inner: RebornRuntimeInput,
    #[cfg(feature = "slack-v2-host-beta")]
    pub(crate) slack_personal_lazy_slot: Option<SlackPersonalSetupServiceSlot>,
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
    // (operator may not have run `ironclaw config init` yet);
    // sparse fields are OK (each absent field falls back to the
    // CLI-shaped default baked into composition).
    let config_file = read_config_file(config)?;

    let owner_id = default_owner_id(config_file.as_ref());

    let profile = effective_profile(config, config_file.as_ref())?;
    reject_unsupported_runtime_sections(config_file.as_ref(), caller, profile)?;
    let mut services_input = match profile {
        RebornProfile::LocalDev
        | RebornProfile::LocalDevYolo
        | RebornProfile::HostedSingleTenantVolume => {
            build_standalone_local_runtime_services_input(profile, owner_id, config, options)?
        }
        RebornProfile::HostedSingleTenant => build_hosted_single_tenant_services_input(
            profile,
            owner_id,
            config,
            config_file.as_ref(),
        )?,
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
    #[cfg(feature = "slack-v2-host-beta")]
    let slack_personal_lazy_slot =
        if let Some(redirect_uri) = resolve_slack_personal_oauth_redirect_uri_from_env()? {
            let slot = SlackPersonalSetupServiceSlot::new(redirect_uri);
            services_input = services_input.with_slack_personal_oauth_lazy(slot.clone());
            Some(slot)
        } else {
            None
        };
    let identity = runtime_identity(config_file.as_ref());
    let tenant_id = TenantId::new(identity.tenant_id).context("invalid runtime tenant identity")?;
    let agent_id = AgentId::new(identity.agent_id).context("invalid runtime agent identity")?;
    services_input = services_input.with_local_runtime_identity(tenant_id, agent_id);

    Ok(RuntimeServicesInput {
        services_input,
        #[cfg(feature = "slack-v2-host-beta")]
        slack_personal_lazy_slot,
        config_file,
    })
}

fn build_standalone_local_runtime_services_input(
    profile: RebornProfile,
    owner_id: &str,
    config: &RebornBootConfig,
    options: RuntimeInputOptions,
) -> anyhow::Result<RebornBuildInput> {
    let local_runtime_root = local_runtime_storage_root(config, profile);
    let workspace_root = std::env::current_dir()
        .with_context(|| format!("failed to resolve current directory for {profile} workspace"))?;
    let mut services_input = local_runtime_build_input_with_options(
        composition_profile(profile),
        owner_id,
        local_runtime_root,
        RebornRuntimeProfileOptions {
            confirm_host_access: options.confirm_host_access,
        },
    )
    .with_context(|| format!("failed to build local-runtime services for profile={profile}"))?
    .with_local_runtime_workspace_root(workspace_root);
    if services_input.requires_local_runtime_confirmed_host_home_root() {
        let host_home_root =
            confirmed_host_home_root(options).context("local-dev-yolo host access")?;
        services_input = services_input.with_local_runtime_confirmed_host_home_root(host_home_root);
    }
    services_input = services_input.with_optional_nearai_mcp_bootstrap_config(
        nearai_mcp_bootstrap_config_from_env().context("NEAR AI MCP bootstrap config")?,
    );
    Ok(services_input)
}

#[cfg(feature = "postgres")]
fn build_hosted_single_tenant_services_input(
    profile: RebornProfile,
    owner_id: &str,
    config: &RebornBootConfig,
    config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
) -> anyhow::Result<RebornBuildInput> {
    let workspace_root = std::env::current_dir()
        .context("failed to resolve current directory for hosted single-tenant workspace")?;
    let runtime_policy = hosted_single_tenant_runtime_policy()
        .context("failed to resolve hosted single-tenant runtime policy")?;
    Ok(
        RebornBuildInput::hosted_single_tenant_postgres_from_config_and_env(
            composition_profile(profile),
            owner_id,
            local_runtime_storage_root(config, profile),
            config_file,
        )
        .map_err(anyhow::Error::from)?
        .with_runtime_policy(runtime_policy)
        .with_local_runtime_workspace_root(workspace_root)
        .with_optional_nearai_mcp_bootstrap_config(
            nearai_mcp_bootstrap_config_from_env().context("NEAR AI MCP bootstrap config")?,
        ),
    )
}

#[cfg(not(feature = "postgres"))]
fn build_hosted_single_tenant_services_input(
    profile: RebornProfile,
    _owner_id: &str,
    _config: &RebornBootConfig,
    _config_file: Option<&ironclaw_reborn_config::RebornConfigFile>,
) -> anyhow::Result<RebornBuildInput> {
    anyhow::bail!(
        "profile={profile} requires a binary built with the `postgres` feature for hosted \
         single-tenant storage; the default PostgreSQL URL env var is IRONCLAW_REBORN_POSTGRES_URL"
    )
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

#[cfg(feature = "slack-v2-host-beta")]
pub(crate) fn resolve_slack_personal_oauth_redirect_uri_from_env()
-> anyhow::Result<Option<OAuthRedirectUri>> {
    resolve_slack_personal_oauth_redirect_uri(optional_nonempty_env)
}

#[cfg(any(feature = "slack-v2-host-beta", test))]
fn resolve_slack_personal_oauth_redirect_uri(
    mut lookup: impl FnMut(&str) -> Option<String>,
) -> anyhow::Result<Option<OAuthRedirectUri>> {
    let Some(raw) = lookup("IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI") else {
        return Ok(None);
    };
    let uri = OAuthRedirectUri::new(raw)
        .context("invalid IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI")?;
    Ok(Some(uri))
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

    tracing::debug!(
        target = "ironclaw::reborn::cli::google_oauth",
        has_client_secret = client.client_secret.is_some(),
        has_hosted_domain_hint = hosted_domain_hint.is_some(),
        "Google OAuth backend config resolved from environment"
    );

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

pub(crate) fn local_runtime_storage_root(
    config: &RebornBootConfig,
    profile: RebornProfile,
) -> PathBuf {
    config
        .home()
        .path()
        .join(profile.local_runtime_storage_subdir())
}

fn composition_profile(profile: RebornProfile) -> RebornCompositionProfile {
    match profile {
        RebornProfile::LocalDev => RebornCompositionProfile::LocalDev,
        RebornProfile::LocalDevYolo => RebornCompositionProfile::LocalDevYolo,
        RebornProfile::HostedSingleTenant => RebornCompositionProfile::HostedSingleTenant,
        RebornProfile::HostedSingleTenantVolume => {
            RebornCompositionProfile::HostedSingleTenantVolume
        }
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
    if file.storage.is_some()
        && !matches!(
            profile,
            RebornProfile::HostedSingleTenant
                | RebornProfile::Production
                | RebornProfile::MigrationDryRun
        )
    {
        sections.push("[storage]");
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

/// Resolve a `[runner]` concurrency cap against the in-effect default.
///
/// - `None` (field absent from the file) → keep `current_default` (the value
///   `TurnRunnerSettings::default()` already placed in `settings`).
/// - `Some(0)` → explicit "unlimited" sentinel → `None`.
/// - `Some(n)` → that cap.
fn resolve_concurrency_cap(
    raw: Option<u32>,
    current_default: Option<std::num::NonZeroU32>,
) -> Option<std::num::NonZeroU32> {
    match raw {
        None => current_default,
        Some(0) => None,
        Some(n) => std::num::NonZeroU32::new(n),
    }
}

/// Resolve a worker-count value against the in-effect default.
///
/// - `None` (absent) → keep `current_default`.
/// - `Some(0)` → explicit "unlimited" sentinel → `None`. The scheduler
///   semaphore is then sized to `tokio::sync::Semaphore::MAX_PERMITS`, so the
///   per-user / per-origin caps become the only concurrency bound (used to
///   stress-test backends with no global throttle).
/// - `Some(n)` → that worker count, verbatim. The operator's explicit override
///   is trusted: a large value just sizes the scheduler semaphore counter (the
///   permit count caps concurrency, runner tasks are still only spawned per
///   claimed run), degrading smoothly toward the `0` = unlimited regime. No
///   silent clamp — mirrors the per-user / per-origin caps in
///   [`resolve_concurrency_cap`].
///
/// This is pure layering only. The upper-bound check against tokio's semaphore
/// ceiling is deliberately NOT done here so a higher-precedence env override can
/// still replace an oversized lower-precedence config value before validation;
/// see [`ensure_worker_count_within_ceiling`], applied once to the final value.
fn resolve_worker_count(
    raw: Option<usize>,
    current_default: Option<std::num::NonZeroUsize>,
) -> Option<std::num::NonZeroUsize> {
    match raw {
        None => current_default,
        Some(0) => None,
        // `n` is non-zero here (the `Some(0)` arm handled zero), so this never
        // collapses to the unlimited sentinel.
        Some(n) => std::num::NonZeroUsize::new(n),
    }
}

/// Reject a *resolved* worker count above tokio's semaphore ceiling.
///
/// The value flows into `tokio::sync::Semaphore::new`, which **panics** above
/// [`tokio::sync::Semaphore::MAX_PERMITS`]. Applied once to the FINAL effective
/// `worker_count` — after config + env precedence — so a valid env override can
/// win over an oversized lower-precedence config value instead of the config
/// value failing startup before the override is even read. The `0` = unlimited
/// path resolves to `None` (sized to exactly `MAX_PERMITS`, which tokio
/// accepts), so the unlimited sentinel stays the way to ask for "no bound".
///
/// `ironclaw_runner`'s `scheduler_permit_count` additionally saturates at the
/// ceiling as an infallible backstop for direct composition callers; this gate
/// is the operator-facing fail-loud half of that defense.
fn ensure_worker_count_within_ceiling(
    worker_count: Option<std::num::NonZeroUsize>,
) -> anyhow::Result<()> {
    if let Some(count) = worker_count {
        let n = count.get();
        anyhow::ensure!(
            n <= tokio::sync::Semaphore::MAX_PERMITS,
            "runner worker_count {n} exceeds the scheduler ceiling of {} permits \
             (tokio::sync::Semaphore::MAX_PERMITS); reduce it, or set 0 for unlimited",
            tokio::sync::Semaphore::MAX_PERMITS
        );
    }
    Ok(())
}

/// Apply an `IRONCLAW_REBORN_RUNNER_*` env override for a concurrency cap onto
/// `slot`. Absent → unchanged; `0` → unlimited (`None`); positive → that cap.
/// Strict-presence semantics: a set-but-blank / non-numeric value is fatal.
fn apply_cap_env_override(
    name: &str,
    slot: &mut Option<std::num::NonZeroU32>,
) -> anyhow::Result<()> {
    if let Some(raw) = crate::operator_env::strict_env_var_parsed::<u32>(name)? {
        *slot = resolve_concurrency_cap(Some(raw), *slot);
    }
    Ok(())
}

/// Apply the `IRONCLAW_REBORN_RUNNER_WORKER_COUNT` env override onto `slot`.
/// Absent → unchanged; `0` → unlimited (`None`); positive → that count.
/// Strict-presence semantics: a set-but-blank / non-numeric value is fatal.
///
/// Sibling of [`apply_cap_env_override`]; kept separate only because the worker
/// count is `usize` / `NonZeroUsize` (scheduler permits) while the caps are
/// `u32` / `NonZeroU32`, and stable Rust cannot express one helper generic over
/// `NonZero<T>` (`ZeroablePrimitive` is unstable).
fn apply_worker_count_env_override(
    name: &str,
    slot: &mut Option<std::num::NonZeroUsize>,
) -> anyhow::Result<()> {
    if let Some(raw) = crate::operator_env::strict_env_var_parsed::<usize>(name)? {
        *slot = resolve_worker_count(Some(raw), *slot);
    }
    Ok(())
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
        // worker_count: absent → default; `0` → unlimited (None); positive →
        // that count, verbatim. The semaphore-ceiling check is deferred to the
        // final merged value below so a valid env override can still win.
        settings.worker_count = resolve_worker_count(runner.worker_count, settings.worker_count);

        // Each cap: absent in the file → keep the struct default already in
        // `settings`; explicit `0` → "unlimited" sentinel (None); positive → cap.
        settings.max_concurrent_runs_per_user = resolve_concurrency_cap(
            runner.max_concurrent_runs_per_user,
            settings.max_concurrent_runs_per_user,
        );
        settings.max_concurrent_trigger_runs = resolve_concurrency_cap(
            runner.max_concurrent_trigger_runs,
            settings.max_concurrent_trigger_runs,
        );
        settings.max_concurrent_conversation_runs = resolve_concurrency_cap(
            runner.max_concurrent_conversation_runs,
            settings.max_concurrent_conversation_runs,
        );
    }

    // Layer 1: environment-variable overrides (highest precedence, applied
    // even when no `[runner]` config section exists). Strict-presence
    // semantics; `0` means "unlimited" for every concurrency knob — for
    // `worker_count` that removes the global scheduler throttle entirely.
    apply_worker_count_env_override(
        "IRONCLAW_REBORN_RUNNER_WORKER_COUNT",
        &mut settings.worker_count,
    )?;
    apply_cap_env_override(
        "IRONCLAW_REBORN_RUNNER_MAX_CONCURRENT_RUNS_PER_USER",
        &mut settings.max_concurrent_runs_per_user,
    )?;
    apply_cap_env_override(
        "IRONCLAW_REBORN_RUNNER_MAX_CONCURRENT_TRIGGER_RUNS",
        &mut settings.max_concurrent_trigger_runs,
    )?;
    apply_cap_env_override(
        "IRONCLAW_REBORN_RUNNER_MAX_CONCURRENT_CONVERSATION_RUNS",
        &mut settings.max_concurrent_conversation_runs,
    )?;

    // Validate the final, fully-merged worker count once (env override has the
    // highest precedence), so an oversized lower-precedence config value can be
    // rescued by a valid env override before this fail-loud ceiling check.
    ensure_worker_count_within_ceiling(settings.worker_count)?;

    Ok(settings)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::MutexGuard};

    use ironclaw_reborn_composition::{
        CredentialRefreshSettings, RebornCompositionProfile, TurnStatus,
        test_support::assistant_reply_without_text_for_test,
    };
    #[cfg(feature = "webui-v2-beta")]
    use ironclaw_reborn_composition::{LocalTriggerAccessRole, LocalTriggerAccessSource};
    use ironclaw_reborn_config::RebornBootConfig;

    use super::test_env::EnvGuard;
    #[cfg(feature = "webui-v2-beta")]
    use super::with_run_local_trigger_fire_access_checker;
    use super::{
        RuntimeInputCaller, RuntimeInputOptions, apply_credential_refresh_override, block_on_cli,
        build_runtime_input, build_runtime_input_with_options, no_assistant_text_message,
        protect_reborn_log_filter, resolve_google_oauth_config,
        resolve_slack_personal_oauth_redirect_uri, runner_settings,
    };
    // Only the `#[cfg(feature = "libsql")]` hosted-volume test consumes this.
    #[cfg(feature = "libsql")]
    use super::local_runtime_storage_root;
    use ironclaw_reborn_composition::DEFAULT_TURN_RUNNER_WORKER_COUNT;

    struct RuntimeEnvGuard {
        // Fields drop in declaration order: restore the env before releasing
        // the process-wide lock.
        _resource_governor_singleton: EnvGuard,
        _lock: MutexGuard<'static, ()>,
    }

    fn lock_runtime_env() -> RuntimeEnvGuard {
        let lock = super::test_env::lock_runtime_env();
        let resource_governor_singleton = EnvGuard::set(
            "IRONCLAW_REBORN_POSTGRES_RESOURCE_GOVERNOR_SINGLETON",
            "true",
        );
        RuntimeEnvGuard {
            _resource_governor_singleton: resource_governor_singleton,
            _lock: lock,
        }
    }

    fn parse_runner_section(toml: &str) -> ironclaw_reborn_config::RebornConfigFile {
        ironclaw_reborn_config::RebornConfigFile::parse_text(
            toml,
            &std::path::PathBuf::from("/test/config.toml"),
        )
        .expect("must parse")
    }

    #[test]
    fn runner_settings_absent_runner_gives_defaults() {
        // Hold the env lock + clear runner env so a sibling env-override test
        // cannot bleed `IRONCLAW_REBORN_RUNNER_*` into this config/default case.
        let _lock = lock_runtime_env();
        let _env = clear_runner_env();
        let settings = runner_settings(None).expect("should succeed");
        assert_eq!(
            settings.worker_count.map(|v| v.get()),
            Some(DEFAULT_TURN_RUNNER_WORKER_COUNT.get())
        );
        // Out-of-box: per-user + trigger caps protect live chat from a
        // trigger storm; conversations stay uncapped.
        assert_eq!(
            settings.max_concurrent_runs_per_user.map(|v| v.get()),
            Some(3)
        );
        assert_eq!(
            settings.max_concurrent_trigger_runs.map(|v| v.get()),
            Some(8)
        );
        assert!(settings.max_concurrent_conversation_runs.is_none());
    }

    #[test]
    fn runner_settings_present_section_absent_caps_keep_defaults() {
        // A `[runner]` section that only tunes worker_count must NOT silently
        // wipe the protective cap defaults.
        let _lock = lock_runtime_env();
        let _env = clear_runner_env();
        let cfg = parse_runner_section("[runner]\nworker_count = 7\n");
        let settings = runner_settings(Some(&cfg)).expect("should succeed");
        assert_eq!(settings.worker_count.map(|v| v.get()), Some(7));
        assert_eq!(
            settings.max_concurrent_runs_per_user.map(|v| v.get()),
            Some(3)
        );
        assert_eq!(
            settings.max_concurrent_trigger_runs.map(|v| v.get()),
            Some(8)
        );
        assert!(settings.max_concurrent_conversation_runs.is_none());
    }

    #[test]
    fn runner_settings_zero_worker_count_means_unlimited() {
        // `0` is the explicit "no global throttle" sentinel: worker_count
        // resolves to None and the scheduler semaphore is sized to
        // Semaphore::MAX_PERMITS downstream.
        let _lock = lock_runtime_env();
        let _env = clear_runner_env();
        let cfg = parse_runner_section("[runner]\nworker_count = 0\n");
        let settings = runner_settings(Some(&cfg)).expect("should succeed");
        assert!(settings.worker_count.is_none());
    }

    #[test]
    fn runner_settings_present_worker_count_round_trips() {
        let _lock = lock_runtime_env();
        let _env = clear_runner_env();
        let cfg = parse_runner_section("[runner]\nworker_count = 7\n");
        let settings = runner_settings(Some(&cfg)).expect("should succeed");
        assert_eq!(settings.worker_count.map(|v| v.get()), Some(7));
    }

    #[test]
    fn runner_settings_large_worker_count_passes_through_unclamped() {
        // A deliberate operator override is trusted verbatim — no silent clamp.
        // `0` remains the only "unlimited" sentinel.
        let _lock = lock_runtime_env();
        let _env = clear_runner_env();
        let cfg = parse_runner_section("[runner]\nworker_count = 512\n");
        let settings = runner_settings(Some(&cfg)).expect("should succeed");
        assert_eq!(settings.worker_count.map(|v| v.get()), Some(512));
    }

    #[test]
    fn runner_settings_worker_count_above_semaphore_max_is_fatal() {
        // A value above `tokio::sync::Semaphore::MAX_PERMITS` would panic
        // `Semaphore::new` at scheduler construction; it must fail loud at
        // config validation instead. `0` stays the way to ask for unlimited.
        let _lock = lock_runtime_env();
        let _env = clear_runner_env();
        let toml = format!(
            "[runner]\nworker_count = {}\n",
            tokio::sync::Semaphore::MAX_PERMITS + 1
        );
        let cfg = parse_runner_section(&toml);
        let err = runner_settings(Some(&cfg)).expect_err("oversized worker_count must be rejected");
        assert!(
            err.to_string().contains("exceeds the scheduler ceiling"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn runner_settings_worker_count_at_semaphore_max_is_accepted() {
        // The exact ceiling is valid (tokio accepts `Semaphore::new(MAX_PERMITS)`);
        // guards against an off-by-one that would reject the boundary. Covers
        // both the config-file and env-override paths.
        let max = tokio::sync::Semaphore::MAX_PERMITS;
        let _lock = lock_runtime_env();

        let _env = clear_runner_env();
        let cfg = parse_runner_section(&format!("[runner]\nworker_count = {max}\n"));
        let settings = runner_settings(Some(&cfg)).expect("ceiling value must be accepted");
        assert_eq!(settings.worker_count.map(|v| v.get()), Some(max));

        let _w = EnvGuard::set("IRONCLAW_REBORN_RUNNER_WORKER_COUNT", &max.to_string());
        let settings = runner_settings(None).expect("ceiling value must be accepted");
        assert_eq!(settings.worker_count.map(|v| v.get()), Some(max));
    }

    #[test]
    fn runner_settings_zero_caps_become_none_unlimited() {
        let _lock = lock_runtime_env();
        let _env = clear_runner_env();
        let cfg = parse_runner_section(
            "[runner]\nmax_concurrent_runs_per_user = 0\nmax_concurrent_trigger_runs = 0\nmax_concurrent_conversation_runs = 0\n",
        );
        let settings = runner_settings(Some(&cfg)).expect("should succeed");
        assert!(settings.max_concurrent_runs_per_user.is_none());
        assert!(settings.max_concurrent_trigger_runs.is_none());
        assert!(settings.max_concurrent_conversation_runs.is_none());
    }

    #[test]
    fn runner_settings_nonzero_caps_round_trip() {
        let _lock = lock_runtime_env();
        let _env = clear_runner_env();
        let cfg = parse_runner_section(
            "[runner]\nmax_concurrent_runs_per_user = 3\nmax_concurrent_trigger_runs = 5\nmax_concurrent_conversation_runs = 2\n",
        );
        let settings = runner_settings(Some(&cfg)).expect("should succeed");
        assert_eq!(
            settings.max_concurrent_runs_per_user.map(|v| v.get()),
            Some(3)
        );
        assert_eq!(
            settings.max_concurrent_trigger_runs.map(|v| v.get()),
            Some(5)
        );
        assert_eq!(
            settings.max_concurrent_conversation_runs.map(|v| v.get()),
            Some(2)
        );
    }

    /// Clear all four runner env knobs so an ambient value in the dev/CI
    /// environment cannot leak into a test asserting config-file/default
    /// behavior. Returns the guards; keep them alive for the test body.
    fn clear_runner_env() -> [EnvGuard; 4] {
        [
            EnvGuard::clear("IRONCLAW_REBORN_RUNNER_WORKER_COUNT"),
            EnvGuard::clear("IRONCLAW_REBORN_RUNNER_MAX_CONCURRENT_RUNS_PER_USER"),
            EnvGuard::clear("IRONCLAW_REBORN_RUNNER_MAX_CONCURRENT_TRIGGER_RUNS"),
            EnvGuard::clear("IRONCLAW_REBORN_RUNNER_MAX_CONCURRENT_CONVERSATION_RUNS"),
        ]
    }

    #[test]
    fn runner_env_worker_count_zero_means_unlimited() {
        let _lock = lock_runtime_env();
        let _guards = clear_runner_env();
        let _w = EnvGuard::set("IRONCLAW_REBORN_RUNNER_WORKER_COUNT", "0");
        let settings = runner_settings(None).expect("should succeed");
        assert!(settings.worker_count.is_none());
    }

    #[test]
    fn runner_env_worker_count_overrides_config_file() {
        // Env is the highest-precedence layer: it must win over a `[runner]`
        // worker_count set in the config file.
        let _lock = lock_runtime_env();
        let _guards = clear_runner_env();
        let _w = EnvGuard::set("IRONCLAW_REBORN_RUNNER_WORKER_COUNT", "4");
        let cfg = parse_runner_section("[runner]\nworker_count = 7\n");
        let settings = runner_settings(Some(&cfg)).expect("should succeed");
        assert_eq!(settings.worker_count.map(|v| v.get()), Some(4));
    }

    #[test]
    fn runner_env_worker_count_overrides_oversized_config_file() {
        // Env has the highest precedence, and the ceiling check runs on the
        // FINAL merged value — so a valid env override must rescue a config file
        // whose worker_count is above the semaphore ceiling, rather than the
        // lower-precedence config value failing startup first.
        let _lock = lock_runtime_env();
        let _guards = clear_runner_env();
        let _w = EnvGuard::set("IRONCLAW_REBORN_RUNNER_WORKER_COUNT", "512");
        let toml = format!(
            "[runner]\nworker_count = {}\n",
            tokio::sync::Semaphore::MAX_PERMITS + 1
        );
        let cfg = parse_runner_section(&toml);
        let settings =
            runner_settings(Some(&cfg)).expect("valid env override must win over oversized config");
        assert_eq!(settings.worker_count.map(|v| v.get()), Some(512));
    }

    #[test]
    fn runner_env_large_worker_count_passes_through_unclamped() {
        let _lock = lock_runtime_env();
        let _guards = clear_runner_env();
        let _w = EnvGuard::set("IRONCLAW_REBORN_RUNNER_WORKER_COUNT", "512");
        let settings = runner_settings(None).expect("should succeed");
        assert_eq!(settings.worker_count.map(|v| v.get()), Some(512));
    }

    #[test]
    fn runner_env_worker_count_above_semaphore_max_is_fatal() {
        let _lock = lock_runtime_env();
        let _guards = clear_runner_env();
        let _w = EnvGuard::set(
            "IRONCLAW_REBORN_RUNNER_WORKER_COUNT",
            &(tokio::sync::Semaphore::MAX_PERMITS + 1).to_string(),
        );
        let err = runner_settings(None).expect_err("oversized worker_count must be rejected");
        assert!(
            err.to_string().contains("exceeds the scheduler ceiling"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn runner_env_caps_zero_means_unlimited() {
        let _lock = lock_runtime_env();
        let _guards = clear_runner_env();
        let _u = EnvGuard::set("IRONCLAW_REBORN_RUNNER_MAX_CONCURRENT_RUNS_PER_USER", "0");
        let _t = EnvGuard::set("IRONCLAW_REBORN_RUNNER_MAX_CONCURRENT_TRIGGER_RUNS", "0");
        let settings = runner_settings(None).expect("should succeed");
        assert!(settings.max_concurrent_runs_per_user.is_none());
        assert!(settings.max_concurrent_trigger_runs.is_none());
    }

    #[test]
    fn runner_env_cap_overrides_config_file() {
        let _lock = lock_runtime_env();
        let _guards = clear_runner_env();
        let _u = EnvGuard::set("IRONCLAW_REBORN_RUNNER_MAX_CONCURRENT_RUNS_PER_USER", "9");
        let cfg = parse_runner_section("[runner]\nmax_concurrent_runs_per_user = 3\n");
        let settings = runner_settings(Some(&cfg)).expect("should succeed");
        assert_eq!(
            settings.max_concurrent_runs_per_user.map(|v| v.get()),
            Some(9)
        );
    }

    #[test]
    fn runner_env_blank_value_is_fatal() {
        // Strict-presence semantics: a set-but-blank slot is an operator error,
        // not a silent fall-through to the default.
        let _lock = lock_runtime_env();
        let _guards = clear_runner_env();
        let _w = EnvGuard::set("IRONCLAW_REBORN_RUNNER_WORKER_COUNT", "   ");
        let err = runner_settings(None).expect_err("blank env value must be rejected");
        assert!(
            err.to_string()
                .contains("IRONCLAW_REBORN_RUNNER_WORKER_COUNT"),
            "error should name the offending var: {err}"
        );
    }

    #[test]
    fn runner_env_non_numeric_value_is_fatal() {
        let _lock = lock_runtime_env();
        let _guards = clear_runner_env();
        let _w = EnvGuard::set("IRONCLAW_REBORN_RUNNER_WORKER_COUNT", "lots");
        let err = runner_settings(None).expect_err("non-numeric env value must be rejected");
        assert!(
            err.to_string()
                .contains("IRONCLAW_REBORN_RUNNER_WORKER_COUNT"),
            "error should name the offending var: {err}"
        );
    }

    #[test]
    fn runner_env_oversized_invalid_value_truncates_display() {
        // A long invalid value (e.g. a pasted credential) must be truncated in
        // the parse error, never echoed in full into startup logs.
        let _lock = lock_runtime_env();
        let _guards = clear_runner_env();
        let oversized = "z".repeat(100);
        let _w = EnvGuard::set("IRONCLAW_REBORN_RUNNER_WORKER_COUNT", &oversized);
        let err = runner_settings(None)
            .expect_err("oversized non-numeric env value must be rejected")
            .to_string();
        assert!(err.contains('…'), "error should be truncated: {err}");
        assert!(
            !err.contains(&oversized),
            "error must not echo the full oversized value: {err}"
        );
    }

    #[test]
    fn build_runtime_input_env_runner_worker_count_zero_reaches_runtime_input() {
        // Drives the full startup boundary to prove the WORKER_COUNT=0 env
        // override propagates onto RebornRuntimeInput.runner, not only inside
        // the runner_settings helper.
        let _lock = lock_runtime_env();
        let _guards = clear_runner_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let _w = EnvGuard::set("IRONCLAW_REBORN_RUNNER_WORKER_COUNT", "0");

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.into_os_string()),
            None,
            None,
            None,
        )
        .expect("boot config");

        let runtime_input =
            build_runtime_input(&config, RuntimeInputCaller::Run).expect("runtime input");

        assert!(runtime_input.runner.worker_count.is_none());
    }

    #[cfg(feature = "root-llm-provider")]
    #[test]
    fn runtime_inputs_carry_boot_config_for_initial_llm_reload() {
        let _lock = lock_runtime_env();
        let _guards = clear_runner_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        for caller in [RuntimeInputCaller::Run, RuntimeInputCaller::Serve] {
            let temp = tempfile::tempdir().expect("tempdir");
            let reborn_home = temp.path().join("reborn-home");
            std::fs::create_dir_all(&reborn_home).expect("mkdir");
            let config = RebornBootConfig::resolve_from_env_parts(
                Some(reborn_home.into_os_string()),
                None,
                None,
                None,
            )
            .expect("boot config");

            let runtime_input = build_runtime_input(&config, caller).expect("runtime input");

            let boot = runtime_input.boot.unwrap_or_else(|| {
                panic!("{caller:?} must carry boot config for the initial LLM reload")
            });
            assert_eq!(boot.home().path(), config.home().path());
        }
    }

    #[test]
    fn runner_env_cap_blank_value_is_fatal() {
        // Strict-presence semantics apply to cap vars, not just worker_count:
        // a set-but-blank slot must be rejected rather than silently ignored.
        let _lock = lock_runtime_env();
        let _guards = clear_runner_env();
        let _u = EnvGuard::set("IRONCLAW_REBORN_RUNNER_MAX_CONCURRENT_RUNS_PER_USER", "   ");
        let err = runner_settings(None).expect_err("blank env cap value must be rejected");
        assert!(
            err.to_string()
                .contains("IRONCLAW_REBORN_RUNNER_MAX_CONCURRENT_RUNS_PER_USER"),
            "error should name the offending var: {err}"
        );
    }

    #[test]
    fn runner_env_cap_non_numeric_value_is_fatal() {
        let _lock = lock_runtime_env();
        let _guards = clear_runner_env();
        let _u = EnvGuard::set(
            "IRONCLAW_REBORN_RUNNER_MAX_CONCURRENT_RUNS_PER_USER",
            "many",
        );
        let err = runner_settings(None).expect_err("non-numeric env cap value must be rejected");
        assert!(
            err.to_string()
                .contains("IRONCLAW_REBORN_RUNNER_MAX_CONCURRENT_RUNS_PER_USER"),
            "error should name the offending var: {err}"
        );
    }

    #[test]
    fn runner_env_conversation_runs_positive_value_overrides() {
        // max_concurrent_conversation_runs defaults to None (unlimited); a
        // positive env value must set a bounded cap so a misspelled or
        // silently-ignored knob cannot escape detection.
        let _lock = lock_runtime_env();
        let _guards = clear_runner_env();
        let _c = EnvGuard::set(
            "IRONCLAW_REBORN_RUNNER_MAX_CONCURRENT_CONVERSATION_RUNS",
            "2",
        );
        let settings = runner_settings(None).expect("should succeed");
        assert_eq!(
            settings.max_concurrent_conversation_runs.map(|v| v.get()),
            Some(2)
        );
    }

    #[test]
    fn reborn_log_filter_suppresses_noisy_targets_for_broad_debug() {
        let filter = protect_reborn_log_filter("debug");

        assert!(filter.contains("debug"));
        assert!(filter.contains("tokio_postgres=warn"));
        assert!(filter.contains("deadpool_postgres=warn"));
        assert!(filter.contains("h2=warn"));
        assert!(filter.contains("hyper=warn"));
        assert!(filter.contains("hyper_util=warn"));
        assert!(filter.contains("reqwest=warn"));
        assert!(filter.contains("rustls=warn"));
        assert!(filter.contains("tower=warn"));
        assert!(filter.contains("tower_http=warn"));
        assert!(filter.contains("ironclaw_llm=info"));
    }

    #[test]
    fn reborn_log_filter_keeps_explicit_noisy_target_directives() {
        let filter = protect_reborn_log_filter(
            "debug,tokio_postgres::query=debug,ironclaw_llm::nearai_chat=debug",
        );

        assert!(filter.contains("tokio_postgres::query=debug"));
        assert!(!filter.contains("tokio_postgres=warn"));
        assert!(filter.contains("ironclaw_llm::nearai_chat=debug"));
        assert!(!filter.contains("ironclaw_llm=info"));
        assert!(filter.contains("reqwest=warn"));
    }

    #[test]
    fn reborn_log_filter_blank_env_uses_info_with_noisy_target_suppression() {
        let filter = protect_reborn_log_filter("   ");

        assert!(filter.starts_with("info,"));
        assert!(filter.contains("tokio_postgres=warn"));
    }

    #[test]
    fn credential_refresh_override_keeps_caller_default_without_env() {
        // Serve base is enabled; absent env leaves it enabled.
        let serve = apply_credential_refresh_override(
            CredentialRefreshSettings::enabled(),
            Err(std::env::VarError::NotPresent),
        )
        .expect("absent env is valid");
        assert!(serve.enabled, "Serve default stays on when env unset");
        // Non-Serve base is disabled; absent env leaves it disabled.
        let other = apply_credential_refresh_override(
            CredentialRefreshSettings::default(),
            Err(std::env::VarError::NotPresent),
        )
        .expect("absent env is valid");
        assert!(!other.enabled, "non-Serve default stays off when env unset");
    }

    #[test]
    fn credential_refresh_override_kill_switch_disables() {
        for raw in ["0", "false", "FALSE", " 0 "] {
            let out = apply_credential_refresh_override(
                CredentialRefreshSettings::enabled(),
                Ok(raw.to_string()),
            )
            .expect("valid kill-switch value");
            assert!(!out.enabled, "kill-switch {raw:?} must disable");
        }
    }

    #[test]
    fn credential_refresh_override_force_on_enables() {
        for raw in ["1", "true", "TRUE", " true "] {
            let out = apply_credential_refresh_override(
                CredentialRefreshSettings::default(),
                Ok(raw.to_string()),
            )
            .expect("valid force-on value");
            assert!(out.enabled, "force-on {raw:?} must enable");
        }
    }

    #[test]
    fn credential_refresh_override_blank_falls_through_to_base() {
        let out = apply_credential_refresh_override(
            CredentialRefreshSettings::enabled(),
            Ok("   ".to_string()),
        )
        .expect("blank value is valid");
        assert!(out.enabled, "blank value keeps the caller base");
    }

    #[test]
    fn credential_refresh_override_invalid_value_is_error() {
        let result = apply_credential_refresh_override(
            CredentialRefreshSettings::default(),
            Ok("maybe".to_string()),
        );
        assert!(result.is_err(), "invalid value must be a hard error");
    }

    fn clear_trigger_poller_env() -> (EnvGuard, EnvGuard) {
        (
            EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_ENABLED"),
            EnvGuard::clear("IRONCLAW_TRIGGER_POLLER_INTERVAL_SECS"),
        )
    }

    fn clear_credential_refresh_env() -> EnvGuard {
        EnvGuard::clear("IRONCLAW_CREDENTIAL_REFRESH_ENABLED")
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
    fn no_assistant_text_message_formats_failed_reply_with_category() {
        let reply = assistant_reply_without_text_for_test(
            TurnStatus::Failed,
            Some("scheduler_executor_panic"),
        );

        let message = no_assistant_text_message(&reply);

        assert!(
            message.contains("The agent runtime stopped unexpectedly."),
            "{message}"
        );
        assert!(
            message.contains("failure_category=scheduler_executor_panic"),
            "{message}"
        );
        assert!(message.contains("status=Failed"), "{message}");
        assert!(
            message.contains(&format!("run_id={}", reply.run_id)),
            "{message}"
        );
    }

    #[test]
    fn build_runtime_input_maps_configured_cli_identity() {
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        .expect("runtime input")
        .inner;
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

    #[cfg(feature = "libsql")]
    #[test]
    fn build_runtime_input_accepts_hosted_single_tenant_volume_profile() {
        let _lock = lock_runtime_env();
        let (_enabled, _interval) = clear_trigger_poller_env();

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.clone().into_os_string()),
            None,
            None,
            Some("hosted-single-tenant-volume".into()),
        )
        .expect("boot config");

        let runtime_input =
            build_runtime_input(&config, RuntimeInputCaller::Run).expect("runtime input");
        assert!(!runtime_input.grants_trusted_laptop_access());
        let services = runtime_input.services.expect("services input");
        let policy = services.runtime_policy().expect("runtime policy");

        assert_eq!(
            services.profile(),
            RebornCompositionProfile::HostedSingleTenantVolume
        );
        assert_eq!(policy.process_backend.as_str(), "none");
        assert_eq!(policy.filesystem_backend.as_str(), "scoped_virtual");
        assert_eq!(
            local_runtime_storage_root(
                &config,
                ironclaw_reborn_config::RebornProfile::HostedSingleTenantVolume,
            ),
            reborn_home.join("hosted-single-tenant-volume")
        );
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
        let _lock = lock_runtime_env();
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
    fn build_runtime_input_for_hosted_volume_rejects_storage_section() {
        let _lock = lock_runtime_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let (_temp, config) = boot_config_with_config_toml(
            "hosted-single-tenant-volume",
            r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"
"#,
        );

        let err = build_runtime_input(&config, RuntimeInputCaller::Run)
            .err()
            .expect("hosted volume profile must reject production storage section");

        assert!(
            err.to_string().contains("[storage]"),
            "error must mention storage section, got: {err:#}"
        );
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_requires_storage_section() {
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
    fn build_runtime_input_hosted_single_tenant_constructs_postgres_local_runtime_input() {
        let _lock = lock_runtime_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let (database_sslmode, allow_cleartext) = clear_reborn_postgres_tls_env();
        let postgres_url = EnvGuard::set(
            "IRONCLAW_REBORN_POSTGRES_URL",
            "postgres://event_user:RAW_PASSWORD_SENTINEL_3162@db.example.com/events?sslmode=require",
        );
        let secret_master_key =
            EnvGuard::set("IRONCLAW_REBORN_SECRET_MASTER_KEY", "test-master-key");
        let pool_max_size = EnvGuard::set("IRONCLAW_REBORN_POSTGRES_POOL_MAX_SIZE", "1");
        let (_temp, config) = boot_config_with_config_toml(
            "hosted-single-tenant",
            r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"
"#,
        );

        let runtime_input =
            build_runtime_input(&config, RuntimeInputCaller::Serve).expect("runtime input");
        let services = runtime_input.services.expect("services input");
        let policy = services.runtime_policy().expect("runtime policy");

        assert_eq!(
            services.profile(),
            RebornCompositionProfile::HostedSingleTenant
        );
        assert_eq!(policy.requested_profile.as_str(), "local_dev");
        assert!(!services.grants_trusted_laptop_access());
        drop(pool_max_size);
        drop(secret_master_key);
        drop(postgres_url);
        drop(database_sslmode);
        drop(allow_cleartext);
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_rejects_invalid_postgres_pool_max_size_override() {
        let _lock = lock_runtime_env();
        let (_enabled, _interval) = clear_trigger_poller_env();
        let (database_sslmode, allow_cleartext) = clear_reborn_postgres_tls_env();
        let postgres_url = EnvGuard::set(
            "IRONCLAW_REBORN_POSTGRES_URL",
            "postgres://event_user:RAW_PASSWORD_SENTINEL_3162@db.example.com/events?sslmode=require",
        );
        let secret_master_key =
            EnvGuard::set("IRONCLAW_REBORN_SECRET_MASTER_KEY", "test-master-key");
        let pool_max_size = EnvGuard::set("IRONCLAW_REBORN_POSTGRES_POOL_MAX_SIZE", "0");
        let (_temp, config) = boot_config_with_config_toml(
            "hosted-single-tenant",
            r#"
[storage]
backend = "postgres"
url_env = "IRONCLAW_REBORN_POSTGRES_URL"
secret_master_key_env = "IRONCLAW_REBORN_SECRET_MASTER_KEY"
"#,
        );

        let err = match build_runtime_input(&config, RuntimeInputCaller::Serve) {
            Ok(_) => panic!("zero pool override must fail"),
            Err(error) => error,
        };

        assert!(
            err.to_string()
                .contains("IRONCLAW_REBORN_POSTGRES_POOL_MAX_SIZE"),
            "error must identify pool override env var: {err:#}"
        );
        drop(pool_max_size);
        drop(secret_master_key);
        drop(postgres_url);
        drop(database_sslmode);
        drop(allow_cleartext);
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn build_runtime_input_production_preserves_whitespace_secret_master_key() {
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
    fn build_runtime_input_maps_credential_refresh_caller_defaults() {
        let _lock = lock_runtime_env();
        let (_trigger_enabled, _trigger_interval) = clear_trigger_poller_env();
        let _credential_refresh = clear_credential_refresh_env();

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.clone().into_os_string()),
            None,
            None,
            None,
        )
        .expect("boot config");

        let run_input =
            build_runtime_input(&config, RuntimeInputCaller::Run).expect("run runtime input");
        assert!(
            !run_input.credential_refresh.enabled,
            "run must keep proactive credential refresh disabled by default"
        );

        let serve_input =
            build_runtime_input(&config, RuntimeInputCaller::Serve).expect("serve runtime input");
        assert!(
            serve_input.credential_refresh.enabled,
            "serve must enable proactive credential refresh by default"
        );
    }

    #[test]
    fn build_runtime_input_maps_credential_refresh_env_overrides() {
        let _lock = lock_runtime_env();
        let (_trigger_enabled, _trigger_interval) = clear_trigger_poller_env();

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.into_os_string()),
            None,
            None,
            None,
        )
        .expect("boot config");

        let force_on = EnvGuard::set("IRONCLAW_CREDENTIAL_REFRESH_ENABLED", "true");
        let run_input =
            build_runtime_input(&config, RuntimeInputCaller::Run).expect("run runtime input");
        assert!(
            run_input.credential_refresh.enabled,
            "env force-on must reach runtime_input for run callers"
        );
        drop(force_on);

        let kill_switch = EnvGuard::set("IRONCLAW_CREDENTIAL_REFRESH_ENABLED", "false");
        let serve_input =
            build_runtime_input(&config, RuntimeInputCaller::Serve).expect("serve runtime input");
        assert!(
            !serve_input.credential_refresh.enabled,
            "env kill-switch must reach runtime_input for serve callers"
        );
        drop(kill_switch);
    }

    #[test]
    fn build_runtime_input_rejects_invalid_credential_refresh_env() {
        let _lock = lock_runtime_env();
        let (_trigger_enabled, _trigger_interval) = clear_trigger_poller_env();
        let _credential_refresh = EnvGuard::set("IRONCLAW_CREDENTIAL_REFRESH_ENABLED", "maybe");

        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("mkdir");
        let config = RebornBootConfig::resolve_from_env_parts(
            Some(reborn_home.into_os_string()),
            None,
            None,
            None,
        )
        .expect("boot config");

        let err = match build_runtime_input(&config, RuntimeInputCaller::Serve) {
            Ok(_) => panic!("invalid credential refresh env must fail runtime input build"),
            Err(err) => err,
        };

        assert!(
            err.to_string()
                .contains("IRONCLAW_CREDENTIAL_REFRESH_ENABLED must be one of 1, true, 0, false"),
            "{err:#}"
        );
    }

    #[test]
    fn build_runtime_input_maps_trigger_poller_enabled_config() {
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
        let _lock = lock_runtime_env();
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
    fn resolve_slack_personal_oauth_redirect_uri_returns_none_when_unset() {
        let uri = resolve_slack_personal_oauth_redirect_uri(|_| None)
            .expect("empty env should not fail setup");
        assert!(uri.is_none());
    }

    #[test]
    fn resolve_slack_personal_oauth_redirect_uri_returns_uri_when_set() {
        const REDIRECT_URI: &str =
            "http://127.0.0.1:3000/api/reborn/product-auth/oauth/slack_personal/callback";
        let vars = HashMap::from([(
            "IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI",
            REDIRECT_URI,
        )]);
        let uri =
            resolve_slack_personal_oauth_redirect_uri(|name| vars.get(name).map(|v| v.to_string()))
                .expect("valid redirect URI resolves")
                .expect("URI present when var is set");
        assert_eq!(uri.as_str(), REDIRECT_URI);
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
