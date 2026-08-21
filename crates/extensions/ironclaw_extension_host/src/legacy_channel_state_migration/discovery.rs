use super::*;

/// One durable tenant and the released secret-owner scopes needed to import
/// its provider setup without guessing the configured default owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rc1ChannelMigrationScope {
    pub admin_scope: ResourceScope,
    pub oauth_channel_secret_scope: ResourceScope,
    pub proof_code_channel_secret_scope: ResourceScope,
}

/// Discover all released channel-state tenants and the exact owner scopes of
/// their setup secrets from durable paths. This is intentionally independent
/// of the configured default owner: hosted rc1 could persist multiple tenants
/// in one backend.
pub async fn discover_rc1_channel_migration_scopes(
    filesystem: Arc<dyn RootFilesystem>,
) -> Result<Vec<Rc1ChannelMigrationScope>, Rc1ChannelStateMigrationError> {
    let tenants_root = VirtualPath::new("/tenants").map_err(log_malformed)?;
    let tenant_entries = match filesystem
        .list_dir_bounded(&tenants_root, MAX_RC1_CHANNEL_TENANTS.saturating_add(1))
        .await
    {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
        Err(error) => return Err(log_unavailable(error)),
    };
    if tenant_entries.len() > MAX_RC1_CHANNEL_TENANTS {
        tracing::debug!(
            root = %tenants_root,
            limit = MAX_RC1_CHANNEL_TENANTS,
            observed = tenant_entries.len(),
            "rc1 channel migration tenant discovery bound exceeded"
        );
        return Err(Rc1ChannelStateMigrationError::Unavailable);
    }
    let mut scopes = Vec::new();
    for tenant_entry in tenant_entries {
        if tenant_entry.file_type != FileType::Directory || tenant_entry.name == "__system__" {
            continue;
        }
        let tenant_segment = tenant_entry.name;
        let shared = format!("/tenants/{tenant_segment}/shared");
        let rows = query_all(&filesystem, &shared).await?;
        if !rows
            .iter()
            .any(|row| is_rc1_channel_state_path(row.path.as_str()))
        {
            continue;
        }
        let tenant = TenantId::new(&tenant_segment).map_err(log_malformed)?;
        let oauth_handles = oauth_channel::secret_handles(&rows, &shared)?;
        let proof_code_handles = proof_code_channel::secret_handles(&rows, &shared)?;
        let admin_scope = ResourceScope {
            tenant_id: tenant.clone(),
            user_id: UserId::from_trusted(SYSTEM_RESERVED_ID.to_string()),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        let oauth_channel_secret_scope = discover_secret_scope(
            filesystem.as_ref(),
            &tenant_segment,
            &tenant,
            &oauth_handles,
        )
        .await?
        .unwrap_or_else(|| admin_scope.clone());
        let proof_code_channel_secret_scope = discover_secret_scope(
            filesystem.as_ref(),
            &tenant_segment,
            &tenant,
            &proof_code_handles,
        )
        .await?
        .unwrap_or_else(|| admin_scope.clone());
        scopes.push(Rc1ChannelMigrationScope {
            admin_scope,
            oauth_channel_secret_scope,
            proof_code_channel_secret_scope,
        });
    }
    Ok(scopes)
}

pub fn is_rc1_channel_state_path(path: &str) -> bool {
    oauth_channel::is_source_state_path(path) || proof_code_channel::is_source_state_path(path)
}

async fn discover_secret_scope(
    filesystem: &dyn RootFilesystem,
    tenant_segment: &str,
    tenant: &TenantId,
    handles: &[SecretHandle],
) -> Result<Option<ResourceScope>, Rc1ChannelStateMigrationError> {
    if handles.is_empty() {
        return Ok(None);
    }
    let mut candidates: BTreeMap<String, (ResourceScope, BTreeSet<String>)> = BTreeMap::new();
    let mut scope_candidates = 0usize;
    let users_root = format!("/tenants/{tenant_segment}/users");
    for user in
        bounded_directory_names(filesystem, &users_root, MAX_RC1_CHANNEL_USERS_PER_TENANT).await?
    {
        let secrets_root = format!("{users_root}/{user}/secrets");
        probe_secret_scope(
            filesystem,
            tenant_segment,
            tenant,
            handles,
            &secrets_root,
            &mut scope_candidates,
            &mut candidates,
        )
        .await?;

        for project in bounded_directory_names(
            filesystem,
            &format!("{secrets_root}/projects"),
            MAX_RC1_CHANNEL_PROJECTS_PER_OWNER,
        )
        .await?
        {
            probe_secret_scope(
                filesystem,
                tenant_segment,
                tenant,
                handles,
                &format!("{secrets_root}/projects/{project}/secrets"),
                &mut scope_candidates,
                &mut candidates,
            )
            .await?;
        }

        for agent in bounded_directory_names(
            filesystem,
            &format!("{secrets_root}/agents"),
            MAX_RC1_CHANNEL_AGENTS_PER_USER,
        )
        .await?
        {
            let agent_root = format!("{secrets_root}/agents/{agent}");
            probe_secret_scope(
                filesystem,
                tenant_segment,
                tenant,
                handles,
                &format!("{agent_root}/secrets"),
                &mut scope_candidates,
                &mut candidates,
            )
            .await?;
            for project in bounded_directory_names(
                filesystem,
                &format!("{agent_root}/projects"),
                MAX_RC1_CHANNEL_PROJECTS_PER_OWNER,
            )
            .await?
            {
                probe_secret_scope(
                    filesystem,
                    tenant_segment,
                    tenant,
                    handles,
                    &format!("{agent_root}/projects/{project}/secrets"),
                    &mut scope_candidates,
                    &mut candidates,
                )
                .await?;
            }
        }
    }
    let required = handles
        .iter()
        .map(|handle| handle.as_str().to_string())
        .collect::<BTreeSet<_>>();
    let mut matches = candidates
        .into_values()
        .filter_map(|(scope, found)| (found == required).then_some(scope));
    let result = matches.next();
    if result.is_none() || matches.next().is_some() {
        return Err(Rc1ChannelStateMigrationError::Unavailable);
    }
    Ok(result)
}

async fn bounded_directory_names(
    filesystem: &dyn RootFilesystem,
    root: &str,
    limit: usize,
) -> Result<Vec<String>, Rc1ChannelStateMigrationError> {
    let root = VirtualPath::new(root).map_err(log_malformed)?;
    let entries = match filesystem
        .list_dir_bounded(&root, limit.saturating_add(1))
        .await
    {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
        Err(error) => return Err(log_unavailable(error)),
    };
    if entries.len() > limit {
        tracing::debug!(
            root = %root,
            limit,
            observed = entries.len(),
            "rc1 channel migration directory discovery bound exceeded"
        );
        return Err(Rc1ChannelStateMigrationError::Unavailable);
    }
    Ok(entries
        .into_iter()
        .filter(|entry| entry.file_type == FileType::Directory)
        .map(|entry| entry.name)
        .collect())
}

async fn probe_secret_scope(
    filesystem: &dyn RootFilesystem,
    tenant_segment: &str,
    tenant: &TenantId,
    handles: &[SecretHandle],
    secret_root: &str,
    scope_candidates: &mut usize,
    candidates: &mut BTreeMap<String, (ResourceScope, BTreeSet<String>)>,
) -> Result<(), Rc1ChannelStateMigrationError> {
    *scope_candidates = scope_candidates.saturating_add(1);
    if *scope_candidates > MAX_RC1_CHANNEL_SECRET_SCOPE_CANDIDATES {
        tracing::debug!(
            root = secret_root,
            limit = MAX_RC1_CHANNEL_SECRET_SCOPE_CANDIDATES,
            observed = *scope_candidates,
            "rc1 channel migration secret-scope discovery bound exceeded"
        );
        return Err(Rc1ChannelStateMigrationError::Unavailable);
    }
    for handle in handles {
        let path = VirtualPath::new(format!("{secret_root}/{}.json", handle.as_str()))
            .map_err(log_malformed)?;
        match filesystem.get(&path).await {
            Ok(Some(_)) => {}
            Ok(None) | Err(FilesystemError::NotFound { .. }) => continue,
            Err(error) => return Err(log_unavailable(error)),
        }
        let scope = secret_scope_from_path(path.as_str(), tenant_segment, tenant, handle.as_str())?
            .ok_or(Rc1ChannelStateMigrationError::Malformed)?;
        let key = format!(
            "{}\0{}\0{}",
            scope.user_id.as_str(),
            scope.agent_id.as_ref().map_or("", AgentId::as_str),
            scope.project_id.as_ref().map_or("", ProjectId::as_str),
        );
        candidates
            .entry(key)
            .or_insert_with(|| (scope, BTreeSet::new()))
            .1
            .insert(handle.as_str().to_string());
    }
    Ok(())
}

fn secret_scope_from_path(
    path: &str,
    tenant_segment: &str,
    tenant: &TenantId,
    handle: &str,
) -> Result<Option<ResourceScope>, Rc1ChannelStateMigrationError> {
    let prefix = format!("/tenants/{tenant_segment}/users/");
    let Some(rest) = path.strip_prefix(&prefix) else {
        return Ok(None);
    };
    let parts = rest.split('/').collect::<Vec<_>>();
    if parts.len() < 4 || parts[1] != "secrets" {
        return Ok(None);
    }
    let expected_leaf = format!("{handle}.json");
    if parts.last().copied() != Some(expected_leaf.as_str())
        || parts.get(parts.len().saturating_sub(2)).copied() != Some("secrets")
    {
        return Ok(None);
    }
    let user_id = if parts[0] == "__system__" {
        UserId::from_trusted(SYSTEM_RESERVED_ID.to_string())
    } else {
        UserId::new(parts[0]).map_err(log_malformed)?
    };
    let mut cursor = 2usize;
    let mut agent_id = None;
    let mut project_id = None;
    if parts.get(cursor).copied() == Some("agents") {
        let value = parts
            .get(cursor.saturating_add(1))
            .ok_or(Rc1ChannelStateMigrationError::Malformed)?;
        agent_id = Some(AgentId::new(*value).map_err(log_malformed)?);
        cursor = cursor.saturating_add(2);
    }
    if parts.get(cursor).copied() == Some("projects") {
        let value = parts
            .get(cursor.saturating_add(1))
            .ok_or(Rc1ChannelStateMigrationError::Malformed)?;
        project_id = Some(ProjectId::new(*value).map_err(log_malformed)?);
        cursor = cursor.saturating_add(2);
    }
    if parts.get(cursor).copied() != Some("secrets") || cursor + 2 != parts.len() {
        return Ok(None);
    }
    Ok(Some(ResourceScope {
        tenant_id: tenant.clone(),
        user_id,
        agent_id,
        project_id,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }))
}
