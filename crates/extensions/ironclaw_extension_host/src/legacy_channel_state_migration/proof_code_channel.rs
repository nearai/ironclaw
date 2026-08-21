use super::*;

pub(super) const TELEGRAM: &str = "telegram";
pub(super) const TELEGRAM_GROUP: &str = "extension.telegram";

pub(super) fn provider_key() -> &'static str {
    TELEGRAM
}

pub(super) fn root_migration_spec() -> Rc1ChannelRootMigrationSpec {
    Rc1ChannelRootMigrationSpec {
        provider_key: TELEGRAM,
    }
}

pub(super) fn is_source_state_path(path: &str) -> bool {
    [
        "/telegram-setup/",
        "/telegram-binding/",
        "/telegram-dm-targets/",
        "/telegram-pairing/",
        "/telegram-conversations/",
        "/telegram-product-workflow/",
    ]
    .iter()
    .any(|segment| path.contains(segment))
}

pub(super) fn secret_handles(
    rows: &[VersionedEntry],
    shared: &str,
) -> Result<Vec<SecretHandle>, Rc1ChannelStateMigrationError> {
    let path = format!("{shared}/telegram-setup/installation.json");
    let Some(row) = rows.iter().find(|row| row.path.as_str() == path) else {
        return Ok(Vec::new());
    };
    let setup: Rc1TelegramSetup = parse(row)?;
    Ok(match setup {
        Rc1TelegramSetup::Active(setup)
        | Rc1TelegramSetup::Lifecycle(Rc1TelegramSetupLifecycle::Clearing { setup }) => {
            vec![setup.bot_token_handle, setup.webhook_secret_handle]
        }
        Rc1TelegramSetup::Lifecycle(Rc1TelegramSetupLifecycle::RollingBack { saved, .. }) => {
            vec![saved.bot_token_handle, saved.webhook_secret_handle]
        }
        Rc1TelegramSetup::Lifecycle(Rc1TelegramSetupLifecycle::Cleared { .. }) => Vec::new(),
    })
}

pub(super) struct Prepared {
    setup: Option<Rc1TelegramSetup>,
    pairing: Rc1TelegramPairingDisposition,
}

pub(super) async fn prepare(
    inputs: &Rc1ChannelStateMigrationInputs,
    shared: &str,
) -> Result<Prepared, Rc1ChannelStateMigrationError> {
    let setup = read_optional::<Rc1TelegramSetup>(
        inputs,
        &format!("{shared}/telegram-setup/installation.json"),
    )
    .await?;
    match setup.as_ref() {
        Some(Rc1TelegramSetup::Lifecycle(
            Rc1TelegramSetupLifecycle::Clearing { .. }
            | Rc1TelegramSetupLifecycle::RollingBack { .. },
        )) => return Err(Rc1ChannelStateMigrationError::InterruptedSetup),
        Some(Rc1TelegramSetup::Active(_))
        | Some(Rc1TelegramSetup::Lifecycle(Rc1TelegramSetupLifecycle::Cleared { .. }))
        | None => {}
    }
    validate_source_rows(inputs, shared).await?;
    let pairing =
        inspect_telegram_pairing_disposition(&inputs.filesystem, &inputs.admin_scope, shared)
            .await?;
    Ok(Prepared { setup, pairing })
}

pub(super) async fn migrate(
    inputs: &Rc1ChannelStateMigrationInputs,
    shared: &str,
    target_installation: Option<&String>,
    prepared: Prepared,
) -> Result<Rc1ChannelStateMigrationReport, Rc1ChannelStateMigrationError> {
    let active = match prepared.setup.as_ref() {
        Some(Rc1TelegramSetup::Active(setup)) => Some(setup),
        Some(Rc1TelegramSetup::Lifecycle(Rc1TelegramSetupLifecycle::Cleared { .. })) | None => None,
        Some(Rc1TelegramSetup::Lifecycle(
            Rc1TelegramSetupLifecycle::Clearing { .. }
            | Rc1TelegramSetupLifecycle::RollingBack { .. },
        )) => return Err(Rc1ChannelStateMigrationError::InterruptedSetup),
    };
    let mut report = Rc1ChannelStateMigrationReport::default();
    if let Some(setup) = active {
        report.configuration_values += migrate_telegram_setup(inputs, setup).await?;
    }
    let bindings = migrate_telegram_identities(inputs, shared, active, target_installation).await?;
    report.identities += bindings.changed;
    let (dm_targets, skipped) =
        migrate_telegram_dm_targets(inputs, shared, active, &bindings.active).await?;
    report.dm_targets += dm_targets;
    report.unbound_dm_targets_skipped += skipped;
    if prepared.pairing.already_complete {
        report.proof_code_pairing_rows_unchanged = prepared.pairing.marker.source_rows;
    } else {
        report.proof_code_pairing_challenges_expired = prepared.pairing.marker.challenges_expired;
        report.proof_code_pending_completions_expired =
            prepared.pairing.marker.pending_completions_expired;
    }
    commit_disposition_marker(
        &inputs.filesystem,
        &format!("{shared}/channel-extensions/telegram/migrations/rc1-pairing-v1.complete.json"),
        &prepared.pairing.marker,
        prepared.pairing.already_complete,
    )
    .await?;
    Ok(report)
}

async fn migrate_telegram_setup(
    inputs: &Rc1ChannelStateMigrationInputs,
    setup: &Rc1TelegramSetupActive,
) -> Result<usize, Rc1ChannelStateMigrationError> {
    import_admin(
        inputs,
        TELEGRAM_GROUP,
        "rc1-telegram-setup-v1",
        vec![
            submitted("bot_username", setup.bot_username.clone())?,
            submitted("telegram_webhook_url", setup.webhook_url.clone())?,
            submitted_secret(
                inputs,
                &inputs.proof_code_channel_secret_scope,
                "telegram_bot_token",
                &setup.bot_token_handle,
            )
            .await?,
            submitted_secret(
                inputs,
                &inputs.proof_code_channel_secret_scope,
                "telegram_webhook_secret",
                &setup.webhook_secret_handle,
            )
            .await?,
        ],
    )
    .await
}

struct TelegramBindingImport {
    changed: usize,
    active: Vec<Rc1TelegramBinding>,
}

async fn migrate_telegram_identities(
    inputs: &Rc1ChannelStateMigrationInputs,
    shared: &str,
    setup: Option<&Rc1TelegramSetupActive>,
    target_installation: Option<&String>,
) -> Result<TelegramBindingImport, Rc1ChannelStateMigrationError> {
    let rows = query_all(
        &inputs.filesystem,
        &format!("{shared}/telegram-binding/identities"),
    )
    .await?;
    let old_installation = setup.map(|setup| format!("tg-bot-{}", setup.bot_id));
    let mut changed = 0;
    let mut active = Vec::new();
    for row in rows {
        let record: Rc1TelegramBinding = parse(&row)?;
        if !record.active {
            continue;
        }
        ExternalActorBindingEpoch::new(record.epoch.clone()).map_err(log_malformed)?;
        let provider_user_id = rewrite_installation_prefix(
            &record.provider_user_id,
            old_installation.as_deref(),
            target_installation.map(String::as_str),
        )?;
        changed += bind_identity(inputs, TELEGRAM, provider_user_id, &record.user_id).await?;
        active.push(record);
    }
    Ok(TelegramBindingImport { changed, active })
}

async fn migrate_telegram_dm_targets(
    inputs: &Rc1ChannelStateMigrationInputs,
    shared: &str,
    setup: Option<&Rc1TelegramSetupActive>,
    bindings: &[Rc1TelegramBinding],
) -> Result<(usize, usize), Rc1ChannelStateMigrationError> {
    let rows = query_all(&inputs.filesystem, &format!("{shared}/telegram-dm-targets")).await?;
    let old_installation = setup.map(|setup| format!("tg-bot-{}", setup.bot_id));
    let mut changed = 0;
    let mut skipped = 0usize;
    for row in rows {
        let target: Rc1TelegramDmTarget = parse(&row)?;
        let candidates = bindings
            .iter()
            .filter(|binding| binding.user_id == target.user_id.as_str())
            .filter_map(|binding| {
                let (installation, actor) = binding.provider_user_id.split_once(':')?;
                old_installation
                    .as_deref()
                    .is_none_or(|expected| expected == installation)
                    .then_some(actor)
            })
            .collect::<Vec<_>>();
        let actor = match candidates.as_slice() {
            [actor] => *actor,
            [] => {
                skipped = skipped.saturating_add(1);
                continue;
            }
            _ => return Err(Rc1ChannelStateMigrationError::Conflict),
        };
        changed += upsert_dm_target(
            inputs,
            TELEGRAM,
            &target.user_id,
            actor.to_string(),
            crate::dm_target_payload(None, &target.chat_id.to_string()),
        )
        .await?;
    }
    Ok((changed, skipped))
}

async fn validate_source_rows(
    inputs: &Rc1ChannelStateMigrationInputs,
    shared: &str,
) -> Result<(), Rc1ChannelStateMigrationError> {
    for row in query_all(
        &inputs.filesystem,
        &format!("{shared}/telegram-binding/identities"),
    )
    .await?
    {
        let _: Rc1TelegramBinding = parse(&row)?;
    }
    for row in query_all(&inputs.filesystem, &format!("{shared}/telegram-dm-targets")).await? {
        let _: Rc1TelegramDmTarget = parse(&row)?;
    }
    Ok(())
}

pub(super) async fn inspect_telegram_pairing_disposition(
    filesystem: &Arc<dyn RootFilesystem>,
    admin_scope: &ResourceScope,
    shared: &str,
) -> Result<Rc1TelegramPairingDisposition, Rc1ChannelStateMigrationError> {
    let codes = query_all(filesystem, &format!("{shared}/telegram-pairing/codes")).await?;
    let mut challenges = 0;
    for row in &codes {
        let record: Rc1TelegramPairingRecord = parse(row)?;
        if record.tenant_id != admin_scope.tenant_id {
            return Err(Rc1ChannelStateMigrationError::Malformed);
        }
        AdapterInstallationId::new(&record.installation_id).map_err(log_malformed)?;
        if record.consumed_at.is_none() {
            challenges += 1;
        }
    }
    let users = query_all(filesystem, &format!("{shared}/telegram-pairing/users")).await?;
    for row in &users {
        let _: Rc1TelegramPairingPointer = parse(row)?;
    }
    let mut completions = 0;
    let pending_completions = query_all(
        filesystem,
        &format!("{shared}/telegram-pairing/pending-completions"),
    )
    .await?;
    for row in &pending_completions {
        let completion: Rc1TelegramPairingCompletion = parse(row)?;
        AdapterInstallationId::new(&completion.installation_id).map_err(log_malformed)?;
        if !completion.completed {
            completions += 1;
        }
    }
    let mut all_rows = codes;
    all_rows.extend(users);
    all_rows.extend(pending_completions);
    let marker = Rc1TelegramPairingDispositionMarker {
        schema: "rc1-telegram-pairing-v1".to_string(),
        source_digest: source_rows_digest(&all_rows),
        challenges_expired: challenges,
        pending_completions_expired: completions,
        source_rows: all_rows.len(),
    };
    let marker_path =
        format!("{shared}/channel-extensions/telegram/migrations/rc1-pairing-v1.complete.json");
    let already_complete = disposition_marker_matches(filesystem, &marker_path, &marker).await?;
    Ok(Rc1TelegramPairingDisposition {
        marker,
        already_complete,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Rc1TelegramPairingDispositionMarker {
    pub(super) schema: String,
    pub(super) source_digest: String,
    pub(super) challenges_expired: usize,
    pub(super) pending_completions_expired: usize,
    pub(super) source_rows: usize,
}

#[derive(Debug)]
pub(super) struct Rc1TelegramPairingDisposition {
    pub(super) marker: Rc1TelegramPairingDispositionMarker,
    pub(super) already_complete: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rc1TelegramSetupActive {
    pub(super) bot_id: i64,
    pub(super) bot_username: String,
    pub(super) webhook_url: String,
    pub(super) bot_token_handle: SecretHandle,
    pub(super) webhook_secret_handle: SecretHandle,
    pub(super) revision: u64,
    pub(super) updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "lifecycle", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Rc1TelegramSetupLifecycle {
    Clearing {
        setup: Rc1TelegramSetupActive,
    },
    RollingBack {
        saved: Rc1TelegramSetupActive,
        previous: Option<Rc1TelegramSetupActive>,
        #[serde(default)]
        provider_compensated: bool,
    },
    Cleared {
        cleared_revision: u64,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum Rc1TelegramSetup {
    Active(Rc1TelegramSetupActive),
    Lifecycle(Rc1TelegramSetupLifecycle),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rc1TelegramBinding {
    pub(super) provider_user_id: String,
    pub(super) user_id: String,
    pub(super) epoch: String,
    #[serde(default = "default_true")]
    pub(super) active: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rc1TelegramDmTarget {
    pub(super) user_id: UserId,
    pub(super) chat_id: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rc1TelegramPairingRecord {
    pub(super) code: ChannelPairingCode,
    pub(super) tenant_id: TenantId,
    pub(super) user_id: UserId,
    pub(super) installation_id: String,
    pub(super) created_at: DateTime<Utc>,
    pub(super) expires_at: DateTime<Utc>,
    pub(super) consumed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rc1TelegramPairingPointer {
    pub(super) code: ChannelPairingCode,
    #[serde(default = "default_true")]
    pub(super) active: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rc1TelegramPairingCompletion {
    pub(super) installation_id: String,
    pub(super) user_id: UserId,
    pub(super) chat_id: i64,
    pub(super) completed: bool,
}

fn default_true() -> bool {
    true
}
