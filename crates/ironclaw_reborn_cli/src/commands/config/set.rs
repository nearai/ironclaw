//! `ironclaw-reborn config set <key> [value]` — write one config value,
//! routed through `capability_config::ConfigKey`'s alias table so the
//! destination (config.toml literal / encrypted secret store / token
//! file), shape validation, and secret-vs-non-secret refusal all come
//! from that single chokepoint.

use std::io::{IsTerminal, Write as _};
use std::path::Path;
use std::sync::Arc;

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
    /// Value to set. Omit for a secret-destination key (`<provider>.api_key`,
    /// `google.client_secret`) to be prompted for instead, with input hidden.
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
         slack.enabled, webui.token (--rotate only)\nRun `ironclaw-reborn config list` to see \
         all readable keys",
        super::capability_config::DEFAULT_LLM_API_KEY_PROVIDER,
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
    let value = match value_arg {
        Some(value) => value,
        None if key.is_secret_prompted() => prompt.prompt(&label)?,
        None => anyhow::bail!("`config set {label}` requires a value"),
    };

    match validate_shape(&key, &value) {
        ShapeVerdict::Reject(message) => anyhow::bail!(message),
        ShapeVerdict::Warn(message) => eprintln!("warning: {message}"),
        ShapeVerdict::Ok => {}
    }

    // Secret-shape law: a value destined for config.toml must not look
    // like inline secret material (the same rule
    // `RebornConfigFile::validate` enforces on parse, applied here before
    // any write so the refusal happens at the CLI layer, not buried in a
    // toml_edit write-then-validate round trip).
    if key.destination() == ConfigDestination::ConfigToml
        && let Err(error) = ironclaw_reborn_config::reject_inline_secret(label.clone(), &value)
    {
        anyhow::bail!("refusing to write `{label}` to config.toml: {error}");
    }

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
            write_google_client_secret(home, &value, store_opener)?;
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
/// capability this key belongs to — the same
/// `capability_config::google_remediation_text`/`slack_remediation_text`
/// strings a later capability-requirements pass is expected to reuse for
/// "not configured" tool-result errors (see `capability_config`'s module
/// doc for that follow-up's cross-crate caveat).
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
    let store = store_opener.open(context.boot_config().home().path())?;
    let value_owned = value.to_string();
    crate::runtime::block_on_cli(async move {
        ironclaw_reborn_composition::LlmKeyStore::new(store)
            .put(
                &canonical_provider_id,
                ironclaw_secrets::SecretMaterial::from(value_owned),
            )
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
    home: &RebornHome,
    value: &str,
    store_opener: &dyn SecretStoreOpener,
) -> anyhow::Result<()> {
    let store = store_opener.open(home.path())?;
    let value_owned = value.to_string();
    crate::runtime::block_on_cli(async move {
        ironclaw_reborn_composition::GoogleOauthSecretStore::new(store)
            .put(ironclaw_secrets::SecretMaterial::from(value_owned))
            .await
            .map_err(anyhow::Error::from)
    })
}

#[cfg(not(feature = "libsql"))]
fn write_google_client_secret(
    _home: &RebornHome,
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
/// process kill itself mid-turn. A running `ironclaw-reborn` service (or a
/// manually run `serve`) keeps serving the old value until this step is
/// run — one explicit, unconditional line for every caller (human, an
/// agent's own shell tool, a script) rather than branching on TTY-ness or
/// service state.
fn print_apply_step() {
    println!("  to apply: ironclaw-reborn service restart");
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
    println!(
        "warning: rotating the WebChat v2 token invalidates every existing browser session \
         (the token also signs sessions) — anyone signed in will need to sign in again"
    );
    let home = context.boot_config().home();
    crate::webui_token::rotate_webui_token_file(home.path())?;
    println!("webui.token: rotated");
    print_apply_step();
    // The token file on disk is already rotated, but a running `serve`
    // process keeps the old token in memory until it restarts — so the
    // login link below (already built from the new token) only becomes
    // valid once `ironclaw-reborn service restart` (printed above) runs.
    #[cfg(feature = "webui-v2-beta")]
    if let Some(link) = crate::webui_token::login_link(home) {
        println!("login_link: {link} (valid after restart)");
    }
    Ok(())
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
                "`config set {label}` needs a value; pass it as an argument, or run from an \
                 interactive terminal to be prompted with input hidden"
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
/// from. Mirrors `onboard::LlmKeyStoreOpener` — kept as a distinct trait
/// (not reused from `onboard`) since that one is private to the `onboard`
/// module; both production impls open the same physical local-dev store.
#[cfg_attr(not(feature = "libsql"), allow(dead_code))]
trait SecretStoreOpener {
    fn open(&self, home_path: &Path) -> anyhow::Result<Arc<dyn ironclaw_secrets::SecretStore>>;
}

#[cfg_attr(not(feature = "libsql"), allow(dead_code))]
struct LocalDevSecretStoreOpener;

impl SecretStoreOpener for LocalDevSecretStoreOpener {
    #[cfg(feature = "libsql")]
    fn open(&self, home_path: &Path) -> anyhow::Result<Arc<dyn ironclaw_secrets::SecretStore>> {
        let home_path = home_path.to_path_buf();
        crate::runtime::block_on_cli(async move {
            ironclaw_reborn_composition::open_local_dev_secret_store(&home_path)
                .await
                .map_err(anyhow::Error::from)
        })
    }

    #[cfg(not(feature = "libsql"))]
    fn open(&self, _home_path: &Path) -> anyhow::Result<Arc<dyn ironclaw_secrets::SecretStore>> {
        anyhow::bail!("secret-store-backed `config set` keys require the `libsql` Cargo feature")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

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
    }

    #[cfg(feature = "libsql")]
    impl FakeSecretStoreOpener {
        fn new() -> Self {
            Self {
                store: Arc::new(ironclaw_secrets::InMemorySecretStore::new()),
            }
        }
    }

    #[cfg(feature = "libsql")]
    impl SecretStoreOpener for FakeSecretStoreOpener {
        fn open(
            &self,
            _home_path: &Path,
        ) -> anyhow::Result<Arc<dyn ironclaw_secrets::SecretStore>> {
            Ok(self.store.clone())
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
        fn open(
            &self,
            _home_path: &Path,
        ) -> anyhow::Result<Arc<dyn ironclaw_secrets::SecretStore>> {
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

    #[cfg(feature = "libsql")]
    #[test]
    fn google_client_secret_writes_secret_store_only() {
        let (_tmp, context) = RebornCliContext::test_context();
        let opener = FakeSecretStoreOpener::new();

        set_value_key(
            &context,
            ConfigKey::GoogleClientSecret,
            Some("GOCSPX-abc123".to_string()),
            &mut NeverPromptSource,
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

        set_value_key(
            &context,
            ConfigKey::LlmApiKey {
                provider_id: "openai".to_string(),
            },
            Some("sk-test-value-1234567890".to_string()),
            &mut NeverPromptSource,
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
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("mkdir");

        execute_webui_token(&context, None, true).expect("rotate must succeed");

        assert!(crate::webui_token::webui_token_file_is_valid(home.path()));
    }
}
