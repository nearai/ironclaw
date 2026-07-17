//! Onboarding's LLM-credential provisioning step: prompt for a provider and
//! API key, then persist both — the secret store write lands before the
//! `config.toml` selection (see [`provision_llm_credentials`]'s doc).

use std::path::Path;

use ironclaw_reborn_config::RebornHome;

use super::prompts::{LlmCredentialPromptError, PromptSource};

/// Outcome of onboard's LLM provider/API-key prompt step. Every variant is a
/// successful `execute()` (exit 0) — mirrors [`super::master_key::MasterKeyProvisionOutcome`]'s
/// shape: `SkippedNonInteractive` is expected and normal, not a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LlmCredentialProvisionOutcome {
    Configured {
        provider_id: String,
        model: String,
    },
    /// `[llm.default]` was already pointed at a provider AND the encrypted
    /// secret store already has a key for it (see
    /// [`already_configured_outcome`]) — this run skipped prompting
    /// entirely rather than re-asking for credentials that are already
    /// durably stored.
    AlreadyConfigured {
        provider_id: String,
        model: String,
    },
    SkippedNonInteractive,
}

impl LlmCredentialProvisionOutcome {
    pub(crate) fn display_line(&self) -> String {
        match self {
            Self::Configured { provider_id, model } => {
                format!("configured provider `{provider_id}` (model `{model}`)")
            }
            Self::AlreadyConfigured { provider_id, model } => {
                format!(
                    "already configured (provider `{provider_id}`, model `{model}`); use \
                     --force to reconfigure"
                )
            }
            Self::SkippedNonInteractive => "skipped (non-interactive session)".to_string(),
        }
    }
}

/// Where [`provision_llm_credentials`] gets its (already-open) encrypted
/// secret store from. Injected — mirrors [`PromptSource`] — so a test can
/// supply a store whose `put` fails, proving the write-ordering guarantee
/// (secret stored before config is written; see
/// [`provision_llm_credentials`]'s doc) without touching the real
/// local-dev libsql-backed store.
///
/// Gated with the same `libsql`+`root-llm-provider` cfg as
/// `ironclaw_reborn_composition::LlmKeyStore` itself: that type (and
/// `open_local_dev_secret_store`) only exists behind those features, so this
/// trait's return type can't compile without them. See the `#[cfg(not(...))]`
/// stub below for the feature-off case.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
pub(crate) trait LlmKeyStoreOpener {
    fn open(&self, home_path: &Path) -> anyhow::Result<ironclaw_reborn_composition::LlmKeyStore>;
}

/// Production [`LlmKeyStoreOpener`]: opens the real local-dev encrypted
/// secret store `serve` later reads from (see
/// `ironclaw_reborn_composition::open_local_dev_secret_store`'s doc for why
/// this is the same physical storage `serve` opens).
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
pub(crate) struct LocalDevLlmKeyStoreOpener;

#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
impl LlmKeyStoreOpener for LocalDevLlmKeyStoreOpener {
    fn open(&self, home_path: &Path) -> anyhow::Result<ironclaw_reborn_composition::LlmKeyStore> {
        let home_path = home_path.to_path_buf();
        crate::runtime::block_on_cli(async move {
            let store = ironclaw_reborn_composition::open_local_dev_secret_store(&home_path)
                .await
                .map_err(anyhow::Error::from)?;
            Ok::<_, anyhow::Error>(ironclaw_reborn_composition::LlmKeyStore::new(store))
        })
    }
}

/// Feature-off stub for [`LlmKeyStoreOpener`]/[`LocalDevLlmKeyStoreOpener`]:
/// without both `libsql` and `root-llm-provider` there is no `LlmKeyStore`
/// type to open at all. This stub exists solely so `execute()`'s
/// unconditional `&LocalDevLlmKeyStoreOpener` call site compiles across every
/// feature combination — the feature-off `provision_llm_credentials` below
/// ignores its `store_opener` parameter entirely, so `open` is never called.
#[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
pub(crate) trait LlmKeyStoreOpener {
    fn open(&self, home_path: &Path) -> anyhow::Result<()>;
}

#[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
pub(crate) struct LocalDevLlmKeyStoreOpener;

#[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
impl LlmKeyStoreOpener for LocalDevLlmKeyStoreOpener {
    fn open(&self, _home_path: &Path) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Prompt for an LLM provider (via the numbered menu — see
/// [`super::prompts::PromptSource::provider_menu`]), its API key when the
/// selected provider requires one, and a model override, then persist what's
/// needed. Both prompts (`api_key`, `model`) run first — they're pure reads
/// of user input with nothing durable behind them — so every fallible step
/// remaining once a write starts is a write itself, never a prompt: any API
/// key value goes into the encrypted secret store via the canonical
/// `LlmKeyStore` handle (`llm_provider_<id>_api_key`) FIRST — the same handle
/// the webui2 settings surface writes and `apply_startup_stored_llm_key`
/// reads at boot — and only once that succeeds does the provider selection
/// land in `[llm.default]` in `config.toml` SECOND (existing
/// `RebornProviderAdmin::set_provider` config machinery, the same one
/// `ironclaw-reborn models set-provider` uses). This ordering means
/// `config.toml` can never point at a provider whose key failed to persist
/// durably: a `LlmKeyStore::put` failure returns an error before
/// `set_provider` is ever called, leaving `config.toml` exactly as it was —
/// and, symmetrically, a prompt failure (e.g. Ctrl-D on the model prompt)
/// can never leave an orphan key in the secret store, because no prompt runs
/// after the store write starts. A provider whose menu entry has
/// `api_key_required: false` (e.g. `nearai`) skips the key prompt and
/// secret-store write entirely — there is nothing to persist there.
///
/// Skips prompting entirely (an idempotent no-op) on a rerun where
/// `[llm.default]` is already user-configured AND (the provider doesn't
/// require a key OR the store already has a key for it), unless `force` is
/// set — see [`already_configured_outcome`].
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
pub(crate) fn provision_llm_credentials(
    home: &RebornHome,
    boot: &ironclaw_reborn_config::RebornBootConfig,
    prompts: &mut dyn PromptSource,
    store_opener: &dyn LlmKeyStoreOpener,
    force: bool,
) -> Result<LlmCredentialProvisionOutcome, LlmCredentialPromptError> {
    if !prompts.is_interactive() {
        return Err(LlmCredentialPromptError::NonInteractive);
    }

    let admin = ironclaw_reborn_composition::RebornProviderAdmin::new(boot.clone());

    if !force && let Some(outcome) = already_configured_outcome(&admin, home, store_opener)? {
        return Ok(outcome);
    }

    let entries = admin
        .menu_entries()
        .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
    let selection = prompts.provider_menu(&entries)?;
    // Canonical second check: the menu already only offers canonical ids,
    // but re-resolving here keeps this call site's provider id agreeing
    // with `set_provider`'s own resolution, exactly like the pre-menu code
    // did for its free-text answer.
    let canonical_provider_id = admin
        .resolve_provider_id(&selection)
        .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
    let entry = entries
        .iter()
        .find(|entry| entry.id == canonical_provider_id)
        .ok_or_else(|| {
            LlmCredentialPromptError::Other(anyhow::anyhow!(
                "selected provider `{canonical_provider_id}` is not on the onboarding menu"
            ))
        })?;

    // Both prompts run BEFORE any write: they're pure reads of user input,
    // so gathering them first means the only fallible steps left are the
    // two durable writes below (store, then config) — no prompt can fail
    // partway through with a secret already committed. See this function's
    // doc for the store-then-config write ordering.
    let key = if entry.api_key_required {
        let key = prompts.api_key(&canonical_provider_id)?;
        // Defense in depth: `StdinPromptSource::api_key` already re-prompts
        // on a blank answer, but this guards every `PromptSource`
        // implementation — present or future — so a blank key can never
        // reach the secret store regardless of where it slipped through.
        if key.trim().is_empty() {
            return Err(LlmCredentialPromptError::Other(anyhow::anyhow!(
                "LLM API key must not be blank"
            )));
        }
        Some(key)
    } else {
        None
    };

    let default_model = admin
        .list(Some(&canonical_provider_id), false)
        .map_err(|error| LlmCredentialPromptError::Other(error.into()))?
        .providers
        .into_iter()
        .next()
        .map(|info| info.default_model)
        .unwrap_or_default();
    let model = prompts.model(&canonical_provider_id, &default_model)?;

    if let Some(key) = key {
        let store = store_opener
            .open(home.path())
            .map_err(LlmCredentialPromptError::Other)?;
        let provider_id_for_store = canonical_provider_id.clone();
        crate::runtime::block_on_cli(async move {
            store
                .put_plaintext(&provider_id_for_store, key)
                .await
                .map_err(anyhow::Error::from)
        })
        .map_err(LlmCredentialPromptError::Other)?;
    }

    let write_outcome = admin
        .set_provider(&canonical_provider_id, model.as_deref())
        .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;

    Ok(LlmCredentialProvisionOutcome::Configured {
        provider_id: write_outcome.provider_id,
        model: write_outcome.model,
    })
}

/// `Some` when `[llm.default]` already names a provider AND that provider is
/// durably credentialed — either it doesn't require an API key at all (e.g.
/// `nearai`), or the encrypted secret store already has a key stored for it
/// — the idempotent-rerun case [`provision_llm_credentials`] must skip
/// prompting for (a bare stub-seeded `[llm.default]` with no stored key for
/// a key-requiring provider, e.g. right after a fresh `onboard` on a
/// headless box, does NOT count: that provider has never actually been
/// credentialed, so a later interactive rerun must still prompt).
///
/// Before this fix, a no-key provider like `nearai` was *never* recognized
/// as already-configured (the store-key check always came back empty for
/// it), so every rerun re-prompted even though there was nothing left to
/// configure — see the regression test
/// `provision_llm_credentials_is_idempotent_for_a_no_key_provider`.
///
/// A store-open failure is treated as "can't tell" and falls through to
/// prompting rather than erroring the whole run (a fresh prompt/write can
/// still succeed even if the pre-flight check couldn't read the store). A
/// registry lookup failure, in contrast, is propagated — see
/// [`provider_api_key_required`]'s doc.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn already_configured_outcome(
    admin: &ironclaw_reborn_composition::RebornProviderAdmin,
    home: &RebornHome,
    store_opener: &dyn LlmKeyStoreOpener,
) -> Result<Option<LlmCredentialProvisionOutcome>, LlmCredentialPromptError> {
    let status = admin
        .status()
        .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
    let Some(selection) = status.default else {
        return Ok(None);
    };
    let Some(provider_id) = selection.provider_id else {
        return Ok(None);
    };

    let Some(api_key_required) = provider_api_key_required(admin, &provider_id)? else {
        return Ok(None);
    };
    if !api_key_required {
        return Ok(Some(LlmCredentialProvisionOutcome::AlreadyConfigured {
            provider_id,
            model: selection.model.unwrap_or_default(),
        }));
    }

    let store = match store_opener.open(home.path()) {
        Ok(store) => store,
        Err(error) => {
            tracing::debug!(
                %error,
                "secret store open failed while checking already-configured LLM; falling \
                 through to prompt"
            );
            return Ok(None);
        }
    };
    let provider_id_for_check = provider_id.clone();
    let has_key = crate::runtime::block_on_cli(async move {
        store
            .exists(&provider_id_for_check)
            .await
            .map_err(anyhow::Error::from)
    })
    .map_err(LlmCredentialPromptError::Other)?;
    if !has_key {
        return Ok(None);
    }
    Ok(Some(LlmCredentialProvisionOutcome::AlreadyConfigured {
        provider_id,
        model: selection.model.unwrap_or_default(),
    }))
}

/// Whether `provider_id` requires an API key, per the full provider
/// registry (not menu-restricted — `[llm.default]` may name a provider
/// that's excluded from the onboard menu, e.g. one set via `models
/// set-provider`).
///
/// Returns `Err` when the registry lookup itself fails (e.g. a corrupt or
/// unreadable `providers.json`) — that's a real failure, not "unconfigured",
/// and must not be swallowed into a silent re-prompt (a swallowed registry
/// error here would make `already_configured_outcome` treat a broken
/// registry the same as "never configured", re-running the interactive
/// prompt — and on `--force`, re-writing credentials — every time). Returns
/// `Ok(None)` only for the genuinely "can't tell" case: `provider_id` isn't
/// in the registry's list result, or has no setup metadata attached.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn provider_api_key_required(
    admin: &ironclaw_reborn_composition::RebornProviderAdmin,
    provider_id: &str,
) -> Result<Option<bool>, LlmCredentialPromptError> {
    let providers = admin
        .list(Some(provider_id), true)
        .map_err(|error| LlmCredentialPromptError::Other(error.into()))?
        .providers;
    Ok(providers
        .into_iter()
        .next()
        .and_then(|info| info.metadata)
        .map(|metadata| metadata.api_key_required))
}

/// Without both `libsql` (the store opener) and `root-llm-provider`
/// (`RebornProviderAdmin`/`LlmKeyStore`) the LLM credential step has nothing
/// to write to — same reasoning as `provision_master_key`'s
/// not-any-storage-feature fallback above.
#[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
pub(crate) fn provision_llm_credentials(
    _home: &RebornHome,
    _boot: &ironclaw_reborn_config::RebornBootConfig,
    _prompts: &mut dyn PromptSource,
    _store_opener: &dyn LlmKeyStoreOpener,
    _force: bool,
) -> Result<LlmCredentialProvisionOutcome, LlmCredentialPromptError> {
    Ok(LlmCredentialProvisionOutcome::SkippedNonInteractive)
}

#[cfg(all(test, feature = "libsql", feature = "root-llm-provider"))]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::context::RebornCliContext;

    /// Selects `provider` on the menu (matched by id), answers `key` for the
    /// API-key prompt (only reached when the selected entry requires one),
    /// and answers `model` for the model prompt (`None` means an empty
    /// answer — use the catalog default).
    struct FakePromptSource {
        provider: &'static str,
        key: &'static str,
        model: Option<&'static str>,
    }

    impl PromptSource for FakePromptSource {
        fn is_interactive(&self) -> bool {
            true
        }

        fn provider_menu(
            &mut self,
            entries: &[ironclaw_reborn_composition::ProviderMenuEntry],
        ) -> Result<String, LlmCredentialPromptError> {
            entries
                .iter()
                .find(|entry| entry.id == self.provider)
                .map(|entry| entry.id.clone())
                .ok_or_else(|| {
                    LlmCredentialPromptError::Other(anyhow::anyhow!(
                        "fake-selected provider `{}` is not on the menu",
                        self.provider
                    ))
                })
        }

        fn api_key(&mut self, _provider: &str) -> Result<String, LlmCredentialPromptError> {
            Ok(self.key.to_string())
        }

        fn model(
            &mut self,
            _provider_id: &str,
            _default_model: &str,
        ) -> Result<Option<String>, LlmCredentialPromptError> {
            Ok(self.model.map(str::to_string))
        }
    }

    struct NonInteractivePromptSource;

    impl PromptSource for NonInteractivePromptSource {
        fn is_interactive(&self) -> bool {
            false
        }

        fn provider_menu(
            &mut self,
            _entries: &[ironclaw_reborn_composition::ProviderMenuEntry],
        ) -> Result<String, LlmCredentialPromptError> {
            unreachable!("provider_menu() must not be called once is_interactive() is false")
        }

        fn api_key(&mut self, _provider: &str) -> Result<String, LlmCredentialPromptError> {
            unreachable!("api_key must not be prompted once provider_menu() has already failed")
        }

        fn model(
            &mut self,
            _provider_id: &str,
            _default_model: &str,
        ) -> Result<Option<String>, LlmCredentialPromptError> {
            unreachable!("model must not be prompted once provider_menu() has already failed")
        }
    }

    /// A [`PromptSource`] whose prompt methods panic if called — used to
    /// prove an idempotent rerun (item 3) skips prompting entirely rather
    /// than merely tolerating a repeated answer.
    struct PanickingPromptSource;

    impl PromptSource for PanickingPromptSource {
        fn is_interactive(&self) -> bool {
            true
        }

        fn provider_menu(
            &mut self,
            _entries: &[ironclaw_reborn_composition::ProviderMenuEntry],
        ) -> Result<String, LlmCredentialPromptError> {
            panic!("provider_menu() must not be called on an idempotent, already-configured rerun")
        }

        fn api_key(&mut self, _provider: &str) -> Result<String, LlmCredentialPromptError> {
            panic!("api_key() must not be called on an idempotent, already-configured rerun")
        }

        fn model(
            &mut self,
            _provider_id: &str,
            _default_model: &str,
        ) -> Result<Option<String>, LlmCredentialPromptError> {
            panic!("model() must not be called on an idempotent, already-configured rerun")
        }
    }

    /// A [`LlmKeyStoreOpener`] whose store's `put` always fails — used to
    /// prove `provision_llm_credentials` writes the secret store BEFORE
    /// `config.toml`: a `put` failure must leave `config.toml` untouched.
    struct FailingLlmKeyStoreOpener;

    impl LlmKeyStoreOpener for FailingLlmKeyStoreOpener {
        fn open(
            &self,
            _home_path: &Path,
        ) -> anyhow::Result<ironclaw_reborn_composition::LlmKeyStore> {
            Ok(ironclaw_reborn_composition::LlmKeyStore::new(Arc::new(
                FailingSecretStore,
            )))
        }
    }

    struct FailingSecretStore;

    #[async_trait::async_trait]
    impl ironclaw_secrets::SecretStore for FailingSecretStore {
        async fn put(
            &self,
            _scope: ironclaw_host_api::ResourceScope,
            _handle: ironclaw_host_api::SecretHandle,
            _material: ironclaw_secrets::SecretMaterial,
            _expires_at: Option<ironclaw_host_api::Timestamp>,
        ) -> Result<ironclaw_secrets::SecretMetadata, ironclaw_secrets::SecretStoreError> {
            Err(ironclaw_secrets::SecretStoreError::StoreUnavailable {
                reason: "simulated failure for write-ordering RED test".to_string(),
            })
        }

        async fn metadata(
            &self,
            _scope: &ironclaw_host_api::ResourceScope,
            _handle: &ironclaw_host_api::SecretHandle,
        ) -> Result<Option<ironclaw_secrets::SecretMetadata>, ironclaw_secrets::SecretStoreError>
        {
            unreachable!("not exercised by provision_llm_credentials")
        }

        async fn metadata_for_scope(
            &self,
            _scope: &ironclaw_host_api::ResourceScope,
        ) -> Result<Vec<ironclaw_secrets::SecretMetadata>, ironclaw_secrets::SecretStoreError>
        {
            unreachable!("not exercised by provision_llm_credentials")
        }

        async fn delete(
            &self,
            _scope: &ironclaw_host_api::ResourceScope,
            _handle: &ironclaw_host_api::SecretHandle,
        ) -> Result<bool, ironclaw_secrets::SecretStoreError> {
            unreachable!("not exercised by provision_llm_credentials")
        }

        async fn lease_once(
            &self,
            _scope: &ironclaw_host_api::ResourceScope,
            _handle: &ironclaw_host_api::SecretHandle,
        ) -> Result<ironclaw_secrets::SecretLease, ironclaw_secrets::SecretStoreError> {
            unreachable!("not exercised by provision_llm_credentials")
        }

        async fn consume(
            &self,
            _scope: &ironclaw_host_api::ResourceScope,
            _lease_id: ironclaw_secrets::SecretLeaseId,
        ) -> Result<ironclaw_secrets::SecretMaterial, ironclaw_secrets::SecretStoreError> {
            unreachable!("not exercised by provision_llm_credentials")
        }

        async fn revoke(
            &self,
            _scope: &ironclaw_host_api::ResourceScope,
            _lease_id: ironclaw_secrets::SecretLeaseId,
        ) -> Result<ironclaw_secrets::SecretLease, ironclaw_secrets::SecretStoreError> {
            unreachable!("not exercised by provision_llm_credentials")
        }

        async fn leases_for_scope(
            &self,
            _scope: &ironclaw_host_api::ResourceScope,
        ) -> Result<Vec<ironclaw_secrets::SecretLease>, ironclaw_secrets::SecretStoreError>
        {
            unreachable!("not exercised by provision_llm_credentials")
        }
    }

    /// Seed a cached master-key dotfile so the real local-dev store opener's
    /// resolver never reaches the OS keychain step in a test — see
    /// `ironclaw_reborn_composition::factory`'s
    /// `open_local_dev_secret_store_opens_a_working_store_over_the_bare_root`
    /// for the same seeding pattern.
    fn seed_cached_master_key(home: &RebornHome) {
        std::fs::write(
            home.path()
                .join(ironclaw_reborn_composition::LOCAL_DEV_SECRETS_MASTER_KEY_PATH),
            ironclaw_secrets::keychain::generate_master_key_hex(),
        )
        .expect("seed cached master key");
    }

    /// A [`PromptSource`] that selects `provider` on the menu and answers
    /// `model`, but panics if `api_key()` is ever called — used to prove a
    /// no-API-key provider (e.g. `nearai`) never reaches the key prompt.
    struct KeyPanicsIfCalledPromptSource {
        provider: &'static str,
        model: Option<&'static str>,
    }

    impl PromptSource for KeyPanicsIfCalledPromptSource {
        fn is_interactive(&self) -> bool {
            true
        }

        fn provider_menu(
            &mut self,
            entries: &[ironclaw_reborn_composition::ProviderMenuEntry],
        ) -> Result<String, LlmCredentialPromptError> {
            entries
                .iter()
                .find(|entry| entry.id == self.provider)
                .map(|entry| entry.id.clone())
                .ok_or_else(|| {
                    LlmCredentialPromptError::Other(anyhow::anyhow!(
                        "fake-selected provider `{}` is not on the menu",
                        self.provider
                    ))
                })
        }

        fn api_key(&mut self, _provider: &str) -> Result<String, LlmCredentialPromptError> {
            panic!("api_key() must not be called for a provider that doesn't require one")
        }

        fn model(
            &mut self,
            _provider_id: &str,
            _default_model: &str,
        ) -> Result<Option<String>, LlmCredentialPromptError> {
            Ok(self.model.map(str::to_string))
        }
    }

    /// RED (B2 step 1, adapted for the menu): a fake interactive
    /// `PromptSource` selecting `openai` (a key-requiring menu entry) and
    /// answering `"sk-test-value"` must land the provider selection in
    /// `config.toml` and the key value in the encrypted secret store,
    /// readable back through a *fresh* open of the same root — proving the
    /// opener and `LlmKeyStore::put`/`read` agree on physical storage.
    ///
    /// Also proves item 3's idempotent-rerun guard for a key-requiring
    /// provider: a second call with a `PanickingPromptSource` (whose prompt
    /// methods panic if invoked) must return `AlreadyConfigured` without
    /// ever calling `provider_menu()`/`api_key()` — proving the rerun is
    /// skipped, not merely tolerated.
    #[test]
    fn provision_llm_credentials_writes_config_and_secret_store_through_fake_prompts() {
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        seed_cached_master_key(home);

        let mut prompts = FakePromptSource {
            provider: "openai",
            key: "sk-test-value",
            model: None,
        };
        let outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            false,
        )
        .expect("provision must succeed with a fake interactive source");
        assert_eq!(
            outcome,
            LlmCredentialProvisionOutcome::Configured {
                provider_id: "openai".to_string(),
                model: "gpt-5-mini".to_string(),
            }
        );

        let home_path = home.path().to_path_buf();
        let stored = crate::runtime::block_on_cli(async move {
            let store = ironclaw_reborn_composition::open_local_dev_secret_store(&home_path)
                .await
                .map_err(anyhow::Error::from)?;
            ironclaw_reborn_composition::LlmKeyStore::new(store)
                .read("openai")
                .await
                .map_err(anyhow::Error::from)
        })
        .expect("read back through a fresh open of the same root");
        let material = stored.expect("a value must have been written");
        assert_eq!(
            secrecy::ExposeSecret::expose_secret(&material),
            "sk-test-value"
        );

        let config_text =
            std::fs::read_to_string(home.config_file_path()).expect("read config.toml");
        assert!(
            config_text.contains("provider_id = \"openai\""),
            "config.toml: {config_text}"
        );

        // Item 3: a rerun with an already-configured provider + stored key
        // must skip prompting entirely.
        let mut second_prompts = PanickingPromptSource;
        let second_outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut second_prompts,
            &LocalDevLlmKeyStoreOpener,
            false,
        )
        .expect("an idempotent rerun must succeed without prompting");
        assert_eq!(
            second_outcome,
            LlmCredentialProvisionOutcome::AlreadyConfigured {
                provider_id: "openai".to_string(),
                model: "gpt-5-mini".to_string(),
            }
        );
    }

    /// RED (thermo-caught idempotency bug): a no-API-key provider (`nearai`)
    /// must skip the key prompt and secret-store write entirely on first
    /// run, AND a rerun must recognize it as already configured without
    /// prompting — before this fix, `already_configured_outcome` only ever
    /// looked at the store for a stored key, so a no-key provider was
    /// *never* recognized as configured and every `onboard` rerun
    /// re-prompted even though there was nothing left to configure.
    #[test]
    fn provision_llm_credentials_is_idempotent_for_a_no_key_provider() {
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        seed_cached_master_key(home);

        let mut prompts = KeyPanicsIfCalledPromptSource {
            provider: "nearai",
            model: None,
        };
        let outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            false,
        )
        .expect("provision must succeed without ever calling api_key()");
        assert_eq!(
            outcome,
            LlmCredentialProvisionOutcome::Configured {
                provider_id: "nearai".to_string(),
                model: "deepseek-ai/DeepSeek-V4-Flash".to_string(),
            }
        );

        let home_path = home.path().to_path_buf();
        let has_key = crate::runtime::block_on_cli(async move {
            let store = ironclaw_reborn_composition::open_local_dev_secret_store(&home_path)
                .await
                .map_err(anyhow::Error::from)?;
            ironclaw_reborn_composition::LlmKeyStore::new(store)
                .exists("nearai")
                .await
                .map_err(anyhow::Error::from)
        })
        .expect("read back through a fresh open of the same root");
        assert!(
            !has_key,
            "a provider that doesn't require an API key must never write to the secret store"
        );

        // The regression this test pins: a rerun must recognize `nearai` as
        // already configured — with no stored key to check, the old
        // `already_configured_outcome` fell through to prompting every
        // time.
        let mut second_prompts = PanickingPromptSource;
        let second_outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut second_prompts,
            &LocalDevLlmKeyStoreOpener,
            false,
        )
        .expect("an idempotent rerun must succeed without prompting");
        assert_eq!(
            second_outcome,
            LlmCredentialProvisionOutcome::AlreadyConfigured {
                provider_id: "nearai".to_string(),
                model: "deepseek-ai/DeepSeek-V4-Flash".to_string(),
            }
        );
    }

    /// RED (B2 step 2): a non-interactive fake source must surface as a
    /// typed [`LlmCredentialPromptError::NonInteractive`] — never a panic or
    /// process exit — and must not write anything: `provider_menu()`/
    /// `api_key()` are `unreachable!()` (proving the interactivity check
    /// short-circuits before either prompt runs) and `config.toml` must not
    /// exist afterward (proving no store/config touch happens before both
    /// prompts have succeeded).
    #[test]
    fn provision_llm_credentials_propagates_non_interactive_error_without_touching_anything() {
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");

        let mut prompts = NonInteractivePromptSource;
        let error = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            false,
        )
        .expect_err("a non-interactive source must return a typed error");
        assert!(matches!(error, LlmCredentialPromptError::NonInteractive));
        assert!(
            !home.config_file_path().exists(),
            "a non-interactive prompt failure must not write config.toml"
        );
    }

    /// RED (item 2, write ordering): a store whose `put` always fails must
    /// leave `config.toml` completely untouched — proving the secret is
    /// written BEFORE the provider selection, not after. Under the old
    /// ordering (config first, store second) `config.toml` would already
    /// carry `provider_id = "openai"` by the time the store write failed.
    /// Uses `openai` (key-requiring), not `nearai` — a no-key provider never
    /// opens the store at all, so it couldn't exercise this ordering.
    #[test]
    fn provision_llm_credentials_leaves_config_untouched_when_the_store_put_fails() {
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");

        let mut prompts = FakePromptSource {
            provider: "openai",
            key: "sk-test-value",
            model: None,
        };
        let error = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &FailingLlmKeyStoreOpener,
            false,
        )
        .expect_err("a failing store put must surface as an error");
        assert!(matches!(error, LlmCredentialPromptError::Other(_)));
        assert!(
            !home.config_file_path().exists(),
            "a failed key-store write must leave config.toml untouched — store first, config \
             second"
        );
    }

    /// RED (round-2 review item 3): a `PromptSource` whose `api_key()`
    /// returns a whitespace-only answer (e.g. a fake standing in for an
    /// implementation that didn't get the blank-rejection retry loop
    /// `StdinPromptSource::api_key` has) must never reach the secret store —
    /// `provision_llm_credentials`'s own blank guard is the backstop for
    /// every `PromptSource`, not just the terminal-backed one.
    #[test]
    fn provision_llm_credentials_rejects_a_blank_api_key_without_touching_anything() {
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        seed_cached_master_key(home);

        let mut prompts = FakePromptSource {
            provider: "openai",
            key: "   ",
            model: None,
        };
        let error = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            false,
        )
        .expect_err("a blank API key must be rejected");
        assert!(matches!(error, LlmCredentialPromptError::Other(_)));
        assert!(
            !home.config_file_path().exists(),
            "a rejected blank API key must leave config.toml untouched"
        );
    }

    /// (v) An excluded provider id typed at the menu (not a menu entry —
    /// `ollama`/`bedrock`/etc are excluded by `menu_entries()` by design)
    /// must be rejected as invalid, never resolved via the full registry.
    /// Covers `bedrock` (onboarding-scope exclusion) and `openai_compatible`
    /// (base-URL-trap exclusion — see `RebornProviderAdmin::menu_entries`'s
    /// doc): both are real, resolvable registry providers, so this pins
    /// that menu exclusion — not registry absence — is what blocks them.
    #[test]
    fn provision_llm_credentials_rejects_a_menu_excluded_provider_id() {
        for excluded_provider in ["bedrock", "openai_compatible"] {
            let (_tmp, context) = RebornCliContext::test_context();
            let home = context.boot_config().home();
            std::fs::create_dir_all(home.path()).expect("create reborn home");

            let mut prompts = FakePromptSource {
                provider: excluded_provider,
                key: "unused",
                model: None,
            };
            let error = provision_llm_credentials(
                home,
                context.boot_config(),
                &mut prompts,
                &LocalDevLlmKeyStoreOpener,
                false,
            )
            .expect_err(&format!(
                "menu-excluded provider `{excluded_provider}` must be rejected"
            ));
            assert!(matches!(error, LlmCredentialPromptError::Other(_)));
            assert!(
                !home.config_file_path().exists(),
                "a rejected menu selection ({excluded_provider}) must leave config.toml untouched"
            );
        }
    }

    /// (iv) An empty model answer must land the catalog default in
    /// `[llm.default].model`, not a blank string.
    #[test]
    fn provision_llm_credentials_empty_model_answer_uses_catalog_default() {
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        seed_cached_master_key(home);

        let mut prompts = FakePromptSource {
            provider: "openai",
            key: "sk-test-value",
            model: None,
        };
        let outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            false,
        )
        .expect("provision must succeed");
        assert_eq!(
            outcome,
            LlmCredentialProvisionOutcome::Configured {
                provider_id: "openai".to_string(),
                model: "gpt-5-mini".to_string(),
            },
            "an empty model answer must resolve to openai's catalog default model"
        );
    }
}
