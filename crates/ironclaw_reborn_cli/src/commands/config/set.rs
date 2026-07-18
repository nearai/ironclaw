//! `ironclaw config set <key> [value]` — write one config value,
//! routed through `capability_config::ConfigKey`'s alias table so the
//! destination (config.toml literal / encrypted secret store / token
//! file), shape validation, and secret-vs-non-secret refusal all come
//! from that single chokepoint.

use std::io::{IsTerminal, Write as _};
use std::path::Path;

use anyhow::Context as _;
use clap::Args;
use ironclaw_reborn_config::{GoogleFieldUpdate, GoogleOauthConfigUpdate, RebornHome};

use super::capability_config::{ConfigDestination, ConfigKey, ShapeVerdict, validate_shape};
use crate::context::RebornCliContext;

#[derive(Debug, Args)]
pub(super) struct ConfigSetCommand {
    /// Dot-separated config key (e.g. google.client_id, openai.api_key,
    /// slack.enabled, webui.token).
    key: String,
    /// Value to set for non-secret keys. Secret-destination keys
    /// (`<provider>.api_key`, `google.client_secret`) reject positional values
    /// and always prompt with input hidden.
    value: Option<String>,
    /// `webui.token` only: rotate the WebChat v2 bearer token. Invalidates
    /// every existing browser session (the token doubles as the
    /// session-signing key).
    #[arg(long)]
    rotate: bool,
}

impl ConfigSetCommand {
    pub(super) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        let Some(key) = ConfigKey::classify(&self.key) else {
            anyhow::bail!(unknown_key_message(&self.key));
        };

        if matches!(key, ConfigKey::WebuiToken) {
            return execute_webui_token(&context, self.value, self.rotate);
        }
        if self.rotate {
            anyhow::bail!("--rotate is only valid for `config set webui.token`");
        }

        set_value_key(
            &context,
            key,
            self.value,
            &mut StdinSecretValueSource,
            &LocalDevSecretStoreOpener,
        )
    }
}

fn unknown_key_message(key: &str) -> String {
    format!(
        "unknown config key `{key}` for `config set`\nSupported keys: <provider>.api_key \
         (default provider `{}`), google.client_id, google.client_secret, google.redirect_uri, \
         slack.enabled, webui.token (--rotate only)\nRun `ironclaw config list` to see \
         all readable keys",
        super::init::DEFAULT_LLM_PROVIDER_ID,
    )
}

fn describe_key(key: &ConfigKey) -> String {
    match key {
        ConfigKey::LlmApiKey { provider_id } => format!("{provider_id}.api_key"),
        ConfigKey::GoogleClientId => "google.client_id".to_string(),
        ConfigKey::GoogleClientSecret => "google.client_secret".to_string(),
        ConfigKey::GoogleRedirectUri => "google.redirect_uri".to_string(),
        ConfigKey::SlackEnabled => "slack.enabled".to_string(),
        ConfigKey::WebuiToken => "webui.token".to_string(),
    }
}

/// Core routing: resolve a value (arg or hidden prompt), apply shape
/// validation, refuse a secret-shaped value headed for `config.toml`,
/// write to the alias's destination, then print the explicit apply step
/// (`config set` never restarts anything itself — see `print_apply_step`).
/// Free function (not a method) so tests can inject a fake prompt source
/// and secret-store opener — the same shape `onboard`'s
/// `provision_llm_credentials` already established.
fn set_value_key(
    context: &RebornCliContext,
    key: ConfigKey,
    value_arg: Option<String>,
    prompt: &mut dyn SecretValueSource,
    store_opener: &dyn SecretStoreOpener,
) -> anyhow::Result<()> {
    let label = describe_key(&key);
    if key.is_secret_prompted() && value_arg.is_some() {
        anyhow::bail!(
            "`config set {label}` does not accept a positional value because shell history and \
             process listings can expose it; omit the value and use the hidden interactive prompt"
        );
    }
    let value = match value_arg {
        Some(value) => value,
        None if key.is_secret_prompted() => prompt.prompt(&label)?,
        None => anyhow::bail!("`config set {label}` requires a value"),
    };
    let value = value.trim().to_string();
    if key.is_secret_prompted() && value.is_empty() {
        anyhow::bail!("`config set {label}` requires a non-blank value");
    }

    // Secret-shape law FIRST, before shape validation: a value destined for
    // config.toml must not look like inline secret material. Must run before
    // `validate_shape` so a secret pasted into the wrong key (e.g. an API key
    // into `google.redirect_uri`) is caught here, whose error never echoes
    // the raw value, before `validate_shape`'s Reject arms would see it.
    if key.destination() == ConfigDestination::ConfigToml
        && let Err(error) = ironclaw_reborn_config::reject_inline_secret(label.clone(), &value)
    {
        anyhow::bail!("refusing to write `{label}` to config.toml: {error}");
    }

    match validate_shape(&key, &value) {
        ShapeVerdict::Reject(message) => anyhow::bail!(message),
        ShapeVerdict::Warn(message) => eprintln!("warning: {message}"),
        ShapeVerdict::Ok => {}
    }

    // This is a host-owned operator/bootstrap write plane, not an in-turn
    // model capability. Sending API keys, OAuth secrets, or the WebUI token
    // through ToolDispatcher would make secret ingress model-visible.
    // dispatch-exempt: host-owned operator config/secret ingress, not in-turn tool dispatch
    let home = context.boot_config().home();
    match &key {
        ConfigKey::LlmApiKey { provider_id } => {
            write_llm_api_key(context, provider_id, &value, store_opener)?;
        }
        ConfigKey::GoogleClientId => {
            write_google_field(home, Some(GoogleFieldUpdate::Set(value.clone())), None)?;
        }
        ConfigKey::GoogleRedirectUri => {
            write_google_field(home, None, Some(GoogleFieldUpdate::Set(value.clone())))?;
        }
        ConfigKey::GoogleClientSecret => {
            write_google_client_secret(context, &value, store_opener)?;
        }
        ConfigKey::SlackEnabled => {
            write_slack_enabled(home, &value)?;
        }
        ConfigKey::WebuiToken => unreachable!("handled by execute_webui_token"),
    }

    println!("{label}: saved");
    print_remaining_setup_guidance(&key);
    print_apply_step();
    Ok(())
}

/// After a successful write, print the remaining BYO setup steps for the
/// capability this key belongs to, via
/// `capability_config::google_remediation_text`/`slack_remediation_text`.
fn print_remaining_setup_guidance(key: &ConfigKey) {
    match key {
        ConfigKey::GoogleClientId
        | ConfigKey::GoogleClientSecret
        | ConfigKey::GoogleRedirectUri => {
            println!();
            println!("{}", super::capability_config::google_remediation_text());
        }
        ConfigKey::SlackEnabled => {
            println!();
            println!(
                "{}",
                super::capability_config::slack_remediation_text(&default_webui_base_url())
            );
        }
        ConfigKey::LlmApiKey { .. } | ConfigKey::WebuiToken => {}
    }
}

#[cfg(feature = "webui-v2-beta")]
fn default_webui_base_url() -> String {
    format!(
        "http://{}:{}",
        crate::commands::serve::DEFAULT_SERVE_HOST,
        crate::commands::serve::DEFAULT_SERVE_PORT
    )
}

#[cfg(not(feature = "webui-v2-beta"))]
fn default_webui_base_url() -> String {
    "http://127.0.0.1:3000".to_string()
}

fn write_google_field(
    home: &RebornHome,
    client_id: Option<GoogleFieldUpdate>,
    redirect_uri: Option<GoogleFieldUpdate>,
) -> anyhow::Result<()> {
    let update = GoogleOauthConfigUpdate {
        client_id: client_id.unwrap_or_default(),
        redirect_uri: redirect_uri.unwrap_or_default(),
        hosted_domain_hint: GoogleFieldUpdate::Keep,
    };
    ironclaw_reborn_config::update_google_oauth_config(&home.config_file_path(), &update)
        .map_err(anyhow::Error::from)
}

fn write_slack_enabled(home: &RebornHome, value: &str) -> anyhow::Result<()> {
    let enabled = value.eq_ignore_ascii_case("true");
    ironclaw_reborn_config::update_slack_enabled(&home.config_file_path(), enabled)
        .map_err(anyhow::Error::from)
}

#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn write_llm_api_key(
    context: &RebornCliContext,
    provider_id: &str,
    value: &str,
    store_opener: &dyn SecretStoreOpener,
) -> anyhow::Result<()> {
    let admin =
        ironclaw_reborn_composition::RebornProviderAdmin::new(context.boot_config().clone());
    let canonical_provider_id = admin
        .resolve_provider_id(provider_id)
        .map_err(anyhow::Error::from)?;
    let storage_root = crate::runtime::local_runtime_storage_root(
        context.boot_config(),
        context.boot_config().profile(),
    );
    let store = store_opener.open_llm_key_store(&storage_root)?;
    let value_owned = value.to_string();
    crate::runtime::block_on_cli(async move {
        store
            .put_plaintext(&canonical_provider_id, value_owned)
            .await
            .map_err(anyhow::Error::from)
    })
}

#[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
fn write_llm_api_key(
    _context: &RebornCliContext,
    _provider_id: &str,
    _value: &str,
    _store_opener: &dyn SecretStoreOpener,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "`config set <provider>.api_key` requires the binary to be built with the `libsql` and \
         `root-llm-provider` Cargo features"
    )
}

#[cfg(feature = "libsql")]
fn write_google_client_secret(
    context: &RebornCliContext,
    value: &str,
    store_opener: &dyn SecretStoreOpener,
) -> anyhow::Result<()> {
    let storage_root = crate::runtime::local_runtime_storage_root(
        context.boot_config(),
        context.boot_config().profile(),
    );
    let store = store_opener.open_google_oauth_secret_store(&storage_root)?;
    let value_owned = value.to_string();
    crate::runtime::block_on_cli(async move {
        store
            .put_plaintext(value_owned)
            .await
            .map_err(anyhow::Error::from)
    })
}

#[cfg(not(feature = "libsql"))]
fn write_google_client_secret(
    _context: &RebornCliContext,
    _value: &str,
    _store_opener: &dyn SecretStoreOpener,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "`config set google.client_secret` requires the binary to be built with the `libsql` \
         Cargo feature"
    )
}

// ── Apply step ──────────────────────────────────────────────────

/// `config set` never restarts anything itself: this CLI invocation may be
/// running from inside a live `serve` process's own tool-call loop (an
/// agent shelling out to its own CLI), and auto-restarting would let that
/// process kill itself mid-turn. A running `ironclaw` service (or a
/// manually run `serve`) keeps serving the old value until this step is
/// run — one explicit, unconditional line for every caller (human, an
/// agent's own shell tool, a script) rather than branching on TTY-ness or
/// service state.
///
/// This is the CLI-surface twin of
/// `ironclaw_reborn_config::apply_step_text` (the canonical sentence used
/// by the composition-layer tool-error surface, e.g. `gsuite.rs`) — kept
/// as a separate literal rather than unified onto that helper because
/// several CLI tests already pin this exact `  to apply: ...` wording;
/// treat `apply_step_text` as the source of truth for the *content* of
/// this instruction if the two ever need to change together.
fn print_apply_step() {
    println!("  to apply: ironclaw service restart");
}

// ── webui.token --rotate ────────────────────────────────────────

fn execute_webui_token(
    context: &RebornCliContext,
    value: Option<String>,
    rotate: bool,
) -> anyhow::Result<()> {
    if value.is_some() {
        anyhow::bail!(
            "`config set webui.token` does not accept a value; use --rotate to rotate it"
        );
    }
    if !rotate {
        anyhow::bail!(
            "`config set webui.token` requires --rotate (there is nothing else to set on this \
             key)"
        );
    }
    if let Some(env_var_name) = configured_webui_token_env_override(context)? {
        anyhow::bail!(
            "refusing to rotate: {env_var_name} is set and non-empty — the env var overrides \
             the token file, so rotating the file has no effect; unset {env_var_name} or rotate \
             its value directly instead"
        );
    }
    println!(
        "warning: rotating the WebChat v2 token invalidates every existing browser session \
         (the token also signs sessions) — anyone signed in will need to sign in again"
    );
    let home = context.boot_config().home();
    // dispatch-exempt: host-owned operator token rotation, not in-turn tool dispatch
    crate::webui_token::rotate_webui_token_file(home.path())?;
    println!("webui.token: rotated");
    print_apply_step();
    // The token file on disk is already rotated, but a running `serve`
    // process keeps the old token in memory until it restarts — so the
    // login link below (already built from the new token) only becomes
    // valid once `ironclaw service restart` (printed above) runs.
    #[cfg(feature = "webui-v2-beta")]
    if let Some(link) = crate::webui_token::login_link(home)? {
        println!("login_link: {link} (valid after restart)");
    }
    Ok(())
}

/// `Some(env_var_name)` when the operator's `[webui].env_token_var`
/// (defaulting to [`crate::webui_token::DEFAULT_ENV_TOKEN_VAR`]) is set to
/// a non-empty value in the process environment — meaning `serve` reads
/// the token from that env var, not from the rotated file, so rotating
/// the file would silently do nothing at runtime (see
/// `crate::webui_token::resolve_webui_token`'s precedence). `None` means
/// no such override is active and rotation takes effect normally.
fn configured_webui_token_env_override(
    context: &RebornCliContext,
) -> anyhow::Result<Option<String>> {
    let config_file = crate::runtime::read_config_file(context.boot_config())?;
    let env_var_name = config_file
        .as_ref()
        .and_then(|file| file.webui.as_ref())
        .and_then(|section| section.env_token_var.as_deref())
        .unwrap_or(crate::webui_token::DEFAULT_ENV_TOKEN_VAR)
        .to_string();
    Ok(std::env::var_os(&env_var_name)
        .filter(|value| !value.is_empty())
        .map(|_| env_var_name))
}

// ── Injected seams (mirrors onboard's PromptSource/LlmKeyStoreOpener) ──

/// Where `set_value_key`'s hidden secret-value prompt comes from. Mirrors
/// `onboard::prompts::PromptSource` — injected so tests can supply a fixed
/// answer without a real terminal, and so [`StdinSecretValueSource`] is the
/// only place that decides "is this session interactive".
trait SecretValueSource {
    fn prompt(&mut self, label: &str) -> anyhow::Result<String>;
}

struct StdinSecretValueSource;

impl SecretValueSource for StdinSecretValueSource {
    fn prompt(&mut self, label: &str) -> anyhow::Result<String> {
        if !std::io::stdin().is_terminal() {
            anyhow::bail!(
                "`config set {label}` requires an interactive terminal so the value can be \
                 entered with input hidden"
            );
        }
        print!("{label} (input hidden): ");
        std::io::stdout()
            .flush()
            .context("flush stdout before secret prompt")?;
        let value = crate::commands::onboard::prompts::read_masked_line()?;
        println!();
        Ok(value)
    }
}

/// Where `set_value_key` gets its (already-open) encrypted secret store
/// wrappers from — one method per composition-owned store type. Returns
/// composition wrapper types, never `ironclaw_secrets` directly: production
/// code in this crate must not depend on `ironclaw_secrets` (enforced by
/// `reborn_dependency_boundaries.rs::reborn_cli_binary_crate_stays_separate_from_v1_root`).
trait SecretStoreOpener {
    #[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
    fn open_llm_key_store(
        &self,
        home_path: &Path,
    ) -> anyhow::Result<ironclaw_reborn_composition::LlmKeyStore>;

    #[cfg(feature = "libsql")]
    fn open_google_oauth_secret_store(
        &self,
        home_path: &Path,
    ) -> anyhow::Result<ironclaw_reborn_composition::GoogleOauthSecretStore>;
}

struct LocalDevSecretStoreOpener;

impl SecretStoreOpener for LocalDevSecretStoreOpener {
    #[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
    fn open_llm_key_store(
        &self,
        home_path: &Path,
    ) -> anyhow::Result<ironclaw_reborn_composition::LlmKeyStore> {
        // `config set` is a write command: create the reborn home directory
        // (if missing) before opening the store, mirroring
        // `onboard::llm_credentials::open_llm_key_store` — a never-onboarded
        // home has no directory yet, and `open_local_dev_secret_store` opens
        // a libSQL file directly under it without creating parents itself.
        std::fs::create_dir_all(home_path).map_err(|error| {
            anyhow::anyhow!("create reborn home {}: {error}", home_path.display())
        })?;
        let home_path = home_path.to_path_buf();
        crate::runtime::block_on_cli(async move {
            let store = ironclaw_reborn_composition::open_local_dev_secret_store(&home_path)
                .await
                .map_err(anyhow::Error::from)?;
            Ok::<_, anyhow::Error>(ironclaw_reborn_composition::LlmKeyStore::new(store))
        })
    }

    #[cfg(feature = "libsql")]
    fn open_google_oauth_secret_store(
        &self,
        home_path: &Path,
    ) -> anyhow::Result<ironclaw_reborn_composition::GoogleOauthSecretStore> {
        // See `open_llm_key_store` above: `config set` is a write command,
        // so create the reborn home directory before opening the store.
        std::fs::create_dir_all(home_path).map_err(|error| {
            anyhow::anyhow!("create reborn home {}: {error}", home_path.display())
        })?;
        let home_path = home_path.to_path_buf();
        crate::runtime::block_on_cli(async move {
            let store = ironclaw_reborn_composition::open_local_dev_secret_store(&home_path)
                .await
                .map_err(anyhow::Error::from)?;
            Ok::<_, anyhow::Error>(ironclaw_reborn_composition::GoogleOauthSecretStore::new(
                store,
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct FixedPromptSource {
        answers: Mutex<Vec<String>>,
    }

    impl FixedPromptSource {
        fn new(answers: Vec<&str>) -> Self {
            Self {
                answers: Mutex::new(answers.into_iter().rev().map(String::from).collect()),
            }
        }
    }

    impl SecretValueSource for FixedPromptSource {
        fn prompt(&mut self, _label: &str) -> anyhow::Result<String> {
            self.answers
                .get_mut()
                .expect("lock")
                .pop()
                .ok_or_else(|| anyhow::anyhow!("no more fixed prompt answers"))
        }
    }

    struct NeverPromptSource;
    impl SecretValueSource for NeverPromptSource {
        fn prompt(&mut self, label: &str) -> anyhow::Result<String> {
            panic!("unexpected prompt for {label}")
        }
    }

    #[cfg(feature = "libsql")]
    struct FakeSecretStoreOpener {
        store: Arc<dyn ironclaw_secrets::SecretStore>,
        opened_paths: Mutex<Vec<std::path::PathBuf>>,
    }

    #[cfg(feature = "libsql")]
    impl FakeSecretStoreOpener {
        fn new() -> Self {
            Self {
                store: Arc::new(ironclaw_secrets::InMemorySecretStore::new()),
                opened_paths: Mutex::new(Vec::new()),
            }
        }
    }

    #[cfg(feature = "libsql")]
    impl SecretStoreOpener for FakeSecretStoreOpener {
        #[cfg(feature = "root-llm-provider")]
        fn open_llm_key_store(
            &self,
            home_path: &Path,
        ) -> anyhow::Result<ironclaw_reborn_composition::LlmKeyStore> {
            self.opened_paths
                .lock()
                .expect("opened paths lock")
                .push(home_path.to_path_buf());
            Ok(ironclaw_reborn_composition::LlmKeyStore::new(
                self.store.clone(),
            ))
        }

        fn open_google_oauth_secret_store(
            &self,
            home_path: &Path,
        ) -> anyhow::Result<ironclaw_reborn_composition::GoogleOauthSecretStore> {
            self.opened_paths
                .lock()
                .expect("opened paths lock")
                .push(home_path.to_path_buf());
            Ok(ironclaw_reborn_composition::GoogleOauthSecretStore::new(
                self.store.clone(),
            ))
        }
    }

    fn config_toml(context: &RebornCliContext) -> String {
        std::fs::read_to_string(context.boot_config().home().config_file_path()).unwrap_or_default()
    }

    #[test]
    fn unknown_key_message_names_the_key_and_points_at_config_list() {
        assert!(ConfigKey::classify("nonsense.key").is_none());
        let message = unknown_key_message("nonsense.key");
        assert!(message.contains("unknown config key `nonsense.key`"));
        assert!(message.contains("config list"));
    }

    struct FailingStoreOpener;
    impl SecretStoreOpener for FailingStoreOpener {
        #[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
        fn open_llm_key_store(
            &self,
            _home_path: &Path,
        ) -> anyhow::Result<ironclaw_reborn_composition::LlmKeyStore> {
            anyhow::bail!("store opener should not be called")
        }

        #[cfg(feature = "libsql")]
        fn open_google_oauth_secret_store(
            &self,
            _home_path: &Path,
        ) -> anyhow::Result<ironclaw_reborn_composition::GoogleOauthSecretStore> {
            anyhow::bail!("store opener should not be called")
        }
    }

    #[test]
    fn google_client_id_writes_config_toml_literal_value() {
        let (_tmp, context) = RebornCliContext::test_context();
        set_value_key(
            &context,
            ConfigKey::GoogleClientId,
            Some("abc123.apps.googleusercontent.com".to_string()),
            &mut NeverPromptSource,
            &FailingStoreOpener,
        )
        .expect("must succeed");

        let toml = config_toml(&context);
        assert!(
            toml.contains("client_id = \"abc123.apps.googleusercontent.com\""),
            "config: {toml}"
        );
    }

    #[test]
    fn config_toml_values_are_trimmed_before_validation_and_write() {
        let (_tmp, context) = RebornCliContext::test_context();
        set_value_key(
            &context,
            ConfigKey::GoogleClientId,
            Some("  abc123.apps.googleusercontent.com\n".to_string()),
            &mut NeverPromptSource,
            &FailingStoreOpener,
        )
        .expect("surrounding whitespace must be ignored");

        let toml = config_toml(&context);
        assert!(
            toml.contains("client_id = \"abc123.apps.googleusercontent.com\""),
            "config: {toml}"
        );
        assert!(!toml.contains("  abc123"), "whitespace leaked: {toml}");
    }

    #[test]
    fn secret_destination_rejects_positional_values_without_echoing_them() {
        let (_tmp, context) = RebornCliContext::test_context();
        let secret = "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cho";
        let err = set_value_key(
            &context,
            ConfigKey::GoogleClientSecret,
            Some(secret.to_string()),
            &mut NeverPromptSource,
            &FailingStoreOpener,
        )
        .expect_err("secret values must only enter through the hidden prompt");
        let message = err.to_string();
        assert!(message.contains("does not accept a positional value"));
        assert!(
            !message.contains(secret),
            "secret leaked in error: {message}"
        );
    }

    #[test]
    fn google_client_secret_rejects_a_blank_prompt_before_opening_the_store() {
        let (_tmp, context) = RebornCliContext::test_context();
        let err = set_value_key(
            &context,
            ConfigKey::GoogleClientSecret,
            None,
            &mut FixedPromptSource::new(vec!["  \n\t"]),
            &FailingStoreOpener,
        )
        .expect_err("a whitespace-only Google client secret must be rejected");

        assert!(err.to_string().contains("requires a non-blank value"));
        assert!(config_toml(&context).is_empty(), "must not write config");
    }

    #[test]
    fn llm_api_key_rejects_a_blank_prompt_before_opening_the_store() {
        let (_tmp, context) = RebornCliContext::test_context();
        let err = set_value_key(
            &context,
            ConfigKey::LlmApiKey {
                provider_id: "openai".to_string(),
            },
            None,
            &mut FixedPromptSource::new(vec![" \t\n "]),
            &FailingStoreOpener,
        )
        .expect_err("a whitespace-only LLM API key must be rejected");

        assert!(err.to_string().contains("requires a non-blank value"));
        assert!(config_toml(&context).is_empty(), "must not write config");
    }

    #[test]
    fn google_client_id_rejects_bad_shape() {
        let (_tmp, context) = RebornCliContext::test_context();
        let err = set_value_key(
            &context,
            ConfigKey::GoogleClientId,
            Some("not-a-client-id".to_string()),
            &mut NeverPromptSource,
            &FailingStoreOpener,
        )
        .expect_err("bad shape must reject");
        assert!(err.to_string().contains("apps.googleusercontent.com"));
    }

    #[test]
    fn google_redirect_uri_writes_config_toml() {
        let (_tmp, context) = RebornCliContext::test_context();
        set_value_key(
            &context,
            ConfigKey::GoogleRedirectUri,
            Some("http://127.0.0.1:3000/oauth/google/callback".to_string()),
            &mut NeverPromptSource,
            &FailingStoreOpener,
        )
        .expect("must succeed");

        let toml = config_toml(&context);
        assert!(
            toml.contains("redirect_uri = \"http://127.0.0.1:3000/oauth/google/callback\""),
            "config: {toml}"
        );
    }

    #[test]
    fn slack_enabled_round_trips() {
        let (_tmp, context) = RebornCliContext::test_context();
        set_value_key(
            &context,
            ConfigKey::SlackEnabled,
            Some("true".to_string()),
            &mut NeverPromptSource,
            &FailingStoreOpener,
        )
        .expect("must succeed");

        let toml = config_toml(&context);
        assert!(toml.contains("[slack]"), "config: {toml}");
        assert!(toml.contains("enabled = true"), "config: {toml}");
    }

    #[test]
    fn config_toml_destination_refuses_secret_shaped_value() {
        let (_tmp, context) = RebornCliContext::test_context();
        let err = set_value_key(
            &context,
            ConfigKey::GoogleRedirectUri,
            Some("https://sk-proj-1234567890abcdef1234567890.example.test/cb".to_string()),
            &mut NeverPromptSource,
            &FailingStoreOpener,
        )
        .expect_err("secret-shaped value must be refused");
        assert!(err.to_string().contains("refusing to write"));
        assert!(config_toml(&context).is_empty(), "must not write");
    }

    /// Thermo MUST (secret-echo fix): the secret-shape law must run BEFORE
    /// shape validation, so a value shaped like BOTH a secret AND an
    /// invalid URL (i.e. it would also fail `validate_shape`'s URL check)
    /// is caught by `reject_inline_secret` first — whose error never
    /// echoes the raw value — rather than by `validate_shape`'s `Reject`
    /// arm, which (pre-fix) interpolated the value straight into the
    /// error message and would have echoed the secret to the terminal.
    #[test]
    fn secret_shaped_value_is_never_echoed_even_when_also_shape_invalid() {
        let (_tmp, context) = RebornCliContext::test_context();
        let secret_value = "sk-proj-1234567890abcdef1234567890";
        let err = set_value_key(
            &context,
            ConfigKey::GoogleRedirectUri,
            Some(secret_value.to_string()),
            &mut NeverPromptSource,
            &FailingStoreOpener,
        )
        .expect_err("secret-shaped value must be refused");
        let message = err.to_string();
        assert!(
            !message.contains(secret_value),
            "error must never echo the raw secret-shaped value: {message}"
        );
        assert!(config_toml(&context).is_empty(), "must not write");
    }

    #[cfg(feature = "libsql")]
    #[test]
    fn google_client_secret_writes_secret_store_only() {
        let (_tmp, context) = RebornCliContext::test_context();
        let opener = FakeSecretStoreOpener::new();
        let mut prompts = FixedPromptSource::new(vec!["GOCSPX-abc123"]);

        set_value_key(
            &context,
            ConfigKey::GoogleClientSecret,
            None,
            &mut prompts,
            &opener,
        )
        .expect("must succeed");

        assert!(
            config_toml(&context).is_empty(),
            "client_secret must never land in config.toml"
        );
        let store = opener.store.clone();
        let stored = crate::runtime::block_on_cli(async move {
            ironclaw_reborn_composition::GoogleOauthSecretStore::new(store)
                .read()
                .await
        })
        .expect("read store");
        let value = stored.expect("secret stored");
        assert_eq!(
            secrecy::ExposeSecret::expose_secret(&value),
            "GOCSPX-abc123"
        );
        assert_eq!(
            opener
                .opened_paths
                .lock()
                .expect("opened paths lock")
                .as_slice(),
            &[crate::runtime::local_runtime_storage_root(
                context.boot_config(),
                context.boot_config().profile(),
            )],
            "Google secrets must use the active profile's runtime storage root"
        );
    }

    /// `config set` on a secret-destination key may be the FIRST command a
    /// never-onboarded home runs, before `onboard` creates the reborn home
    /// directory. Drives the real `LocalDevSecretStoreOpener` (not
    /// `FakeSecretStoreOpener`) so it actually reaches
    /// `open_local_dev_secret_store`'s libSQL file-open and needs
    /// `create_dir_all` to succeed.
    #[cfg(feature = "libsql")]
    #[test]
    fn google_client_secret_write_creates_the_reborn_home_directory_on_a_never_onboarded_host() {
        let _guard = crate::runtime::test_env::lock_runtime_env();
        // Snapshot-and-restore rather than an unconditional `remove_var`:
        // the surrounding `cargo test` invocation may itself already export
        // `IRONCLAW_DISABLE_OS_KEYCHAIN=1` for the whole process (the gate
        // command does), and unconditionally clearing it on the way out
        // would wipe that out for every test running after this one for the
        // rest of the process, sending them into the real OS keychain.
        let _disable_keychain =
            crate::runtime::test_env::EnvGuard::set("IRONCLAW_DISABLE_OS_KEYCHAIN", "1");

        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        assert!(
            !home.path().exists(),
            "sanity: test_context's reborn home must not exist yet, matching a \
             never-onboarded host"
        );

        let result = set_value_key(
            &context,
            ConfigKey::GoogleClientSecret,
            None,
            &mut FixedPromptSource::new(vec!["GOCSPX-never-onboarded"]),
            &LocalDevSecretStoreOpener,
        );

        result.expect(
            "writing a secret-destination key must create the reborn home directory \
             on first use, not surface a raw SQLITE_CANTOPEN",
        );
    }

    #[cfg(feature = "libsql")]
    #[test]
    fn google_client_secret_prompts_hidden_when_no_value_given() {
        let (_tmp, context) = RebornCliContext::test_context();
        let opener = FakeSecretStoreOpener::new();
        let mut prompts = FixedPromptSource::new(vec!["GOCSPX-prompted"]);

        set_value_key(
            &context,
            ConfigKey::GoogleClientSecret,
            None,
            &mut prompts,
            &opener,
        )
        .expect("must succeed");

        let store = opener.store.clone();
        let stored = crate::runtime::block_on_cli(async move {
            ironclaw_reborn_composition::GoogleOauthSecretStore::new(store)
                .read()
                .await
        })
        .expect("read store");
        assert_eq!(
            secrecy::ExposeSecret::expose_secret(&stored.expect("secret stored")),
            "GOCSPX-prompted"
        );
    }

    #[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
    #[test]
    fn llm_api_key_writes_to_secret_store_and_not_config_toml() {
        let (_tmp, context) = RebornCliContext::test_context();
        let opener = FakeSecretStoreOpener::new();
        let mut prompts = FixedPromptSource::new(vec!["sk-test-value-1234567890"]);

        set_value_key(
            &context,
            ConfigKey::LlmApiKey {
                provider_id: "openai".to_string(),
            },
            None,
            &mut prompts,
            &opener,
        )
        .expect("must succeed");

        assert!(
            config_toml(&context).is_empty(),
            "api key must never land in config.toml"
        );
        let store = opener.store.clone();
        let stored = crate::runtime::block_on_cli(async move {
            ironclaw_reborn_composition::LlmKeyStore::new(store)
                .read("openai")
                .await
        })
        .expect("read store");
        let value = stored.expect("secret stored");
        assert_eq!(
            secrecy::ExposeSecret::expose_secret(&value),
            "sk-test-value-1234567890"
        );
        assert_eq!(
            opener
                .opened_paths
                .lock()
                .expect("opened paths lock")
                .as_slice(),
            &[crate::runtime::local_runtime_storage_root(
                context.boot_config(),
                context.boot_config().profile(),
            )],
            "LLM keys must use the active profile's runtime storage root"
        );
    }

    #[test]
    fn missing_value_errors_for_a_config_toml_destination_key() {
        let (_tmp, context) = RebornCliContext::test_context();
        let err = set_value_key(
            &context,
            ConfigKey::GoogleClientId,
            None,
            &mut NeverPromptSource,
            &FailingStoreOpener,
        )
        .expect_err("config.toml destination key with no prompt support needs a value");
        assert!(err.to_string().contains("requires a value"));
    }

    #[test]
    fn webui_token_requires_rotate_flag() {
        let (_tmp, context) = RebornCliContext::test_context();
        let err =
            execute_webui_token(&context, None, false).expect_err("without --rotate must fail");
        assert!(err.to_string().contains("--rotate"));
    }

    #[test]
    fn webui_token_rejects_a_value_argument() {
        let (_tmp, context) = RebornCliContext::test_context();
        let err = execute_webui_token(&context, Some("x".to_string()), true)
            .expect_err("a value argument must be rejected");
        assert!(err.to_string().contains("does not accept a value"));
    }

    #[test]
    fn webui_token_rotate_writes_a_new_token_file() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let _env = crate::runtime::test_env::EnvGuard::clear("IRONCLAW_REBORN_WEBUI_TOKEN");
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("mkdir");

        execute_webui_token(&context, None, true).expect("rotate must succeed");

        assert!(
            crate::webui_token::webui_token_file_is_valid(home.path()).expect("query must succeed")
        );

        // The rotated token file carries the bearer credential — it must
        // be owner-read/write only, same as every other rotation/repair
        // path in `webui_token.rs`.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let token_path = crate::webui_token::webui_token_file_path(home.path());
            let mode = std::fs::metadata(&token_path)
                .expect("read token file metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "rotated token file must be 0600, got {mode:o}");
        }
    }

    /// Thermo MUST (rotate env-guard): rotating the token FILE while the
    /// configured env var override (`[webui].env_token_var`, default
    /// `IRONCLAW_REBORN_WEBUI_TOKEN`) is set and non-empty would silently
    /// do nothing at runtime — `serve` reads the env var first (see
    /// `webui_token::resolve_webui_token`'s precedence) and never notices
    /// the rotated file. Rotation must refuse instead of silently no-op'ing.
    ///
    /// Also pins the two edges of `configured_webui_token_env_override`'s
    /// `.filter(|value| !value.is_empty())` check: an env var set to the
    /// empty string counts as "unset" (rotate allowed), while a
    /// whitespace-only value is non-empty and still counts as "set" (rotate
    /// refused) — `set_var("", ...)`-style ambiguity is exactly what the
    /// `is_empty()` filter (not e.g. `trim().is_empty()`) resolves.
    #[test]
    fn webui_token_rotate_refuses_when_env_override_is_set() {
        let _lock = crate::runtime::test_env::lock_runtime_env();
        let _original_env =
            crate::runtime::test_env::EnvGuard::clear("IRONCLAW_REBORN_WEBUI_TOKEN");
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("mkdir");
        crate::webui_token::rotate_webui_token_file(home.path()).expect("seed a valid token");
        let token_path = crate::webui_token::webui_token_file_path(home.path());
        let before = std::fs::read_to_string(&token_path).expect("read seeded token file");

        let result = {
            let _env = crate::runtime::test_env::EnvGuard::set(
                "IRONCLAW_REBORN_WEBUI_TOKEN",
                "operator-set-env-token-value",
            );
            execute_webui_token(&context, None, true)
        };

        let err = result.expect_err("rotate must refuse while the env var overrides the token");
        assert!(
            err.to_string().contains("IRONCLAW_REBORN_WEBUI_TOKEN"),
            "error must name the overriding env var: {err}"
        );
        let after = std::fs::read_to_string(&token_path).expect("read token file again");
        assert_eq!(
            before, after,
            "the token file must not be rotated while the env override is active"
        );

        // Edge: empty string == unset — rotate must be ALLOWED.
        let result = {
            let _env = crate::runtime::test_env::EnvGuard::set("IRONCLAW_REBORN_WEBUI_TOKEN", "");
            execute_webui_token(&context, None, true)
        };
        result.expect("empty env var must count as unset; rotate must be allowed");
        let after_empty = std::fs::read_to_string(&token_path).expect("read token file again");
        assert_ne!(
            after, after_empty,
            "rotate must actually have run with an empty env var override"
        );

        // Edge: whitespace-only value is non-empty — rotate must be REFUSED.
        let result = {
            let _env =
                crate::runtime::test_env::EnvGuard::set("IRONCLAW_REBORN_WEBUI_TOKEN", "   ");
            execute_webui_token(&context, None, true)
        };
        let err =
            result.expect_err("whitespace-only env var must still count as set; rotate refused");
        assert!(
            err.to_string().contains("IRONCLAW_REBORN_WEBUI_TOKEN"),
            "error must name the overriding env var: {err}"
        );
        let after_whitespace = std::fs::read_to_string(&token_path).expect("read token file again");
        assert_eq!(
            after_empty, after_whitespace,
            "the token file must not be rotated while a whitespace-only env override is active"
        );
    }

    #[cfg(unix)]
    #[test]
    fn webui_token_rotate_refuses_a_non_utf8_env_override() {
        use std::os::unix::ffi::OsStringExt as _;

        let _lock = crate::runtime::test_env::lock_runtime_env();
        let _original_env =
            crate::runtime::test_env::EnvGuard::clear("IRONCLAW_REBORN_WEBUI_TOKEN");
        // SAFETY: serialized by the canonical env lock and restored by the
        // guard above. The point of this regression is the non-UTF-8 value.
        unsafe {
            std::env::set_var(
                "IRONCLAW_REBORN_WEBUI_TOKEN",
                std::ffi::OsString::from_vec(vec![0xff, 0xfe]),
            );
        }

        let (_tmp, context) = RebornCliContext::test_context();
        let err = execute_webui_token(&context, None, true)
            .expect_err("any non-empty OS env value must block file rotation");
        assert!(err.to_string().contains("IRONCLAW_REBORN_WEBUI_TOKEN"));
    }
}
