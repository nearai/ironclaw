use super::*;

pub(super) const SLACK: &str = "slack";
pub(super) const SLACK_GROUP: &str = "extension.slack";
const MANAGED_SLACK_SUBJECT_PREFIX: &str = "user:slack-channel:";

pub(super) fn provider_key() -> &'static str {
    SLACK
}

pub(super) fn root_migration_spec() -> Rc1ChannelRootMigrationSpec {
    Rc1ChannelRootMigrationSpec {
        provider_key: SLACK,
    }
}

pub(super) fn is_source_state_path(path: &str) -> bool {
    [
        "/slack-setup/",
        "/slack-personal-binding/",
        "/slack-channel-routes/",
        "/slack-conversations/",
        "/slack-product-workflow/",
    ]
    .iter()
    .any(|segment| path.contains(segment))
}

pub(super) fn secret_handles(
    rows: &[VersionedEntry],
    shared: &str,
) -> Result<Vec<SecretHandle>, Rc1ChannelStateMigrationError> {
    let path = format!("{shared}/slack-setup/installation.json");
    let Some(row) = rows.iter().find(|row| row.path.as_str() == path) else {
        return Ok(Vec::new());
    };
    let setup: Rc1SlackSetup = parse(row)?;
    let mut handles = vec![setup.bot_token_handle, setup.signing_secret_handle];
    if let Some(handle) = setup.oauth_client_secret_handle {
        handles.push(handle);
    }
    Ok(handles)
}

pub(super) struct Prepared {
    setup: Option<Rc1SlackSetup>,
    connections: Rc1SlackConnectionDisposition,
}

pub(super) async fn prepare(
    inputs: &Rc1ChannelStateMigrationInputs,
    shared: &str,
) -> Result<Prepared, Rc1ChannelStateMigrationError> {
    let setup =
        read_optional::<Rc1SlackSetup>(inputs, &format!("{shared}/slack-setup/installation.json"))
            .await?;
    validate_source_rows(inputs, shared).await?;
    let connections =
        inspect_slack_connection_disposition(&inputs.filesystem, &inputs.admin_scope, shared)
            .await?;
    Ok(Prepared { setup, connections })
}

pub(super) async fn migrate(
    inputs: &Rc1ChannelStateMigrationInputs,
    shared: &str,
    target_installation: Option<&String>,
    prepared: Prepared,
) -> Result<Rc1ChannelStateMigrationReport, Rc1ChannelStateMigrationError> {
    let mut report = Rc1ChannelStateMigrationReport::default();
    if let Some(setup) = prepared.setup.as_ref() {
        report.configuration_values += migrate_slack_setup(inputs, setup).await?;
    }
    report.identities +=
        migrate_slack_identities(inputs, shared, prepared.setup.as_ref(), target_installation)
            .await?;
    report.route_values += migrate_slack_routes(inputs, shared, prepared.setup.as_ref()).await?;
    report.dm_targets += migrate_slack_dm_targets(inputs, shared, prepared.setup.as_ref()).await?;
    if prepared.connections.already_complete {
        report.oauth_channel_connections_unchanged = prepared.connections.marker.source_rows;
    } else {
        report.oauth_channel_active_connections_superseded =
            prepared.connections.marker.active_superseded;
        report.oauth_channel_stale_connections_expired = prepared.connections.marker.stale_expired;
        report.oauth_channel_disconnected_connections_superseded =
            prepared.connections.marker.disconnected_superseded;
    }
    commit_disposition_marker(
        &inputs.filesystem,
        &format!("{shared}/channel-extensions/slack/migrations/rc1-connections-v1.complete.json"),
        &prepared.connections.marker,
        prepared.connections.already_complete,
    )
    .await?;
    Ok(report)
}

async fn migrate_slack_setup(
    inputs: &Rc1ChannelStateMigrationInputs,
    setup: &Rc1SlackSetup,
) -> Result<usize, Rc1ChannelStateMigrationError> {
    let mut values = vec![
        submitted("slack_installation_id", setup.installation_id.clone())?,
        submitted("slack_team_id", setup.team_id.clone())?,
        submitted("slack_api_app_id", setup.api_app_id.clone())?,
        submitted("slack_bot_user_id", setup.user_id.clone())?,
        submitted_secret(
            inputs,
            &inputs.oauth_channel_secret_scope,
            "slack_bot_token",
            &setup.bot_token_handle,
        )
        .await?,
        submitted_secret(
            inputs,
            &inputs.oauth_channel_secret_scope,
            "slack_signing_secret",
            &setup.signing_secret_handle,
        )
        .await?,
    ];
    if let Some(subject) = &setup.shared_subject_user_id {
        values.push(submitted("slack_shared_subject_user_id", subject.clone())?);
    }
    if let Some(client_id) = &setup.oauth_client_id {
        values.push(submitted("slack_oauth_client_id", client_id.clone())?);
    }
    if let Some(client_secret) = &setup.oauth_client_secret_handle {
        values.push(
            submitted_secret(
                inputs,
                &inputs.oauth_channel_secret_scope,
                "slack_oauth_client_secret",
                client_secret,
            )
            .await?,
        );
    }
    import_admin(inputs, SLACK_GROUP, "rc1-slack-setup-v1", values).await
}

async fn migrate_slack_identities(
    inputs: &Rc1ChannelStateMigrationInputs,
    shared: &str,
    setup: Option<&Rc1SlackSetup>,
    target_installation: Option<&String>,
) -> Result<usize, Rc1ChannelStateMigrationError> {
    let rows = query_all(
        &inputs.filesystem,
        &format!("{shared}/slack-personal-binding/identities"),
    )
    .await?;
    let mut changed = 0;
    for row in rows {
        let record: Rc1SlackIdentity = parse(&row)?;
        if matches!(record.state, Rc1SlackIdentityState::Disconnected) {
            continue;
        }
        if let Some(epoch) = &record.epoch {
            ExternalActorBindingEpoch::new(epoch.clone()).map_err(log_malformed)?;
        }
        if record.disconnected_at.is_some() {
            return Err(Rc1ChannelStateMigrationError::Malformed);
        }
        let provider_user_id = rewrite_installation_prefix(
            &record.provider_user_id,
            setup.map(|setup| setup.installation_id.as_str()),
            target_installation.map(String::as_str),
        )?;
        changed +=
            bind_identity(inputs, &record.provider, provider_user_id, &record.user_id).await?;
    }
    Ok(changed)
}

async fn migrate_slack_routes(
    inputs: &Rc1ChannelStateMigrationInputs,
    shared: &str,
    setup: Option<&Rc1SlackSetup>,
) -> Result<usize, Rc1ChannelStateMigrationError> {
    let rows = query_all(
        &inputs.filesystem,
        &format!("{shared}/slack-channel-routes"),
    )
    .await?;
    let route_count = rows.len();
    let mut allowed = Vec::new();
    let mut explicit = BTreeMap::new();
    for row in rows {
        let route: Rc1SlackRoute = parse(&row)?;
        if route.tenant_id != inputs.admin_scope.tenant_id.as_str() {
            return Err(Rc1ChannelStateMigrationError::Malformed);
        }
        if route.deleted_at.is_some() {
            continue;
        }
        let Some(setup) = setup else {
            return Err(Rc1ChannelStateMigrationError::Malformed);
        };
        if route.installation_id != setup.installation_id || route.team_id != setup.team_id {
            return Err(Rc1ChannelStateMigrationError::Conflict);
        }
        if route
            .subject_user_id
            .starts_with(MANAGED_SLACK_SUBJECT_PREFIX)
        {
            allowed.push(route.channel_id);
        } else if let Some(previous) =
            explicit.insert(route.channel_id, route.subject_user_id.clone())
            && previous != route.subject_user_id
        {
            return Err(Rc1ChannelStateMigrationError::Conflict);
        }
    }
    allowed.sort();
    allowed.dedup();
    let mut values = Vec::new();
    if !allowed.is_empty() {
        let allowed = serde_json::to_string(&allowed).map_err(log_malformed)?;
        if allowed.len() > crate::admin_configuration_service::MAX_VALUE_BYTES {
            return Err(Rc1ChannelStateMigrationError::SourceTooLarge {
                records: route_count,
            });
        }
        values.push(submitted("slack_allowed_channels", allowed)?);
    }
    if !explicit.is_empty() {
        let explicit = serde_json::to_string(&explicit).map_err(log_malformed)?;
        if explicit.len() > crate::admin_configuration_service::MAX_VALUE_BYTES {
            return Err(Rc1ChannelStateMigrationError::SourceTooLarge {
                records: route_count,
            });
        }
        values.push(submitted("slack_subject_routes", explicit)?);
    }
    if values.is_empty() {
        return Ok(0);
    }
    import_admin(inputs, SLACK_GROUP, "rc1-slack-routes-v1", values).await
}

async fn migrate_slack_dm_targets(
    inputs: &Rc1ChannelStateMigrationInputs,
    shared: &str,
    setup: Option<&Rc1SlackSetup>,
) -> Result<usize, Rc1ChannelStateMigrationError> {
    let rows = query_all(
        &inputs.filesystem,
        &format!("{shared}/slack-personal-binding/dm-targets"),
    )
    .await?;
    let mut changed = 0;
    for row in rows {
        let target: Rc1SlackDmTarget = parse(&row)?;
        if target.tenant_id != inputs.admin_scope.tenant_id.as_str() {
            return Err(Rc1ChannelStateMigrationError::Malformed);
        }
        if target.deleted_at.is_some() {
            continue;
        }
        let Some(setup) = setup else {
            return Err(Rc1ChannelStateMigrationError::Malformed);
        };
        if target.installation_id != setup.installation_id || target.team_id != setup.team_id {
            return Err(Rc1ChannelStateMigrationError::Conflict);
        }
        let user = UserId::new(target.user_id).map_err(log_malformed)?;
        let payload = crate::dm_target_payload(Some(&target.team_id), &target.dm_channel_id);
        changed += upsert_dm_target(inputs, SLACK, &user, target.slack_user_id, payload).await?;
    }
    Ok(changed)
}

async fn validate_source_rows(
    inputs: &Rc1ChannelStateMigrationInputs,
    shared: &str,
) -> Result<(), Rc1ChannelStateMigrationError> {
    for row in query_all(
        &inputs.filesystem,
        &format!("{shared}/slack-personal-binding/identities"),
    )
    .await?
    {
        let _: Rc1SlackIdentity = parse(&row)?;
    }
    for row in query_all(
        &inputs.filesystem,
        &format!("{shared}/slack-channel-routes"),
    )
    .await?
    {
        let _: Rc1SlackRoute = parse(&row)?;
    }
    for row in query_all(
        &inputs.filesystem,
        &format!("{shared}/slack-personal-binding/dm-targets"),
    )
    .await?
    {
        let _: Rc1SlackDmTarget = parse(&row)?;
    }
    Ok(())
}

pub(super) async fn inspect_slack_connection_disposition(
    filesystem: &Arc<dyn RootFilesystem>,
    admin_scope: &ResourceScope,
    shared: &str,
) -> Result<Rc1SlackConnectionDisposition, Rc1ChannelStateMigrationError> {
    let rows = query_all(
        filesystem,
        &format!("{shared}/slack-personal-binding/connections"),
    )
    .await?;
    let mut active_superseded = 0usize;
    let mut stale_expired = 0usize;
    let mut disconnected_superseded = 0usize;
    for row in &rows {
        let connection: Rc1SlackConnection = parse(row)?;
        if connection.tenant_id != admin_scope.tenant_id.as_str() {
            return Err(Rc1ChannelStateMigrationError::Malformed);
        }
        let user = UserId::new(&connection.user_id).map_err(log_malformed)?;
        AdapterInstallationId::new(&connection.installation_id).map_err(log_malformed)?;
        let expected_suffix = format!("/{}/{}.json", connection.installation_id, user.as_str());
        if !row.path.as_str().ends_with(&expected_suffix) {
            return Err(Rc1ChannelStateMigrationError::Malformed);
        }
        match connection.state {
            Rc1SlackConnectionState::Connecting => {
                if connection.disconnect_cleanup.is_some() {
                    return Err(Rc1ChannelStateMigrationError::Malformed);
                }
                stale_expired = stale_expired.saturating_add(1);
            }
            Rc1SlackConnectionState::Active => {
                if connection.disconnect_cleanup.is_some() {
                    return Err(Rc1ChannelStateMigrationError::Malformed);
                }
                active_superseded = active_superseded.saturating_add(1);
            }
            Rc1SlackConnectionState::Disconnecting => {
                return Err(Rc1ChannelStateMigrationError::InterruptedSetup);
            }
            Rc1SlackConnectionState::Disconnected => {
                if connection.disconnect_cleanup.is_some() {
                    return Err(Rc1ChannelStateMigrationError::Malformed);
                }
                disconnected_superseded = disconnected_superseded.saturating_add(1);
            }
        }
    }
    let marker = Rc1SlackConnectionDispositionMarker {
        schema: "rc1-slack-connections-v1".to_string(),
        source_digest: source_rows_digest(&rows),
        active_superseded,
        stale_expired,
        disconnected_superseded,
        source_rows: rows.len(),
    };
    let marker_path =
        format!("{shared}/channel-extensions/slack/migrations/rc1-connections-v1.complete.json");
    let already_complete = disposition_marker_matches(filesystem, &marker_path, &marker).await?;
    Ok(Rc1SlackConnectionDisposition {
        marker,
        already_complete,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rc1SlackSetup {
    pub(super) installation_id: String,
    pub(super) team_id: String,
    pub(super) api_app_id: String,
    pub(super) user_id: String,
    #[serde(default)]
    pub(super) shared_subject_user_id: Option<String>,
    pub(super) bot_token_handle: SecretHandle,
    pub(super) signing_secret_handle: SecretHandle,
    #[serde(default)]
    pub(super) oauth_client_id: Option<String>,
    #[serde(default)]
    pub(super) oauth_client_secret_handle: Option<SecretHandle>,
    pub(super) revision: u64,
    pub(super) updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Rc1SlackIdentityState {
    Active,
    Disconnected,
}

fn active_slack_identity() -> Rc1SlackIdentityState {
    Rc1SlackIdentityState::Active
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rc1SlackIdentity {
    pub(super) provider: String,
    pub(super) provider_user_id: String,
    pub(super) user_id: String,
    #[serde(default)]
    pub(super) epoch: Option<String>,
    #[serde(default = "active_slack_identity")]
    pub(super) state: Rc1SlackIdentityState,
    #[serde(default)]
    pub(super) disconnected_at: Option<DateTime<Utc>>,
    pub(super) created_at: DateTime<Utc>,
    pub(super) updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rc1SlackRoute {
    pub(super) tenant_id: String,
    pub(super) installation_id: String,
    pub(super) team_id: String,
    pub(super) channel_id: String,
    pub(super) subject_user_id: String,
    pub(super) updated_at: DateTime<Utc>,
    #[serde(default)]
    pub(super) deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rc1SlackDmTarget {
    pub(super) tenant_id: String,
    pub(super) installation_id: String,
    pub(super) team_id: String,
    pub(super) user_id: String,
    pub(super) slack_user_id: String,
    pub(super) dm_channel_id: String,
    #[serde(default)]
    pub(super) epoch: Option<String>,
    #[serde(default)]
    pub(super) deleted_at: Option<DateTime<Utc>>,
    pub(super) created_at: DateTime<Utc>,
    pub(super) updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Rc1SlackConnectionState {
    Connecting,
    Active,
    Disconnecting,
    Disconnected,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", content = "epoch", rename_all = "snake_case")]
pub(super) enum Rc1SlackDisconnectCleanup {
    AllOwned,
    Epoch(ironclaw_auth::AuthFlowId),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rc1SlackConnection {
    pub(super) tenant_id: String,
    pub(super) user_id: String,
    pub(super) installation_id: String,
    pub(super) epoch: ironclaw_auth::AuthFlowId,
    pub(super) state: Rc1SlackConnectionState,
    #[serde(default)]
    pub(super) disconnect_cleanup: Option<Rc1SlackDisconnectCleanup>,
    pub(super) expires_at: DateTime<Utc>,
    pub(super) created_at: DateTime<Utc>,
    pub(super) updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Rc1SlackConnectionDispositionMarker {
    pub(super) schema: String,
    pub(super) source_digest: String,
    pub(super) active_superseded: usize,
    pub(super) stale_expired: usize,
    pub(super) disconnected_superseded: usize,
    pub(super) source_rows: usize,
}

#[derive(Debug)]
pub(super) struct Rc1SlackConnectionDisposition {
    pub(super) marker: Rc1SlackConnectionDispositionMarker,
    pub(super) already_complete: bool,
}
