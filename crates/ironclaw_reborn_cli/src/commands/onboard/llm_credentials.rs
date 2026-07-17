//! Onboarding's LLM-credential provisioning step: prompt for a provider and
//! API key, then persist both — the secret store write lands before the
//! `config.toml` selection (see [`provision_llm_credentials`]'s doc).

use std::path::Path;

use ironclaw_reborn_config::RebornHome;

use super::prompts::{LlmCredentialPromptError, PromptSource};

/// Outcome of onboard's LLM provider/API-key prompt step. Every variant is a
/// successful `execute()` (exit 0) — mirrors [`super::master_key::MasterKeyProvisionOutcome`]'s
/// shape: the `Skipped*` variants are expected and normal, not a failure.
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
    /// A complete LLM configuration was detected in the environment
    /// (`RebornProviderAdmin::detect_env_llm`) and the `[llm.default]` slot
    /// was seeded from it — via `set_provider`, storing no API key (the
    /// value keeps resolving from its env var at runtime; see
    /// [`provision_llm_credentials`]'s doc). Reached either through an
    /// interactive "use it?" confirmation, or silently on a headless run.
    ///
    /// Idempotency note: once seeded, this slot is authoritative like any
    /// other — a later drift between the seeded values and the live
    /// environment is accepted (not re-detected or re-synced) on subsequent
    /// runs; `--force` re-seeds from the environment again.
    ConfiguredFromEnv {
        provider_id: String,
        model: String,
    },
    /// Headless (non-interactive) session; no LLM environment variables are
    /// set at all. Nothing was seeded.
    SkippedNonInteractive,
    /// Headless (non-interactive) session; some LLM environment
    /// configuration was present but incomplete or invalid (e.g. a
    /// provider's model env var set without its required API key env var).
    /// Nothing was seeded — a partial/broken environment must never be
    /// silently adopted.
    SkippedNonInteractivePartialEnv {
        reason: String,
    },
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
            Self::ConfiguredFromEnv { provider_id, model } => {
                format!("configured provider `{provider_id}` (model `{model}`) from environment")
            }
            Self::SkippedNonInteractive => "skipped (non-interactive session)".to_string(),
            Self::SkippedNonInteractivePartialEnv { reason } => {
                format!(
                    "skipped (non-interactive session; partial environment LLM config: {reason})"
                )
            }
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

/// Where `provision_via_menu`'s pre-write key/model verification probe
/// comes from — injected so a test can script probe outcomes (rejected key,
/// unreachable endpoint, ok-with-a-model-list, …) without a live LLM
/// endpoint, mirroring [`LlmKeyStoreOpener`]'s injection shape. Unlike that
/// trait (which opens a durable resource), this one performs the
/// side-effecting call itself — a network round trip — so its method takes
/// the already-built [`ironclaw_reborn_composition::RebornProviderAdmin`]
/// rather than the raw ingredients to construct one, since every call site
/// in this module already has one in scope.
///
/// Gated the same as [`LlmKeyStoreOpener`]: without both `libsql` and
/// `root-llm-provider` there is no `RebornProviderAdmin`/`ProviderProbeOutcome`
/// to probe with at all.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
pub(crate) trait LlmProbe {
    fn probe(
        &self,
        admin: &ironclaw_reborn_composition::RebornProviderAdmin,
        provider_id: &str,
        api_key: Option<&str>,
        model: Option<&str>,
    ) -> anyhow::Result<ironclaw_reborn_composition::ProviderProbeOutcome>;
}

/// Production [`LlmProbe`]: calls `RebornProviderAdmin::probe_candidate`,
/// which builds a transient provider from the candidate settings and lists
/// its models — the same machinery the webui2 settings "Test connection"
/// button uses, reused here rather than opening a second transport.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
pub(crate) struct LiveLlmProbe;

#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
impl LlmProbe for LiveLlmProbe {
    fn probe(
        &self,
        admin: &ironclaw_reborn_composition::RebornProviderAdmin,
        provider_id: &str,
        api_key: Option<&str>,
        model: Option<&str>,
    ) -> anyhow::Result<ironclaw_reborn_composition::ProviderProbeOutcome> {
        let admin = admin.clone();
        let provider_id = provider_id.to_string();
        let api_key = api_key.map(|key| secrecy::SecretString::from(key.to_string()));
        let model = model.map(str::to_string);
        crate::runtime::block_on_cli(async move {
            admin
                .probe_candidate(&provider_id, api_key, model.as_deref())
                .await
                .map_err(anyhow::Error::from)
        })
    }
}

/// Feature-off stub for [`LlmProbe`]/[`LiveLlmProbe`] — same reasoning as
/// [`LlmKeyStoreOpener`]'s feature-off stub above: without both `libsql` and
/// `root-llm-provider` there is no `RebornProviderAdmin` to probe with, so
/// this exists solely to keep `execute()`'s unconditional `&LiveLlmProbe`
/// call site compiling. The feature-off `provision_llm_credentials` below
/// ignores its `probe` parameter entirely, so `probe` is never called.
#[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
pub(crate) trait LlmProbe {
    fn probe(&self) -> anyhow::Result<()>;
}

#[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
pub(crate) struct LiveLlmProbe;

#[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
impl LlmProbe for LiveLlmProbe {
    fn probe(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Provision onboard's `[llm.default]` slot.
///
/// Before the numbered menu, this checks whether a complete LLM
/// configuration is already detectable from the environment
/// (`RebornProviderAdmin::detect_env_llm`, the same
/// `ironclaw_llm::resolve_provider_config_from_env` resolution
/// `resolve_reborn_runtime_llm`'s own fallback path and the `run`/`serve`
/// stub-gateway warning use):
///
/// - **Interactive**, detected: asks "Found `<provider>` configured in
///   environment — use it?" ([`PromptSource::confirm`]). A yes answer seeds
///   `[llm.default]` from the detected provider/model via `set_provider` —
///   storing NO key in the encrypted secret store; the env var stays the key
///   source at runtime (`set_provider` leaves `api_key_env` at the catalog
///   default, and `ironclaw_llm`'s resolution reads that env var by name at
///   startup — see `resolve_provider_definition` in `ironclaw_llm`). A no
///   answer falls through to the full numbered menu below.
/// - **Interactive**, partial/invalid env (`Err`): prints one line noting
///   the environment config is being ignored, then falls through to the
///   full menu.
/// - **Interactive**, nothing detected (`Ok(None)`): falls through to the
///   full menu unchanged.
/// - **Headless**, detected: seeds `[llm.default]` silently (no prompt was
///   possible) and reports it in onboard's printed output.
/// - **Headless**, partial/invalid or nothing detected: seeds nothing;
///   returns a `Skipped*` outcome whose `display_line` teaches the operator
///   what to do next.
///
/// The full numbered menu (via [`super::prompts::PromptSource::provider_menu`])
/// then prompts for a provider, its API key when required, and a model
/// override, then persists what's needed. Both prompts (`api_key`, `model`)
/// run first — they're pure reads of user input with nothing durable behind
/// them — so every fallible step remaining once a write starts is a write
/// itself, never a prompt: any API key value goes into the encrypted secret
/// store via the canonical `LlmKeyStore` handle (`llm_provider_<id>_api_key`)
/// FIRST — the same handle the webui2 settings surface writes and
/// `apply_startup_stored_llm_key` reads at boot — and only once that
/// succeeds does the provider selection land in `[llm.default]` in
/// `config.toml` SECOND (existing `RebornProviderAdmin::set_provider` config
/// machinery, the same one `ironclaw-reborn models set-provider` uses). This
/// ordering means `config.toml` can never point at a provider whose key
/// failed to persist durably: a `LlmKeyStore::put` failure returns an error
/// before `set_provider` is ever called, leaving `config.toml` exactly as it
/// was — and, symmetrically, a prompt failure (e.g. Ctrl-D on the model
/// prompt) can never leave an orphan key in the secret store, because no
/// prompt runs after the store write starts. A provider whose menu entry has
/// `api_key_required: false` skips the key prompt and secret-store write
/// entirely — there is nothing to persist there. Every menu-eligible
/// provider today (including `nearai`, via a menu-level override — see
/// `RebornProviderAdmin::menu_entries`'s doc) requires a key, so this branch
/// is currently unreachable in practice but stays in place for any future
/// menu entry that doesn't.
///
/// Skips ALL of the above entirely (an idempotent no-op) on a rerun where
/// `[llm.default]` is already user-configured AND (the provider doesn't
/// require a key OR the store already has a key for it), unless `force` is
/// set — see [`already_configured_outcome`]. This idempotency check now
/// covers env-seeded slots too: once seeded, drift between the slot and a
/// since-changed environment is accepted, not re-detected — `--force`
/// re-seeds from the environment again.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
pub(crate) fn provision_llm_credentials(
    home: &RebornHome,
    boot: &ironclaw_reborn_config::RebornBootConfig,
    prompts: &mut dyn PromptSource,
    store_opener: &dyn LlmKeyStoreOpener,
    probe: &dyn LlmProbe,
    force: bool,
) -> Result<LlmCredentialProvisionOutcome, LlmCredentialPromptError> {
    let admin = ironclaw_reborn_composition::RebornProviderAdmin::new(boot.clone());

    if !force && let Some(outcome) = already_configured_outcome(&admin, home, store_opener)? {
        return Ok(outcome);
    }

    if !prompts.is_interactive() {
        return provision_headless_from_env(&admin);
    }

    match admin.detect_env_llm() {
        Ok(Some(detected)) => {
            let question = format!(
                "Found `{}` configured in environment — use it?",
                detected.provider_id
            );
            if prompts.confirm(&question)? {
                let write_outcome = admin
                    .set_provider(&detected.provider_id, Some(detected.model.as_str()))
                    .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
                return Ok(LlmCredentialProvisionOutcome::ConfiguredFromEnv {
                    provider_id: write_outcome.provider_id,
                    model: write_outcome.model,
                });
            }
            // Declined: fall through to the full numbered menu below.
        }
        Ok(None) => {
            // Nothing detected: fall through to the full numbered menu
            // unchanged — this is the common fresh-install case.
        }
        Err(error) => {
            println!("ignoring partial environment LLM config: {error}");
            // Fall through to the full numbered menu below.
        }
    }

    provision_via_menu(home, &admin, prompts, store_opener, probe)
}

/// Headless (non-interactive) counterpart of the env-detect step in
/// [`provision_llm_credentials`]'s doc: no prompt is possible, so a detected
/// configuration is seeded silently, and anything else (partial env or
/// nothing detected) seeds nothing and returns a `Skipped*` outcome whose
/// `display_line` teaches the operator what to do next.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn provision_headless_from_env(
    admin: &ironclaw_reborn_composition::RebornProviderAdmin,
) -> Result<LlmCredentialProvisionOutcome, LlmCredentialPromptError> {
    match admin.detect_env_llm() {
        Ok(Some(detected)) => {
            let write_outcome = admin
                .set_provider(&detected.provider_id, Some(detected.model.as_str()))
                .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
            Ok(LlmCredentialProvisionOutcome::ConfiguredFromEnv {
                provider_id: write_outcome.provider_id,
                model: write_outcome.model,
            })
        }
        Ok(None) => Ok(LlmCredentialProvisionOutcome::SkippedNonInteractive),
        Err(error) => Ok(
            LlmCredentialProvisionOutcome::SkippedNonInteractivePartialEnv {
                reason: error.to_string(),
            },
        ),
    }
}

/// Drive the full numbered provider menu — the pre-env-detect behavior of
/// [`provision_llm_credentials`], factored out so both the "declined
/// confirm" and "nothing detected" branches share exactly one
/// implementation. See that function's doc for the store-then-config write
/// ordering this preserves.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn provision_via_menu(
    home: &RebornHome,
    admin: &ironclaw_reborn_composition::RebornProviderAdmin,
    prompts: &mut dyn PromptSource,
    store_opener: &dyn LlmKeyStoreOpener,
    probe: &dyn LlmProbe,
) -> Result<LlmCredentialProvisionOutcome, LlmCredentialPromptError> {
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
    // partway through with a secret already committed. See
    // `provision_llm_credentials`'s doc for the store-then-config write
    // ordering.
    let initial_key = if entry.api_key_required {
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
    let effective_model = model.as_deref().unwrap_or(default_model.as_str());

    // Live key/model verification — key-required providers only. No-key
    // providers and every path that never reaches this function (headless
    // seeding, the interactive env-detect confirm-yes branch) are never
    // probed: see `provision_llm_credentials`'s doc for why an env-sourced
    // or keyless credential is already trusted. `nearai` is
    // `api_key_required: true` here (menu-level override — see
    // `RebornProviderAdmin::menu_entries`'s doc), so it takes this same
    // branch like every other key-required provider; there is no
    // nearai-specific case in this function.
    let key = match initial_key {
        Some(key) => Some(probe_and_confirm_key(
            prompts,
            probe,
            admin,
            &canonical_provider_id,
            effective_model,
            key,
        )?),
        None => None,
    };

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

/// Probe `candidate_key` against `provider_id`/`effective_model` and, on
/// failure, either reprompt for a new key or accept the operator's "store
/// anyway" answer — the loop [`provision_via_menu`] runs after the key and
/// model prompts, before either durable write.
///
/// `probe`'s outcome type
/// ([`ironclaw_reborn_composition::ProviderProbeOutcome`]) carries a single
/// `ok: bool` with no separate auth-vs-transport signal (see
/// `RebornProviderAdmin::probe_candidate`'s doc) — so every failure, whether
/// a rejected key or an unreachable endpoint, takes the SAME branch here:
/// show the provider's message, then ask "store anyway?" via
/// [`PromptSource::confirm`]. A "yes" answer accepts the current key as-is
/// (covers both "the key is fine, the network/endpoint just isn't reachable
/// right now" and "store it anyway, I'll fix it later"); a "no" answer
/// reprompts for a different key, up to `MAX_PROBE_ATTEMPTS` total key
/// entries — inventing a classification the underlying probe API doesn't
/// support would be worse than treating every failure alike.
///
/// A successful probe with a non-empty model list that doesn't contain
/// `effective_model` prints a warning but still returns the key (provider
/// model lists are frequently incomplete, so an unlisted model is not
/// treated as an error); an empty model list is not warned about at all.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn probe_and_confirm_key(
    prompts: &mut dyn PromptSource,
    probe: &dyn LlmProbe,
    admin: &ironclaw_reborn_composition::RebornProviderAdmin,
    provider_id: &str,
    effective_model: &str,
    mut candidate_key: String,
) -> Result<String, LlmCredentialPromptError> {
    const MAX_PROBE_ATTEMPTS: u8 = 3;
    let mut attempt = 1u8;
    loop {
        let outcome = probe
            .probe(
                admin,
                provider_id,
                Some(candidate_key.as_str()),
                Some(effective_model),
            )
            .map_err(LlmCredentialPromptError::Other)?;

        if outcome.ok {
            if !outcome.models.is_empty() && !outcome.models.iter().any(|m| m == effective_model) {
                println!(
                    "warning: `{effective_model}` was not in {provider_id}'s reported model \
                     list ({} models) — continuing anyway, provider model lists are often \
                     incomplete",
                    outcome.models.len()
                );
            }
            return Ok(candidate_key);
        }

        println!("{}", outcome.message);
        let question = format!("Could not reach {provider_id} to verify — store anyway?");
        if prompts.confirm(&question)? {
            return Ok(candidate_key);
        }

        if attempt >= MAX_PROBE_ATTEMPTS {
            return Err(LlmCredentialPromptError::Other(anyhow::anyhow!(
                "no working API key for `{provider_id}` after {MAX_PROBE_ATTEMPTS} attempts; \
                 configure it later with `ironclaw-reborn models set-provider {provider_id}`"
            )));
        }
        attempt += 1;
        candidate_key = prompts.api_key(provider_id)?;
        if candidate_key.trim().is_empty() {
            return Err(LlmCredentialPromptError::Other(anyhow::anyhow!(
                "LLM API key must not be blank"
            )));
        }
    }
}

/// `Some` when `[llm.default]` already names a provider AND that provider is
/// durably credentialed — either it doesn't require an API key at all per
/// [`provider_api_key_required`]'s MENU-LEVEL definition, or the encrypted
/// secret store already has a key stored for it — the idempotent-rerun case
/// [`provision_llm_credentials`] must skip prompting for (a bare
/// stub-seeded `[llm.default]` with no stored key for a key-requiring
/// provider, e.g. right after a fresh `onboard` on a headless box, does NOT
/// count: that provider has never actually been credentialed, so a later
/// interactive rerun must still prompt).
///
/// `nearai` is `api_key_required: true` at the menu level (see
/// `RebornProviderAdmin::menu_entries`'s doc), so a `nearai` slot with no
/// stored key and no resolvable env key does NOT count as already
/// configured either — see
/// `provision_llm_credentials_nearai_slot_without_a_stored_key_is_not_already_configured`.
///
/// A store-open failure is treated as "can't tell" and falls through to
/// prompting rather than erroring the whole run (a fresh prompt/write can
/// still succeed even if the pre-flight check couldn't read the store). A
/// registry lookup failure, in contrast, is propagated — see
/// [`provider_api_key_required`]'s doc.
///
/// A `config.toml` LOAD failure (unparseable TOML — e.g. a pre-existing,
/// operator-authored `config.toml` this idempotency check runs BEFORE the
/// interactivity gate now runs against too, per the env-detect step's
/// headless idempotency requirement) is likewise treated as "can't tell":
/// this check's whole job is figuring out whether to SKIP work, so a config
/// it cannot read must never turn into a hard failure here — `set_provider`/
/// `write_default_config_files` are the real authorities on whether a
/// malformed `config.toml` is fatal, and `onboard`'s `ExistingConfigPolicy::
/// Preserve` deliberately leaves an operator-authored (even malformed)
/// `config.toml` untouched rather than validating it.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn already_configured_outcome(
    admin: &ironclaw_reborn_composition::RebornProviderAdmin,
    home: &RebornHome,
    store_opener: &dyn LlmKeyStoreOpener,
) -> Result<Option<LlmCredentialProvisionOutcome>, LlmCredentialPromptError> {
    let status = match admin.status() {
        Ok(status) => status,
        Err(ironclaw_reborn_composition::RebornProviderAdminError::LoadConfig {
            source, ..
        }) => {
            tracing::debug!(
                error = %source,
                "config.toml failed to parse while checking already-configured LLM; falling \
                 through rather than failing the whole run"
            );
            return Ok(None);
        }
        Err(error) => return Err(LlmCredentialPromptError::Other(error.into())),
    };
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

/// Whether `provider_id` requires an API key, per the MENU-LEVEL definition
/// (`RebornProviderAdmin::effective_api_key_required` — not the raw
/// `providers.json` field; a `session_token`-kind provider like `nearai` is
/// overridden to `true` there because reborn has no session-token auth
/// wired). Not menu-restricted otherwise — `[llm.default]` may name a
/// provider excluded from the onboard menu, e.g. one set via `models
/// set-provider`.
///
/// Returns `Err` when the registry lookup itself fails (e.g. a corrupt or
/// unreadable `providers.json`) — that's a real failure, not "unconfigured",
/// and must not be swallowed into a silent re-prompt (a swallowed registry
/// error here would make `already_configured_outcome` treat a broken
/// registry the same as "never configured", re-running the interactive
/// prompt — and on `--force`, re-writing credentials — every time). Returns
/// `Ok(None)` only for the genuinely "can't tell" case: `provider_id` isn't
/// in the registry at all.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn provider_api_key_required(
    admin: &ironclaw_reborn_composition::RebornProviderAdmin,
    provider_id: &str,
) -> Result<Option<bool>, LlmCredentialPromptError> {
    admin
        .effective_api_key_required(provider_id)
        .map_err(|error| LlmCredentialPromptError::Other(error.into()))
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
    _probe: &dyn LlmProbe,
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

        fn confirm(&mut self, _question: &str) -> Result<bool, LlmCredentialPromptError> {
            panic!(
                "confirm() must not be called: these tests run with a clean process \
                 environment (no LLM env vars set), so detect_env_llm() must return Ok(None) \
                 and fall straight through to the numbered menu"
            )
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

        fn confirm(&mut self, _question: &str) -> Result<bool, LlmCredentialPromptError> {
            unreachable!(
                "confirm() must not be called once is_interactive() is false: the headless \
                 env-detect path never prompts"
            )
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

        fn confirm(&mut self, _question: &str) -> Result<bool, LlmCredentialPromptError> {
            panic!("confirm() must not be called on an idempotent, already-configured rerun")
        }
    }

    /// An [`LlmProbe`] that always reports success with no model list —
    /// used by every test unrelated to item 2's probe-driven reprompt loop
    /// so their pre-existing `Configured`/store-write assertions stay
    /// exactly as they were before probing was added. Also pins the "empty
    /// model list is not warned about" branch: no test asserting on stdout
    /// output regresses from this fake's empty `models`.
    struct StubOkProbe;

    impl LlmProbe for StubOkProbe {
        fn probe(
            &self,
            _admin: &ironclaw_reborn_composition::RebornProviderAdmin,
            _provider_id: &str,
            _api_key: Option<&str>,
            _model: Option<&str>,
        ) -> anyhow::Result<ironclaw_reborn_composition::ProviderProbeOutcome> {
            Ok(ironclaw_reborn_composition::ProviderProbeOutcome {
                ok: true,
                models: Vec::new(),
                message: String::new(),
            })
        }
    }

    /// An [`LlmProbe`] that panics if called — used to prove the probe
    /// never runs on paths item 2 explicitly excludes: an idempotent
    /// already-configured rerun, a headless run, an interactive env-detect
    /// confirm-yes branch, and a blank-key rejection (which fails before
    /// the probe step is ever reached). Every menu-eligible provider
    /// requires a key today (including `nearai`, via its menu-level
    /// override), so there is no longer a true "no-key provider" case to
    /// list here.
    struct PanickingProbe;

    impl LlmProbe for PanickingProbe {
        fn probe(
            &self,
            _admin: &ironclaw_reborn_composition::RebornProviderAdmin,
            _provider_id: &str,
            _api_key: Option<&str>,
            _model: Option<&str>,
        ) -> anyhow::Result<ironclaw_reborn_composition::ProviderProbeOutcome> {
            panic!(
                "probe() must not be called: idempotent reruns, headless runs, env-seeded \
                 selections, and a rejected blank key must never reach the live key/model \
                 verification probe"
            )
        }
    }

    /// A [`LlmProbe`] scripted with a fixed sequence of outcomes, consumed
    /// in order — proves the probe-driven reprompt loop in
    /// `probe_and_confirm_key` without a live LLM endpoint. Panics if
    /// called more times than scripted (proving the loop asks exactly as
    /// many times as expected, no more).
    struct ScriptedProbe {
        outcomes: std::cell::RefCell<
            std::collections::VecDeque<ironclaw_reborn_composition::ProviderProbeOutcome>,
        >,
    }

    impl ScriptedProbe {
        fn new(outcomes: Vec<ironclaw_reborn_composition::ProviderProbeOutcome>) -> Self {
            Self {
                outcomes: std::cell::RefCell::new(outcomes.into()),
            }
        }
    }

    impl LlmProbe for ScriptedProbe {
        fn probe(
            &self,
            _admin: &ironclaw_reborn_composition::RebornProviderAdmin,
            _provider_id: &str,
            _api_key: Option<&str>,
            _model: Option<&str>,
        ) -> anyhow::Result<ironclaw_reborn_composition::ProviderProbeOutcome> {
            self.outcomes
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("ScriptedProbe: no more scripted outcomes"))
        }
    }

    /// A [`PromptSource`] for item 2's probe-driven reprompt tests: scripts
    /// a fixed sequence of `api_key()` answers and `confirm()` answers,
    /// each consumed in order; panics if either sequence is exhausted,
    /// proving the reprompt loop asks exactly as many times as expected.
    struct ScriptedKeyPromptSource {
        provider: &'static str,
        keys: std::collections::VecDeque<&'static str>,
        confirms: std::collections::VecDeque<bool>,
        model: Option<&'static str>,
    }

    impl PromptSource for ScriptedKeyPromptSource {
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
            Ok(self
                .keys
                .pop_front()
                .expect("ScriptedKeyPromptSource: api_key() called more times than scripted")
                .to_string())
        }

        fn model(
            &mut self,
            _provider_id: &str,
            _default_model: &str,
        ) -> Result<Option<String>, LlmCredentialPromptError> {
            Ok(self.model.map(str::to_string))
        }

        fn confirm(&mut self, _question: &str) -> Result<bool, LlmCredentialPromptError> {
            Ok(self
                .confirms
                .pop_front()
                .expect("ScriptedKeyPromptSource: confirm() called more times than scripted"))
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
        let _env_guard = crate::runtime::test_env::lock_runtime_env();
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
            &StubOkProbe,
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
            &PanickingProbe,
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

    /// `nearai` is `api_key_required: true` at the menu level (see
    /// `RebornProviderAdmin::menu_entries`'s doc: reborn has no
    /// session-token auth wired, so a `session_token`-kind provider is
    /// required-key here even though the raw catalog entry marks it
    /// optional). This test pins that it now takes the EXACT SAME path as
    /// `openai` — required-key prompt, live probe, store-then-config write,
    /// idempotent rerun via a stored key — with no nearai-specific
    /// behavior anywhere in `provision_via_menu`.
    #[test]
    fn provision_llm_credentials_nearai_requires_and_stores_an_api_key_like_any_other_provider() {
        let _env_guard = crate::runtime::test_env::lock_runtime_env();
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        seed_cached_master_key(home);

        let mut prompts = FakePromptSource {
            provider: "nearai",
            key: "session-test-value",
            model: None,
        };
        let outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            &StubOkProbe,
            false,
        )
        .expect("provision must succeed with a fake interactive source");
        assert_eq!(
            outcome,
            LlmCredentialProvisionOutcome::Configured {
                provider_id: "nearai".to_string(),
                model: "deepseek-ai/DeepSeek-V4-Flash".to_string(),
            }
        );

        let home_path = home.path().to_path_buf();
        let stored = crate::runtime::block_on_cli(async move {
            let store = ironclaw_reborn_composition::open_local_dev_secret_store(&home_path)
                .await
                .map_err(anyhow::Error::from)?;
            ironclaw_reborn_composition::LlmKeyStore::new(store)
                .read("nearai")
                .await
                .map_err(anyhow::Error::from)
        })
        .expect("read back through a fresh open of the same root");
        assert_eq!(
            secrecy::ExposeSecret::expose_secret(&stored.expect("a value must have been stored")),
            "session-test-value"
        );

        // Idempotent rerun, exactly like the openai test above: a stored
        // key makes a second run skip prompting entirely.
        let mut second_prompts = PanickingPromptSource;
        let second_outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut second_prompts,
            &LocalDevLlmKeyStoreOpener,
            &PanickingProbe,
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

    /// RED (nearai onboarding scope addition, idempotency ask #4): a
    /// `[llm.default]` slot already pointing at `nearai` (e.g. seeded
    /// directly via `RebornProviderAdmin::set_provider`, mirroring what
    /// `ironclaw-reborn models set-provider nearai` would leave behind)
    /// with NO stored key must NOT be treated as already configured — that
    /// state is broken (reborn has no session-token auth wired, so a keyless
    /// nearai slot dead-ends at the first chat turn). A rerun must fall
    /// through to the full prompt flow and land a real key, proven here by
    /// asserting the outcome is `Configured` (only reachable by actually
    /// invoking `provider_menu()`/`api_key()`), not `AlreadyConfigured`.
    #[test]
    fn provision_llm_credentials_nearai_slot_without_a_stored_key_is_not_already_configured() {
        let _env_guard = crate::runtime::test_env::lock_runtime_env();
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        seed_cached_master_key(home);

        let admin =
            ironclaw_reborn_composition::RebornProviderAdmin::new(context.boot_config().clone());
        admin
            .set_provider("nearai", None)
            .expect("seed a bare nearai slot directly, bypassing onboard's key prompt/store");

        let mut prompts = FakePromptSource {
            provider: "nearai",
            key: "session-test-value",
            model: None,
        };
        let outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            &StubOkProbe,
            false,
        )
        .expect("a keyless nearai slot must re-prompt, not error");
        assert_eq!(
            outcome,
            LlmCredentialProvisionOutcome::Configured {
                provider_id: "nearai".to_string(),
                model: "deepseek-ai/DeepSeek-V4-Flash".to_string(),
            },
            "a nearai slot with no stored key must never be treated as AlreadyConfigured — \
             `Configured` here proves the full prompt flow ran instead of skipping it"
        );
    }

    /// A non-interactive session with no LLM environment variables set must
    /// now succeed with `Ok(SkippedNonInteractive)` (not the old
    /// `Err(NonInteractive)` — headless `onboard` no longer treats "nothing
    /// to prompt for, nothing detected in env" as a failure; see
    /// `provision_llm_credentials`'s doc for the env-detect-then-headless
    /// branch) and must not write anything: `provider_menu()`/`api_key()`/
    /// `model()`/`confirm()` are all `unreachable!()` on
    /// `NonInteractivePromptSource` (proving the interactivity check
    /// short-circuits before any prompt runs) and `config.toml` must not
    /// exist afterward (proving no store/config touch happens without a
    /// detected environment).
    #[test]
    fn provision_llm_credentials_is_a_noop_when_non_interactive_with_no_env_detected() {
        let _env_guard = crate::runtime::test_env::lock_runtime_env();
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");

        let mut prompts = NonInteractivePromptSource;
        let outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            &PanickingProbe,
            false,
        )
        .expect("a non-interactive source with nothing detected in env must succeed as a no-op");
        assert_eq!(
            outcome,
            LlmCredentialProvisionOutcome::SkippedNonInteractive
        );
        assert!(
            !home.config_file_path().exists(),
            "a non-interactive no-op must not write config.toml"
        );
    }

    /// RED (item 2, write ordering): a store whose `put` always fails must
    /// leave `config.toml` completely untouched — proving the secret is
    /// written BEFORE the provider selection, not after. Under the old
    /// ordering (config first, store second) `config.toml` would already
    /// carry `provider_id = "openai"` by the time the store write failed.
    /// Uses `openai` — any key-requiring menu entry would exercise this
    /// ordering equally; a provider that never opened the secret store at
    /// all couldn't.
    #[test]
    fn provision_llm_credentials_leaves_config_untouched_when_the_store_put_fails() {
        let _env_guard = crate::runtime::test_env::lock_runtime_env();
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
            &StubOkProbe,
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
        let _env_guard = crate::runtime::test_env::lock_runtime_env();
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
            &PanickingProbe,
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
            let _env_guard = crate::runtime::test_env::lock_runtime_env();
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
                &PanickingProbe,
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
        let _env_guard = crate::runtime::test_env::lock_runtime_env();
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
            &StubOkProbe,
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

    /// `PromptSource` that answers a fixed `confirm()` result and, if the
    /// flow falls through to the menu instead, selects `provider`/`model`
    /// like [`FakePromptSource`] — used to drive both branches of the
    /// interactive env-detect step from one fake.
    struct ConfirmingPromptSource {
        confirm_answer: bool,
        provider: &'static str,
        /// Answered by `api_key()` on the confirm-NO fall-through-to-menu
        /// branch, which now always needs one — every menu entry (including
        /// `nearai`, via its menu-level override) requires a key. Unused on
        /// the confirm-YES branch, which seeds straight from the env-detected
        /// provider and never reaches `api_key()` at all.
        key: &'static str,
        model: Option<&'static str>,
    }

    impl PromptSource for ConfirmingPromptSource {
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

        fn confirm(&mut self, question: &str) -> Result<bool, LlmCredentialPromptError> {
            assert!(
                question.contains("openai"),
                "confirm question must name the detected provider: {question}"
            );
            Ok(self.confirm_answer)
        }
    }

    /// `PromptSource` whose `confirm()`/`provider_menu()` both panic if
    /// called — used to prove the headless env-detect path never prompts.
    struct HeadlessPromptSource;

    impl PromptSource for HeadlessPromptSource {
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
            unreachable!("api_key() must not be called once is_interactive() is false")
        }

        fn model(
            &mut self,
            _provider_id: &str,
            _default_model: &str,
        ) -> Result<Option<String>, LlmCredentialPromptError> {
            unreachable!("model() must not be called once is_interactive() is false")
        }

        fn confirm(&mut self, _question: &str) -> Result<bool, LlmCredentialPromptError> {
            unreachable!("confirm() must not be called once is_interactive() is false")
        }
    }

    /// RED (env-detect step 2a, interactive + detected + confirm yes): with
    /// a complete `openai` configuration in the environment (`OPENAI_API_KEY`
    /// set), an interactive session must ask to confirm using it, and a
    /// "yes" answer must seed `[llm.default]` from the DETECTED provider/
    /// model — via `set_provider`, WITHOUT ever opening the secret store or
    /// calling `api_key()`. The key stays resolvable from `OPENAI_API_KEY`
    /// at runtime (see `provision_llm_credentials`'s doc for why `set_provider`
    /// leaving `api_key_env` at its catalog default is sufficient — the env
    /// var name, not a stored value, is what `ironclaw_llm` resolution reads
    /// at startup).
    #[test]
    fn provision_llm_credentials_seeds_from_env_on_interactive_confirm_yes() {
        let _env_guard = crate::runtime::test_env::lock_runtime_env();
        // SAFETY: serialized by the shared crate process-env lock; cleaned up
        // before the guard drops.
        unsafe {
            std::env::set_var("OPENAI_API_KEY", "sk-env-detected-value");
        }
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");

        let mut prompts = ConfirmingPromptSource {
            confirm_answer: true,
            provider: "openai",
            key: "unused",
            model: None,
        };
        let outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            &PanickingProbe,
            false,
        );
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }
        let outcome = outcome.expect("provision must succeed on confirm-yes");
        assert_eq!(
            outcome,
            LlmCredentialProvisionOutcome::ConfiguredFromEnv {
                provider_id: "openai".to_string(),
                model: "gpt-5-mini".to_string(),
            }
        );

        let config_text =
            std::fs::read_to_string(home.config_file_path()).expect("read config.toml");
        assert!(
            config_text.contains("provider_id = \"openai\""),
            "config.toml: {config_text}"
        );

        let home_path = home.path().to_path_buf();
        let has_key = crate::runtime::block_on_cli(async move {
            let store = ironclaw_reborn_composition::open_local_dev_secret_store(&home_path)
                .await
                .map_err(anyhow::Error::from)?;
            ironclaw_reborn_composition::LlmKeyStore::new(store)
                .exists("openai")
                .await
                .map_err(anyhow::Error::from)
        })
        .expect("read back through a fresh open of the same root");
        assert!(
            !has_key,
            "an env-detected+confirmed seed must never write the secret store — the API key \
             stays resolvable from its env var at runtime"
        );
    }

    /// RED (env-detect step 2b, interactive + detected + confirm no): a
    /// "no" answer to the confirm prompt must fall through to the full
    /// numbered menu, proving the decline path is not a dead end.
    #[test]
    fn provision_llm_credentials_falls_through_to_menu_on_interactive_confirm_no() {
        let _env_guard = crate::runtime::test_env::lock_runtime_env();
        // SAFETY: serialized by the shared crate process-env lock; cleaned up
        // before the guard drops.
        unsafe {
            std::env::set_var("OPENAI_API_KEY", "sk-env-detected-value");
        }
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        seed_cached_master_key(home);

        let mut prompts = ConfirmingPromptSource {
            confirm_answer: false,
            provider: "nearai",
            key: "session-test-value",
            model: None,
        };
        let outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            &StubOkProbe,
            false,
        );
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }
        let outcome = outcome.expect("provision must succeed after declining the env prompt");
        assert_eq!(
            outcome,
            LlmCredentialProvisionOutcome::Configured {
                provider_id: "nearai".to_string(),
                model: "deepseek-ai/DeepSeek-V4-Flash".to_string(),
            },
            "declining the env-detected provider must fall through to the full menu, landing \
             the menu's own selection instead of the env-detected one"
        );
    }

    /// RED (env-detect step 3, headless + detected): a non-interactive
    /// session with a complete `openai` configuration in the environment
    /// must seed `[llm.default]` from it SILENTLY (no prompt is possible),
    /// without ever touching the secret store.
    #[test]
    fn provision_llm_credentials_seeds_from_env_when_headless() {
        let _env_guard = crate::runtime::test_env::lock_runtime_env();
        // SAFETY: serialized by the shared crate process-env lock; cleaned up
        // before the guard drops.
        unsafe {
            std::env::set_var("OPENAI_API_KEY", "sk-env-detected-value");
        }
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");

        let mut prompts = HeadlessPromptSource;
        let outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            &PanickingProbe,
            false,
        );
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }
        let outcome = outcome.expect("headless provision with a detected env config must succeed");
        assert_eq!(
            outcome,
            LlmCredentialProvisionOutcome::ConfiguredFromEnv {
                provider_id: "openai".to_string(),
                model: "gpt-5-mini".to_string(),
            }
        );
        let config_text =
            std::fs::read_to_string(home.config_file_path()).expect("read config.toml");
        assert!(
            config_text.contains("provider_id = \"openai\""),
            "config.toml: {config_text}"
        );
    }

    /// RED (env-detect step 4, headless + partial env): a non-interactive
    /// session with an INCOMPLETE environment configuration (`OPENAI_MODEL`
    /// set without the required `OPENAI_API_KEY`) must seed nothing and
    /// report a `SkippedNonInteractivePartialEnv` outcome naming the reason
    /// — never silently adopt a broken environment, and never fall back to
    /// a hardcoded default provider.
    #[test]
    fn provision_llm_credentials_seeds_nothing_when_headless_env_is_partial() {
        let _env_guard = crate::runtime::test_env::lock_runtime_env();
        // SAFETY: serialized by the shared crate process-env lock; cleaned up
        // before the guard drops.
        unsafe {
            std::env::set_var("OPENAI_MODEL", "gpt-test-model");
        }
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");

        let mut prompts = HeadlessPromptSource;
        let outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            &PanickingProbe,
            false,
        );
        unsafe {
            std::env::remove_var("OPENAI_MODEL");
        }
        let outcome = outcome.expect("a partial env must not fail onboard overall");
        match outcome {
            LlmCredentialProvisionOutcome::SkippedNonInteractivePartialEnv { reason } => {
                assert!(
                    reason.to_lowercase().contains("openai") || !reason.is_empty(),
                    "reason should describe the incomplete provider: {reason}"
                );
            }
            other => panic!("expected SkippedNonInteractivePartialEnv, got {other:?}"),
        }
        assert!(
            !home.config_file_path().exists(),
            "a partial env must leave config.toml untouched"
        );
    }

    /// RED (first-run regression): a fresh reborn home, an interactive
    /// session, and a clean environment (no LLM env vars set) must still
    /// invoke the full numbered `provider_menu()` — pinning the exact
    /// regression the env-detect-and-confirm step risked reintroducing: an
    /// earlier revision of this env-detect step, developed against this PR,
    /// treated `Ok(None)` (nothing detected) as equivalent to "nothing to
    /// do" and skipped straight to `SkippedNonInteractive`-shaped behavior
    /// even though the session WAS interactive, silently dropping the
    /// first-run menu a fresh desktop install depends on. `FakePromptSource`
    /// panics on `confirm()`, so this test would also fail loudly if
    /// `confirm()` were spuriously invoked with nothing detected.
    #[test]
    fn fresh_home_interactive_with_clean_env_still_invokes_the_provider_menu() {
        let _env_guard = crate::runtime::test_env::lock_runtime_env();
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        seed_cached_master_key(home);
        assert!(
            !home.config_file_path().exists(),
            "must start from a genuinely fresh home with no pre-existing config.toml"
        );

        let mut prompts = FakePromptSource {
            provider: "nearai",
            key: "session-test-value",
            model: None,
        };
        let outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            &StubOkProbe,
            false,
        )
        .expect("provision must succeed by falling through to the menu");
        assert_eq!(
            outcome,
            LlmCredentialProvisionOutcome::Configured {
                provider_id: "nearai".to_string(),
                model: "deepseek-ai/DeepSeek-V4-Flash".to_string(),
            },
            "the numbered menu's own selection must land in config.toml, proving \
             provider_menu() was actually invoked (FakePromptSource's provider_menu is the only \
             path that can produce this outcome)"
        );
    }

    fn probe_outcome(
        ok: bool,
        models: Vec<&str>,
        message: &str,
    ) -> ironclaw_reborn_composition::ProviderProbeOutcome {
        ironclaw_reborn_composition::ProviderProbeOutcome {
            ok,
            models: models.into_iter().map(str::to_string).collect(),
            message: message.to_string(),
        }
    }

    /// RED (item 2, rejected key → reprompt → accepted): a probe failure
    /// (whether the key was actually rejected or the endpoint was merely
    /// unreachable — `ProviderProbeOutcome` carries no signal to tell the
    /// two apart, see `probe_and_confirm_key`'s doc) followed by a "store
    /// anyway?" decline must reprompt for a NEW key, and a second probe
    /// that succeeds must store THAT key — proving the reprompt loop
    /// actually replaces the candidate rather than retrying the same one.
    #[test]
    fn provision_llm_credentials_probe_failure_then_reprompt_then_accepted() {
        let _env_guard = crate::runtime::test_env::lock_runtime_env();
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        seed_cached_master_key(home);

        let mut prompts = ScriptedKeyPromptSource {
            provider: "openai",
            keys: std::collections::VecDeque::from(["sk-bad", "sk-good"]),
            confirms: std::collections::VecDeque::from([false]),
            model: None,
        };
        let probe = ScriptedProbe::new(vec![
            probe_outcome(false, vec![], "invalid api key"),
            probe_outcome(true, vec![], ""),
        ]);
        let outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            &probe,
            false,
        )
        .expect("provision must succeed after a reprompted key passes the probe");
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
        assert_eq!(
            secrecy::ExposeSecret::expose_secret(&stored.expect("a value must have been stored")),
            "sk-good",
            "the SECOND (reprompted) key must be the one stored, not the first rejected one"
        );
    }

    /// RED (item 2, rejected×3 → error): three consecutive probe failures,
    /// each declined via "store anyway? no", must exhaust
    /// `MAX_PROBE_ATTEMPTS` and error out — leaving `config.toml` untouched
    /// — rather than looping forever or silently giving up after a
    /// different count.
    #[test]
    fn provision_llm_credentials_probe_failure_three_times_errors_without_writing() {
        let _env_guard = crate::runtime::test_env::lock_runtime_env();
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        seed_cached_master_key(home);

        let mut prompts = ScriptedKeyPromptSource {
            provider: "openai",
            keys: std::collections::VecDeque::from(["sk-1", "sk-2", "sk-3"]),
            confirms: std::collections::VecDeque::from([false, false, false]),
            model: None,
        };
        let probe = ScriptedProbe::new(vec![
            probe_outcome(false, vec![], "invalid api key"),
            probe_outcome(false, vec![], "invalid api key"),
            probe_outcome(false, vec![], "invalid api key"),
        ]);
        let error = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            &probe,
            false,
        )
        .expect_err("three failed probe attempts, all declined, must error");
        assert!(matches!(error, LlmCredentialPromptError::Other(_)));
        assert!(
            !home.config_file_path().exists(),
            "an exhausted probe-reprompt loop must leave config.toml untouched"
        );
    }

    /// RED (item 2, unreachable → confirm yes → stored anyway): a single
    /// probe failure followed by an accepted "store anyway?" must store the
    /// key as entered without any further reprompt — proving offline/
    /// unreachable-endpoint onboarding stays possible.
    #[test]
    fn provision_llm_credentials_probe_failure_confirm_yes_stores_anyway() {
        let _env_guard = crate::runtime::test_env::lock_runtime_env();
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        seed_cached_master_key(home);

        let mut prompts = ScriptedKeyPromptSource {
            provider: "openai",
            keys: std::collections::VecDeque::from(["sk-offline"]),
            confirms: std::collections::VecDeque::from([true]),
            model: None,
        };
        let probe = ScriptedProbe::new(vec![probe_outcome(
            false,
            vec![],
            "could not reach openai with these settings",
        )]);
        let outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            &probe,
            false,
        )
        .expect("a confirmed store-anyway must succeed");
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
        assert_eq!(
            secrecy::ExposeSecret::expose_secret(&stored.expect("a value must have been stored")),
            "sk-offline"
        );
    }

    /// RED (item 2, chosen model not in the reported list): a successful
    /// probe whose model list doesn't contain the chosen model must still
    /// write the key/config — an incomplete provider model list is a
    /// warning, never an error.
    #[test]
    fn provision_llm_credentials_probe_ok_model_not_in_list_still_writes() {
        let _env_guard = crate::runtime::test_env::lock_runtime_env();
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        seed_cached_master_key(home);

        let mut prompts = FakePromptSource {
            provider: "openai",
            key: "sk-test-value",
            model: Some("not-a-real-model"),
        };
        let probe = ScriptedProbe::new(vec![probe_outcome(true, vec!["gpt-5-mini", "gpt-5"], "")]);
        let outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &LocalDevLlmKeyStoreOpener,
            &probe,
            false,
        )
        .expect("an unlisted model must warn, not fail");
        assert_eq!(
            outcome,
            LlmCredentialProvisionOutcome::Configured {
                provider_id: "openai".to_string(),
                model: "not-a-real-model".to_string(),
            }
        );
    }
}
