// arch-exempt: large_file, targeted onboarding feature-matrix validation stays at the existing credential seam, plan #4088
//! Onboarding's LLM-credential provisioning step: prompt for a provider and
//! API key, then persist both — the secret store write lands before the
//! `config.toml` selection (see [`provision_llm_credentials`]'s doc).

#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
use std::path::Path;

use ironclaw_reborn_config::RebornHome;

use super::prompts::{LlmCredentialPromptError, PromptSource};

/// Outcome of onboard's LLM provider/API-key prompt step. Every variant is a
/// successful `execute()` (exit 0) — mirrors [`super::master_key::MasterKeyProvisionOutcome`]'s
/// shape: the `Skipped*` variants are expected and normal, not a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LlmCredentialProvisionOutcome {
    #[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
    Configured { provider_id: String, model: String },
    /// `[llm.default]` was already pointed at a provider AND the encrypted
    /// secret store already has a key for it (see
    /// [`already_configured_outcome`]) — this run skipped prompting
    /// entirely rather than re-asking for credentials that are already
    /// durably stored.
    #[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
    AlreadyConfigured { provider_id: String, model: String },
    /// Complete LLM config detected in env (`RebornProviderAdmin::detect_env_llm`)
    /// and `[llm.default]` seeded via `set_provider`. The detected API key is
    /// also persisted to the encrypted secret store (same path the menu flow
    /// uses) so a background service — which only carries
    /// `IRONCLAW_REBORN_HOME`, not the operator's shell env — can still
    /// resolve it. Reached via interactive "use it?" confirm or silently on
    /// a headless run.
    /// - Idempotency: once seeded, drift between slot and live env is accepted
    ///   (not re-synced) on later runs; `--force` re-seeds from env again.
    #[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
    ConfiguredFromEnv { provider_id: String, model: String },
    /// Headless (non-interactive) session; no LLM environment variables are
    /// set at all. Nothing was seeded.
    #[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
    SkippedNonInteractive,
    /// Headless (non-interactive) session; some LLM environment
    /// configuration was present but incomplete or invalid (e.g. a
    /// provider's model env var set without its required API key env var).
    /// Nothing was seeded — a partial/broken environment must never be
    /// silently adopted.
    #[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
    SkippedNonInteractivePartialEnv { reason: String },
    /// This binary was built without the storage/provider features required
    /// for interactive LLM provisioning. Unlike `SkippedNonInteractive`,
    /// changing terminal interactivity cannot make this step available.
    #[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
    UnavailableInBuild,
}

impl LlmCredentialProvisionOutcome {
    pub(crate) fn display_line(&self) -> String {
        match self {
            #[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
            Self::Configured { provider_id, model } => {
                format!("configured provider `{provider_id}` (model `{model}`)")
            }
            #[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
            Self::AlreadyConfigured { provider_id, model } => {
                format!(
                    "already configured (provider `{provider_id}`, model `{model}`); use \
                     --force to reconfigure"
                )
            }
            #[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
            Self::ConfiguredFromEnv { provider_id, model } => {
                format!("configured provider `{provider_id}` (model `{model}`) from environment")
            }
            #[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
            Self::SkippedNonInteractive => "skipped (non-interactive session)".to_string(),
            #[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
            Self::SkippedNonInteractivePartialEnv { reason } => {
                format!(
                    "skipped (non-interactive session; partial environment LLM config: {reason})"
                )
            }
            #[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
            Self::UnavailableInBuild => {
                "unavailable in this build (requires `libsql` and `root-llm-provider`)".to_string()
            }
        }
    }
}

/// Where [`provision_llm_credentials`] gets its (already-open) encrypted
/// secret store from. Injected — mirrors [`PromptSource`] — so a test can
/// supply a store whose `put` fails, proving the store-before-config write
/// ordering without touching the real local-dev libsql-backed store.
/// - Gated with the same `libsql`+`root-llm-provider` cfg as
///   `ironclaw_reborn_composition::LlmKeyStore` (only exists behind those
///   features); see the `#[cfg(not(...))]` stub below for feature-off.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
pub(crate) trait LlmKeyStoreOpener {
    fn open(&self, home_path: &Path) -> anyhow::Result<ironclaw_reborn_composition::LlmKeyStore>;
}

/// Production [`LlmKeyStoreOpener`]: opens the real local-dev encrypted
/// secret store `serve` later reads from (see
/// `ironclaw_reborn_composition::open_local_dev_secret_store`'s doc for why
/// this is the same physical storage `serve` opens).
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
pub(crate) struct EncryptedLlmKeyStoreOpener;

#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
impl LlmKeyStoreOpener for EncryptedLlmKeyStoreOpener {
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

/// Feature-off stub: no `LlmKeyStore` type without both `libsql` and
/// `root-llm-provider`. Exists so `execute()`'s unconditional
/// `&EncryptedLlmKeyStoreOpener` call site compiles everywhere — the
/// feature-off `provision_llm_credentials` below never calls `open`.
#[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
pub(crate) trait LlmKeyStoreOpener {}

#[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
pub(crate) struct EncryptedLlmKeyStoreOpener;

#[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
impl LlmKeyStoreOpener for EncryptedLlmKeyStoreOpener {}

/// Where `provision_via_menu`'s pre-write key/model verification probe comes
/// from — injected so a test can script outcomes (rejected key, unreachable
/// endpoint, ok-with-a-model-list, …) without a live LLM endpoint.
/// - Unlike [`LlmKeyStoreOpener`] (opens a durable resource), this performs
///   the side-effecting network call itself, so its method takes the
///   already-built [`ironclaw_reborn_composition::RebornProviderAdmin`]
///   rather than raw construction ingredients.
/// - Gated the same as [`LlmKeyStoreOpener`]: no `RebornProviderAdmin`/
///   `ProviderProbeOutcome` without both `libsql` and `root-llm-provider`.
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

/// Feature-off stub, same reasoning as [`LlmKeyStoreOpener`]'s stub: keeps
/// `execute()`'s unconditional `&LiveLlmProbe` call site compiling; the
/// feature-off `provision_llm_credentials` below never calls `probe`.
#[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
pub(crate) trait LlmProbe {}

#[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
pub(crate) struct LiveLlmProbe;

#[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
impl LlmProbe for LiveLlmProbe {}

/// Provision onboard's `[llm.default]` slot.
///
/// Env-detect step (before the numbered menu), via `RebornProviderAdmin::detect_env_llm`
/// (same resolution `resolve_reborn_runtime_llm`'s fallback and the `run`/`serve`
/// stub-gateway warning use):
/// - **Interactive, detected**: asks "Found `<provider>` configured in environment
///   — use it?" ([`PromptSource::confirm`]). Yes seeds `[llm.default]` from
///   `set_provider`, storing NO key (env var stays the source at runtime —
///   `set_provider` leaves `api_key_env` at catalog default). No falls through
///   to the menu.
/// - **Interactive, partial/invalid env (`Err`)**: prints a note, falls through.
/// - **Interactive, nothing detected (`Ok(None)`)**: falls through unchanged.
/// - **Headless, detected**: seeds `[llm.default]` silently, reported in
///   onboard's printed output.
/// - **Headless, partial/invalid or nothing detected**: seeds nothing, returns
///   a `Skipped*` outcome whose `display_line` teaches the operator what's next.
///
/// Menu step (via [`super::prompts::PromptSource::provider_menu`]): prompts for
/// provider, API key (if required), model override, then persists.
/// - Both prompts run BEFORE any write (pure reads, nothing durable), so the
///   only fallible steps left are the two durable writes.
/// - Write order: secret store (`LlmKeyStore`, key `llm_provider_<id>_api_key`
///   — same handle webui2 settings writes and `apply_startup_stored_llm_key`
///   reads at boot) FIRST, then `[llm.default]` in `config.toml` SECOND
///   (`RebornProviderAdmin::set_provider`, same machinery `models set-provider`
///   uses). Invariant: `config.toml` can never point at a provider whose key
///   failed to persist — a `put` failure aborts before `set_provider` runs,
///   and no prompt runs after the store write starts (no orphan key on a
///   later prompt failure).
/// - `api_key_required: false` menu entries skip the key prompt/store write
///   entirely; every menu-eligible provider today (including `nearai`, via a
///   menu-level override) requires a key, so this is currently unreachable
///   but stays for future entries.
///
/// Idempotent no-op on a rerun where `[llm.default]` is already configured AND
/// (no key required OR store already has one), unless `--force` — see
/// [`already_configured_outcome`]. Covers env-seeded slots too: drift between
/// slot and a since-changed environment is accepted, not re-detected;
/// `--force` re-seeds from environment again.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
pub(crate) fn provision_llm_credentials(
    _home: &RebornHome,
    boot: &ironclaw_reborn_config::RebornBootConfig,
    prompts: &mut dyn PromptSource,
    store_opener: &dyn LlmKeyStoreOpener,
    probe: &dyn LlmProbe,
    force: bool,
) -> Result<LlmCredentialProvisionOutcome, LlmCredentialPromptError> {
    let admin = ironclaw_reborn_composition::RebornProviderAdmin::new(boot.clone());
    // Secret-store root MUST match what `serve` opens at boot
    // (`local_runtime_storage_root`, i.e. `<home>/<profile-subdir>`), NOT the
    // bare home — a key written to the bare-root db is invisible to the
    // runtime (live bug: onboarded key never reached chat turns).
    // NOTE: the directory itself is only created lazily, right before a store
    // is actually opened (see `open_llm_key_store`) — a headless/no-op
    // onboard run must not touch the filesystem.
    let store_root = crate::runtime::local_runtime_storage_root(boot, boot.profile());

    if !force && let Some(outcome) = already_configured_outcome(&admin, &store_root, store_opener)?
    {
        return Ok(outcome);
    }

    if !prompts.is_interactive() {
        return provision_headless_from_env(&store_root, store_opener, &admin);
    }

    match admin.detect_env_llm() {
        Ok(Some(detected)) => {
            let question = format!(
                "Found `{}` configured in environment — use it?",
                detected.provider_id
            );
            if prompts.confirm(&question)? {
                persist_env_detected_key(&store_root, store_opener, &admin, &detected.provider_id)?;
                let write_outcome = admin
                    .set_provider(&detected.provider_id, Some(detected.model.as_str()))
                    .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
                return Ok(LlmCredentialProvisionOutcome::ConfiguredFromEnv {
                    provider_id: write_outcome.provider_id,
                    model: write_outcome.model,
                });
            }
        }
        Ok(None) => {}
        Err(error) => {
            println!("ignoring partial environment LLM config: {error}");
        }
    }

    provision_via_menu(&store_root, &admin, prompts, store_opener, probe)
}

/// Headless counterpart of the env-detect step in [`provision_llm_credentials`]'s
/// doc: no prompt possible, so a detected config is seeded silently; anything
/// else seeds nothing and returns a `Skipped*` outcome.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn provision_headless_from_env(
    store_root: &Path,
    store_opener: &dyn LlmKeyStoreOpener,
    admin: &ironclaw_reborn_composition::RebornProviderAdmin,
) -> Result<LlmCredentialProvisionOutcome, LlmCredentialPromptError> {
    match admin.detect_env_llm() {
        Ok(Some(detected)) => {
            persist_env_detected_key(store_root, store_opener, admin, &detected.provider_id)?;
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

/// Persist the env-detected key for `provider_id` into the encrypted secret
/// store — used by both the interactive confirm-yes and headless env-seed
/// branches above. The installed service inherits only
/// `IRONCLAW_REBORN_HOME`, not the shell env that ran onboard, so a key left
/// only in the env var is invisible to it at boot; the store is the only
/// channel that reaches the daemon. A no-op when the env no longer resolves
/// a key for `provider_id` (keyless provider, or the env changed between
/// `detect_env_llm` and this call) — never writes an empty/wrong value.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn persist_env_detected_key(
    store_root: &Path,
    store_opener: &dyn LlmKeyStoreOpener,
    admin: &ironclaw_reborn_composition::RebornProviderAdmin,
    provider_id: &str,
) -> Result<(), LlmCredentialPromptError> {
    let Some(key) = admin
        .resolve_env_api_key(provider_id)
        .map_err(|error| LlmCredentialPromptError::Other(error.into()))?
    else {
        return Ok(());
    };
    let store =
        open_llm_key_store(store_root, store_opener).map_err(LlmCredentialPromptError::Other)?;
    let provider_id = provider_id.to_string();
    crate::runtime::block_on_cli(async move {
        store
            .put(&provider_id, key)
            .await
            .map_err(anyhow::Error::from)
    })
    .map_err(LlmCredentialPromptError::Other)
}

/// Create `store_root` (if missing) then open the encrypted key store there.
/// Deferred to just before a store is actually needed — see
/// [`provision_llm_credentials`]'s doc — so a headless/no-op onboard run that
/// never touches the store leaves the filesystem untouched.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn open_llm_key_store(
    store_root: &Path,
    store_opener: &dyn LlmKeyStoreOpener,
) -> anyhow::Result<ironclaw_reborn_composition::LlmKeyStore> {
    std::fs::create_dir_all(store_root).map_err(|error| {
        anyhow::anyhow!("create secret-store root {}: {error}", store_root.display())
    })?;
    store_opener.open(store_root)
}

/// Drives the full numbered provider menu, factored out so the "declined
/// confirm" and "nothing detected" branches share one implementation. See
/// [`provision_llm_credentials`]'s doc for the store-then-config write ordering.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn provision_via_menu(
    store_root: &Path,
    admin: &ironclaw_reborn_composition::RebornProviderAdmin,
    prompts: &mut dyn PromptSource,
    store_opener: &dyn LlmKeyStoreOpener,
    probe: &dyn LlmProbe,
) -> Result<LlmCredentialProvisionOutcome, LlmCredentialPromptError> {
    let entries = admin
        .menu_entries()
        .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
    let selection = prompts.provider_menu(&entries)?;
    // Re-resolve to keep this call site's id agreeing with `set_provider`'s
    // own resolution (menu offers canonical ids already, but stay consistent).
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

    // Both prompts run BEFORE any write (pure reads), so only the two durable
    // writes below remain fallible — no prompt can fail with a secret already
    // committed. See `provision_llm_credentials`'s doc for write ordering.
    let initial_key = if entry.api_key_required {
        let key = prompts.api_key(&canonical_provider_id)?;
        // Defense in depth: guards every `PromptSource` impl against a blank
        // key reaching the secret store (`StdinPromptSource` already re-prompts).
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

    // Live key/model verification — key-required providers only. Paths that
    // never reach this function (headless seeding, env-detect confirm-yes)
    // are never probed — env-sourced/keyless credentials are already trusted.
    // `nearai` is `api_key_required: true` here (menu-level override), so it
    // takes this same branch like any other key-required provider.
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
        let store = open_llm_key_store(store_root, store_opener)
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
/// - `probe`'s outcome (`ProviderProbeOutcome`) carries a single `ok: bool`
///   with no auth-vs-transport signal, so every failure (rejected key or
///   unreachable endpoint) takes the same branch: show the provider's
///   message, ask "store anyway?" ([`PromptSource::confirm`]). Yes accepts
///   the key as-is; no reprompts, up to `MAX_PROBE_ATTEMPTS` total entries.
/// - Successful probe with a non-empty model list missing `effective_model`
///   prints a warning but still returns the key (provider lists are often
///   incomplete); an empty model list warns about nothing.
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
                 configure it later with `ironclaw models set-provider {provider_id}`"
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

/// `Some` when `[llm.default]` already names a provider AND is durably
/// credentialed (no API key required per [`provider_api_key_required`]'s
/// menu-level definition, or the secret store already has a key) — the
/// idempotent-rerun case [`provision_llm_credentials`] must skip prompting
/// for.
/// - A bare stub-seeded `[llm.default]` with no stored key for a key-requiring
///   provider does NOT count (never actually credentialed) — a later
///   interactive rerun must still prompt.
/// - `nearai` is `api_key_required: true` at the menu level, so a `nearai`
///   slot with no stored/env key does NOT count either — see
///   `provision_llm_credentials_nearai_slot_without_a_stored_key_is_not_already_configured`.
/// - Store-open failure → "can't tell", falls through to prompting (not a
///   hard error) — deliberate, unlike the two failures below.
/// - Registry lookup failure and `config.toml` LOAD failure (unparseable
///   TOML) both propagate instead of being swallowed to "can't tell": a
///   corrupt/unreadable config is a real failure onboard must surface, not
///   silently reinterpret as "never configured" (which would re-run the
///   prompt, or re-write credentials on `--force`, every time against a
///   config the operator needs to fix by hand). Matches
///   [`provider_api_key_required`]'s registry-lookup-failure precedent.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn already_configured_outcome(
    admin: &ironclaw_reborn_composition::RebornProviderAdmin,
    store_root: &Path,
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

    let store = match open_llm_key_store(store_root, store_opener) {
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
/// overridden to `true` since reborn has no session-token auth wired). Not
/// menu-restricted otherwise — `[llm.default]` may name a provider excluded
/// from the onboard menu, e.g. one set via `models set-provider`.
///
/// - `Err`: registry lookup itself failed (corrupt/unreadable `providers.json`)
///   — a real failure, must not be swallowed into a silent re-prompt (would
///   make `already_configured_outcome` treat a broken registry as "never
///   configured" and re-run the prompt, or re-write credentials on `--force`,
///   every time).
/// - `Ok(None)`: genuinely "can't tell" — `provider_id` isn't in the registry.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn provider_api_key_required(
    admin: &ironclaw_reborn_composition::RebornProviderAdmin,
    provider_id: &str,
) -> Result<Option<bool>, LlmCredentialPromptError> {
    admin
        .effective_api_key_required(provider_id)
        .map_err(|error| LlmCredentialPromptError::Other(error.into()))
}

/// No `libsql`/`root-llm-provider`, nothing to write to — same reasoning as
/// `provision_master_key`'s not-any-storage-feature fallback.
#[cfg(not(all(feature = "libsql", feature = "root-llm-provider")))]
pub(crate) fn provision_llm_credentials(
    home: &RebornHome,
    _boot: &ironclaw_reborn_config::RebornBootConfig,
    _prompts: &mut dyn PromptSource,
    _store_opener: &dyn LlmKeyStoreOpener,
    _probe: &dyn LlmProbe,
    _force: bool,
) -> Result<LlmCredentialProvisionOutcome, LlmCredentialPromptError> {
    // The full provider path validates the persisted config while checking
    // whether credentials are already configured. Preserve that fail-closed
    // behavior in feature-reduced builds even though they cannot provision an
    // LLM: onboarding must never report success over a corrupt config.toml.
    let _ = ironclaw_reborn_config::RebornConfigFile::load(&home.config_file_path())
        .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
    Ok(LlmCredentialProvisionOutcome::UnavailableInBuild)
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

    /// A [`PromptSource`] whose prompt methods panic if called — proves an
    /// idempotent rerun skips prompting entirely, not merely tolerates a
    /// repeated answer.
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

    /// An [`LlmProbe`] that always reports success with no model list — used
    /// by tests unrelated to the probe-driven reprompt loop, so their
    /// `Configured`/store-write assertions stay unaffected by probing. Also
    /// pins "empty model list is not warned about".
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

    /// An [`LlmProbe`] that panics if called — proves the probe never runs
    /// on excluded paths: an idempotent already-configured rerun, a headless
    /// run, an env-detect confirm-yes branch, and a blank-key rejection
    /// (fails before the probe step). Every menu-eligible provider requires
    /// a key today (including `nearai`), so there's no "no-key provider" case.
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
    /// Args a single [`ScriptedProbe::probe`] call was made with — recorded
    /// so a test can assert the selected provider/key/model actually reached
    /// the probe, not just that a probe happened.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RecordedProbeCall {
        provider_id: String,
        api_key: Option<String>,
        model: Option<String>,
    }

    struct ScriptedProbe {
        outcomes: std::cell::RefCell<
            std::collections::VecDeque<ironclaw_reborn_composition::ProviderProbeOutcome>,
        >,
        calls: std::cell::RefCell<Vec<RecordedProbeCall>>,
    }

    impl ScriptedProbe {
        fn new(outcomes: Vec<ironclaw_reborn_composition::ProviderProbeOutcome>) -> Self {
            Self {
                outcomes: std::cell::RefCell::new(outcomes.into()),
                calls: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<RecordedProbeCall> {
            self.calls.borrow().clone()
        }
    }

    impl LlmProbe for ScriptedProbe {
        fn probe(
            &self,
            _admin: &ironclaw_reborn_composition::RebornProviderAdmin,
            provider_id: &str,
            api_key: Option<&str>,
            model: Option<&str>,
        ) -> anyhow::Result<ironclaw_reborn_composition::ProviderProbeOutcome> {
            self.calls.borrow_mut().push(RecordedProbeCall {
                provider_id: provider_id.to_string(),
                api_key: api_key.map(str::to_string),
                model: model.map(str::to_string),
            });
            self.outcomes
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("ScriptedProbe: no more scripted outcomes"))
        }
    }

    /// A [`PromptSource`] for probe-driven reprompt tests: scripts a fixed
    /// sequence of `api_key()`/`confirm()` answers, consumed in order; panics
    /// if either sequence is exhausted, proving the loop asks exactly as many
    /// times as expected.
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

    /// A fake interactive `PromptSource` selecting `openai` (key-requiring)
    /// and answering `"sk-test-value"` must land the provider selection in
    /// `config.toml` and the key in the encrypted secret store, readable back
    /// through a *fresh* open of the same root — proving the opener and
    /// `LlmKeyStore::put`/`read` agree on physical storage.
    ///
    /// Also proves the idempotent-rerun guard for a key-requiring provider: a
    /// second call with `PanickingPromptSource` must return `AlreadyConfigured`
    /// without ever calling `provider_menu()`/`api_key()`.
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
            &EncryptedLlmKeyStoreOpener,
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

        // Verify through the RUNTIME storage root (`<home>/local-dev`) — the same
        // db `serve` opens at boot; pins the onboard-write/serve-read convergence.
        let home_path = home.path().join("local-dev");
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

        // A rerun with an already-configured provider + stored key must skip
        // prompting entirely.
        let mut second_prompts = PanickingPromptSource;
        let second_outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut second_prompts,
            &EncryptedLlmKeyStoreOpener,
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
            &EncryptedLlmKeyStoreOpener,
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

        // Verify through the RUNTIME storage root (`<home>/local-dev`) — the same
        // db `serve` opens at boot; pins the onboard-write/serve-read convergence.
        let home_path = home.path().join("local-dev");
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
            &EncryptedLlmKeyStoreOpener,
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

    /// A `[llm.default]` slot already pointing at `nearai` (e.g. seeded
    /// directly via `set_provider`, mirroring what `models set-provider nearai`
    /// leaves behind) with NO stored key must NOT be treated as already
    /// configured — that state is broken (no session-token auth wired, so a
    /// keyless nearai slot dead-ends at the first chat turn). A rerun must
    /// fall through to the full prompt flow and land a real key.
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
            &EncryptedLlmKeyStoreOpener,
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

    /// A malformed `config.toml` (unparseable TOML) must surface as a real
    /// error from `already_configured_outcome`'s `admin.status()` call, not
    /// be swallowed to "can't tell" and silently fall through to prompting —
    /// `PanickingPromptSource` proves no prompt is ever reached.
    #[test]
    fn provision_llm_credentials_fails_loudly_on_a_malformed_config_toml() {
        let _env_guard = crate::runtime::test_env::lock_runtime_env();
        let (_tmp, context) = RebornCliContext::test_context();
        let home = context.boot_config().home();
        std::fs::create_dir_all(home.path()).expect("create reborn home");
        std::fs::write(home.config_file_path(), "not valid toml [[[").expect("write bad config");

        let mut prompts = PanickingPromptSource;
        let error = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &EncryptedLlmKeyStoreOpener,
            &PanickingProbe,
            false,
        )
        .expect_err("a malformed config.toml must surface as an error, not a silent fall-through");
        assert!(matches!(error, LlmCredentialPromptError::Other(_)));
    }

    /// A non-interactive session with no LLM environment variables set must
    /// succeed with `Ok(SkippedNonInteractive)` and write nothing:
    /// `provider_menu()`/`api_key()`/`model()`/`confirm()` are all
    /// `unreachable!()` on `NonInteractivePromptSource` (interactivity check
    /// short-circuits before any prompt), and `config.toml` must not exist
    /// afterward.
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
            &EncryptedLlmKeyStoreOpener,
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
        let store_root = crate::runtime::local_runtime_storage_root(
            context.boot_config(),
            context.boot_config().profile(),
        );
        assert!(
            !store_root.exists(),
            "a non-interactive no-op must not create the secret-store root either"
        );
    }

    /// A store whose `put` always fails must leave `config.toml` completely
    /// untouched — proves the secret is written BEFORE the provider selection.
    /// Uses `openai`; any key-requiring menu entry would exercise this
    /// equally, but a no-key provider couldn't.
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

    /// A `PromptSource` whose `api_key()` returns a whitespace-only answer
    /// (e.g. a fake without the blank-rejection retry loop
    /// `StdinPromptSource::api_key` has) must never reach the secret store —
    /// `provision_llm_credentials`'s own blank guard backstops every
    /// `PromptSource`, not just the terminal-backed one.
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
            &EncryptedLlmKeyStoreOpener,
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
                &EncryptedLlmKeyStoreOpener,
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
            &EncryptedLlmKeyStoreOpener,
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

    /// With a complete `openai` config in the environment (`OPENAI_API_KEY`
    /// set), an interactive session must ask to confirm using it, and "yes"
    /// must seed `[llm.default]` from the DETECTED provider/model via
    /// `set_provider` AND persist the detected key into the encrypted secret
    /// store. The installed service only inherits `IRONCLAW_REBORN_HOME`,
    /// not the operator's shell env, so a key left only in `OPENAI_API_KEY`
    /// is invisible to it at boot — the store is the only channel that
    /// reaches the daemon. See `provision_llm_credentials`'s doc.
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
        seed_cached_master_key(home);

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
            &EncryptedLlmKeyStoreOpener,
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

        // Verify through the RUNTIME storage root (`<home>/local-dev`) — the same
        // db `serve` opens at boot; pins the onboard-write/serve-read convergence.
        let home_path = home.path().join("local-dev");
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
        let material = stored.expect(
            "an env-detected+confirmed seed must persist the key into the secret store — a \
             service manager does not inherit the shell env, so the store is the only channel \
             that reaches the daemon",
        );
        assert_eq!(
            secrecy::ExposeSecret::expose_secret(&material),
            "sk-env-detected-value"
        );
    }

    /// A "no" answer to the confirm prompt must fall through to the full
    /// numbered menu — the decline path is not a dead end.
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
            &EncryptedLlmKeyStoreOpener,
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

    /// A non-interactive session with a complete `openai` config in the
    /// environment must seed `[llm.default]` from it SILENTLY (no prompt
    /// possible) AND persist the detected key into the encrypted secret
    /// store — the installed service inherits only `IRONCLAW_REBORN_HOME`,
    /// not the seeding shell's env, so the store is the only channel that
    /// reaches the daemon.
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
        seed_cached_master_key(home);

        let mut prompts = HeadlessPromptSource;
        let outcome = provision_llm_credentials(
            home,
            context.boot_config(),
            &mut prompts,
            &EncryptedLlmKeyStoreOpener,
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

        // Verify through the RUNTIME storage root (`<home>/local-dev`) — the
        // same db `serve` opens at boot; pins the onboard-write/serve-read
        // convergence for the headless env-seed path too.
        let home_path = home.path().join("local-dev");
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
        let material = stored.expect(
            "a headless env-seed must persist the key into the secret store — a service \
             manager does not inherit the shell env, so the store is the only channel that \
             reaches the daemon",
        );
        assert_eq!(
            secrecy::ExposeSecret::expose_secret(&material),
            "sk-env-detected-value"
        );
    }

    /// A non-interactive session with an INCOMPLETE env config (`OPENAI_MODEL`
    /// set without `OPENAI_API_KEY`) must seed nothing and report a
    /// `SkippedNonInteractivePartialEnv` outcome naming the reason — never
    /// silently adopt a broken environment or fall back to a hardcoded default.
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
            &EncryptedLlmKeyStoreOpener,
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

    /// A fresh reborn home, interactive session, clean environment (no LLM
    /// env vars) must still invoke the full numbered `provider_menu()` — an
    /// interactive session with `Ok(None)` (nothing detected) must not be
    /// treated as `SkippedNonInteractive`-shaped and skip the first-run menu.
    /// `FakePromptSource` panics on `confirm()`, so this also fails loudly if
    /// `confirm()` is spuriously invoked with nothing detected.
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
            &EncryptedLlmKeyStoreOpener,
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

    /// A probe failure (rejected key or unreachable endpoint —
    /// `ProviderProbeOutcome` carries no signal to tell them apart, see
    /// `probe_and_confirm_key`'s doc) followed by a "store anyway?" decline
    /// must reprompt for a NEW key, and a second successful probe must store
    /// THAT key — the reprompt loop replaces the candidate, not retries it.
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
            &EncryptedLlmKeyStoreOpener,
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

        // Verify through the RUNTIME storage root (`<home>/local-dev`) — the same
        // db `serve` opens at boot; pins the onboard-write/serve-read convergence.
        let home_path = home.path().join("local-dev");
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
        assert_eq!(
            probe.calls(),
            vec![
                RecordedProbeCall {
                    provider_id: "openai".to_string(),
                    api_key: Some("sk-bad".to_string()),
                    model: Some("gpt-5-mini".to_string()),
                },
                RecordedProbeCall {
                    provider_id: "openai".to_string(),
                    api_key: Some("sk-good".to_string()),
                    model: Some("gpt-5-mini".to_string()),
                },
            ],
            "each probe call must carry the SELECTED provider/model and the candidate key \
             actually being tried, not stale values"
        );
    }

    /// Three consecutive probe failures, each declined via "store anyway? no",
    /// must exhaust `MAX_PROBE_ATTEMPTS` and error out, leaving `config.toml`
    /// untouched — not loop forever or give up after a different count.
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
            &EncryptedLlmKeyStoreOpener,
            &probe,
            false,
        )
        .expect_err("three failed probe attempts, all declined, must error");
        assert!(matches!(error, LlmCredentialPromptError::Other(_)));
        assert!(
            !home.config_file_path().exists(),
            "an exhausted probe-reprompt loop must leave config.toml untouched"
        );
        assert_eq!(
            probe
                .calls()
                .into_iter()
                .map(|call| call.api_key)
                .collect::<Vec<_>>(),
            vec![
                Some("sk-1".to_string()),
                Some("sk-2".to_string()),
                Some("sk-3".to_string())
            ],
            "each of the three attempts must probe its own freshly-entered key"
        );
    }

    /// A single probe failure followed by an accepted "store anyway?" must
    /// store the key as entered without further reprompt — offline/
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
            &EncryptedLlmKeyStoreOpener,
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

        // Verify through the RUNTIME storage root (`<home>/local-dev`) — the same
        // db `serve` opens at boot; pins the onboard-write/serve-read convergence.
        let home_path = home.path().join("local-dev");
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
        assert_eq!(
            probe.calls(),
            vec![RecordedProbeCall {
                provider_id: "openai".to_string(),
                api_key: Some("sk-offline".to_string()),
                model: Some("gpt-5-mini".to_string()),
            }],
            "the single probe attempt must carry the entered key and selected model"
        );
    }

    /// A successful probe whose model list doesn't contain the chosen model
    /// must still write the key/config — an incomplete provider model list
    /// is a warning, never an error.
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
            &EncryptedLlmKeyStoreOpener,
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
        assert_eq!(
            probe.calls(),
            vec![RecordedProbeCall {
                provider_id: "openai".to_string(),
                api_key: Some("sk-test-value".to_string()),
                model: Some("not-a-real-model".to_string()),
            }],
            "the probe must be called with the operator's chosen (unlisted) model, not the \
             catalog default"
        );
    }
}
