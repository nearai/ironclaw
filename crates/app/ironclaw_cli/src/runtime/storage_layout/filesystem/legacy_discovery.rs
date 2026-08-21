use super::*;

const UNMANIFESTED_HOME_FILES: &[&str] = &[
    "config.toml",
    "providers.json",
    "webui-token",
    ".onboard-completed.json",
    MIGRATION_LOCK_FILE,
];

const UNMANIFESTED_HOME_DIRECTORIES: &[&str] = &[
    "local-dev",
    "hosted-single-tenant",
    "hosted-single-tenant-volume",
    "hosted-single-tenant-volume-sandboxed",
];

pub(in super::super) fn inspect_legacy_candidates(
    home: &Path,
) -> anyhow::Result<Vec<LegacyCandidate>> {
    validate_unmanifested_home_shape(home)?;
    let sandbox_root = home.join("hosted-single-tenant-volume-sandboxed");
    if unreleased_sandbox_is_populated(&sandbox_root)? {
        bail!(
            "unreleased sandbox legacy root is populated at {}; inspect or archive it explicitly before adoption. IronClaw will not auto-adopt sandbox state or workspaces",
            sandbox_root.display()
        );
    }

    let mut candidates = Vec::new();
    for kind in [
        LegacySourceKind::LocalDev,
        LegacySourceKind::HostedSingleTenant,
        LegacySourceKind::HostedSingleTenantVolume,
    ] {
        let candidate = inspect_profile_root(home, kind)?;
        if let Some(candidate) = candidate {
            candidates.push(candidate);
        }
    }
    if let Some(candidate) = inspect_bare_home(home)? {
        candidates.push(candidate);
    }
    Ok(candidates)
}

/// Reject content outside the closed grammar understood by fresh-layout and
/// legacy-layout admission before either path can publish a manifest.
///
/// Known operator files remain at the installation root. Recognized legacy
/// roots and empty canonical namespaces are validated by their typed callers
/// below; this preflight only prevents arbitrary top-level content from being
/// silently stranded beside a newly published `layout.toml`.
fn validate_unmanifested_home_shape(home: &Path) -> anyhow::Result<()> {
    let entries = match fs::read_dir(home) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("read unmanifested Reborn home {}", home.display()));
        }
    };
    let canonical_paths = RebornStoragePaths::from_installation_root(home);
    let canonical_names = canonical_paths
        .canonical_namespace_roots()
        .map(|path| path.file_name());
    for entry in entries {
        let entry =
            entry.with_context(|| format!("read entry under Reborn home {}", home.display()))?;
        let name = entry.file_name();
        let path = entry.path();
        if LIBSQL_DB_UNIT
            .iter()
            .chain(std::iter::once(&MASTER_KEY_FILE))
            .chain(UNMANIFESTED_HOME_FILES.iter())
            .any(|known| name == std::ffi::OsStr::new(known))
        {
            require_ordinary_file(&path)?;
            continue;
        }
        if UNMANIFESTED_HOME_DIRECTORIES
            .iter()
            .any(|known| name == std::ffi::OsStr::new(known))
            || canonical_names.contains(&Some(name.as_os_str()))
        {
            require_ordinary_directory(&path)?;
            continue;
        }
        bail!(
            "unknown entry `{}` in unmanifested Reborn home {}; initialization and adoption will not discard or reinterpret it",
            name.to_string_lossy(),
            home.display()
        );
    }
    Ok(())
}

pub(in super::super) fn inspect_profile_root(
    home: &Path,
    kind: LegacySourceKind,
) -> anyhow::Result<Option<LegacyCandidate>> {
    let directory = kind
        .profile_directory()
        .ok_or_else(|| anyhow!("bare home is not a profile root"))?;
    let root = home.join(directory);
    if !root.exists() {
        return Ok(None);
    }
    require_ordinary_directory(&root)?;
    let mut db_files = Vec::new();
    let mut has_master_key = false;
    let mut has_system_content = false;
    let mut has_legacy_skills = false;
    for entry in
        fs::read_dir(&root).with_context(|| format!("read legacy root {}", root.display()))?
    {
        let entry = entry.with_context(|| format!("read entry under {}", root.display()))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();
        if LIBSQL_DB_UNIT.contains(&name.as_ref()) {
            require_ordinary_file(&path)?;
            db_files.push(name.into_owned());
        } else if name == MASTER_KEY_FILE {
            require_ordinary_file(&path)?;
            validate_master_key_source(&path)?;
            has_master_key = true;
        } else if name == "system" {
            require_ordinary_directory(&path)?;
            has_system_content = system_tree_has_content(&path)?;
            validate_system_tree(&path)?;
        } else if name == "skills" {
            require_ordinary_directory(&path)?;
            validate_ordinary_tree(&path)?;
            has_legacy_skills |= directory_has_content(&path)?;
        } else if name == "tenants" {
            require_ordinary_directory(&path)?;
            has_legacy_skills |= validate_legacy_tenant_skill_tree(&path)?;
        } else if path.is_dir() && directory_is_empty(&path)? {
            // Empty directory entries are not state and cannot be inferred as
            // workspaces or ownership. Leave them in the preserved snapshot.
        } else {
            bail!(
                "unknown entry `{name}` in populated legacy root {}; adoption will not discard or reinterpret it",
                root.display()
            );
        }
    }

    db_files.sort();
    if db_files.iter().any(|file| file != DB_FILE) && !db_files.iter().any(|file| file == DB_FILE) {
        bail!(
            "legacy root {} has libSQL sidecars without {DB_FILE}",
            root.display()
        );
    }
    let populated =
        !db_files.is_empty() || has_master_key || has_system_content || has_legacy_skills;
    if !populated {
        return Ok(None);
    }
    if kind == LegacySourceKind::HostedSingleTenant {
        if !db_files.is_empty() || has_master_key {
            bail!(
                "{} is a PostgreSQL/system-content legacy source but contains embedded DB/key files; inspect it manually",
                root.display()
            );
        }
    } else if !db_files.is_empty() && !has_master_key {
        bail!(
            "legacy embedded state at {} lacks its cached secrets master key; refusing adoption that could make encrypted secrets unreadable",
            root.display()
        );
    }

    Ok(Some(LegacyCandidate {
        kind,
        source_root: root,
        db_files,
        has_master_key,
        has_system_content,
        has_legacy_skills,
    }))
}

pub(in super::super) fn inspect_bare_home(home: &Path) -> anyhow::Result<Option<LegacyCandidate>> {
    let mut db_files = Vec::new();
    for file in LIBSQL_DB_UNIT {
        let path = home.join(file);
        if path.exists() {
            require_ordinary_file(&path)?;
            db_files.push((*file).to_string());
        }
    }
    let key_path = home.join(MASTER_KEY_FILE);
    let has_master_key = key_path.exists();
    if has_master_key {
        require_ordinary_file(&key_path)?;
        validate_master_key_source(&key_path)?;
    }
    if db_files.is_empty() && !has_master_key {
        return Ok(None);
    }
    if db_files.iter().any(|file| file != DB_FILE) && !db_files.iter().any(|file| file == DB_FILE) {
        bail!(
            "bare Reborn home {} has libSQL sidecars without {DB_FILE}",
            home.display()
        );
    }
    if !db_files.is_empty() && !has_master_key {
        bail!(
            "bare Reborn home {} has embedded state without its cached secrets master key; refusing adoption",
            home.display()
        );
    }
    Ok(Some(LegacyCandidate {
        kind: LegacySourceKind::BareHome,
        source_root: home.to_path_buf(),
        db_files,
        has_master_key,
        has_system_content: false,
        has_legacy_skills: false,
    }))
}

pub(in super::super) fn unreleased_sandbox_is_populated(path: &Path) -> anyhow::Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    require_ordinary_directory(path)?;
    directory_has_content(path)
}

pub(in super::super) fn candidate_paths(candidates: &[LegacyCandidate]) -> String {
    candidates
        .iter()
        .map(|candidate| candidate.source_root.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

pub(in super::super) fn validate_system_tree(root: &Path) -> anyhow::Result<()> {
    require_ordinary_directory(root)?;
    for entry in
        fs::read_dir(root).with_context(|| format!("read system content {}", root.display()))?
    {
        let entry = entry.with_context(|| format!("read system entry under {}", root.display()))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !SYSTEM_CONTENT_DIRS.contains(&name.as_ref()) {
            bail!(
                "unknown system entry `{name}` under {}; adoption will not reinterpret it",
                root.display()
            );
        }
        validate_ordinary_tree(&entry.path())?;
    }
    Ok(())
}

/// Validate the one released host-disk user-skill grammar without accepting
/// arbitrary tenant content as a migration source.
pub(in super::super) fn validate_legacy_tenant_skill_tree(
    tenants_root: &Path,
) -> anyhow::Result<bool> {
    let mut has_content = false;
    for tenant in fs::read_dir(tenants_root)
        .with_context(|| format!("read legacy tenants tree {}", tenants_root.display()))?
    {
        let tenant = tenant
            .with_context(|| format!("read tenant entry under {}", tenants_root.display()))?;
        let tenant_path = tenant.path();
        require_ordinary_directory(&tenant_path)?;
        let tenant_name = tenant.file_name().into_string().map_err(|invalid_name| {
            anyhow!(
                "legacy skill tenant directory name under {} is not valid UTF-8 ({} bytes)",
                tenants_root.display(),
                invalid_name.len()
            )
        })?;
        TenantId::new(tenant_name.clone())
            .map_err(|error| anyhow!("invalid legacy skill tenant `{tenant_name}`: {error}"))?;
        let users_root = tenant_path.join("users");
        for entry in fs::read_dir(&tenant_path)
            .with_context(|| format!("read legacy tenant {}", tenant_path.display()))?
        {
            let entry = entry.with_context(|| {
                format!("read entry under legacy tenant {}", tenant_path.display())
            })?;
            if entry.file_name() != "users" {
                bail!(
                    "unknown entry `{}` under legacy tenant {}; only users/<user>/skills is adoptable",
                    entry.file_name().to_string_lossy(),
                    tenant_path.display()
                );
            }
            require_ordinary_directory(&entry.path())?;
        }
        if !users_root.exists() {
            continue;
        }
        for user in fs::read_dir(&users_root)
            .with_context(|| format!("read legacy users tree {}", users_root.display()))?
        {
            let user =
                user.with_context(|| format!("read user entry under {}", users_root.display()))?;
            let user_path = user.path();
            require_ordinary_directory(&user_path)?;
            let user_name = user.file_name().into_string().map_err(|invalid_name| {
                anyhow!(
                    "legacy skill user directory name under {} is not valid UTF-8 ({} bytes)",
                    users_root.display(),
                    invalid_name.len()
                )
            })?;
            UserId::new(user_name.clone())
                .map_err(|error| anyhow!("invalid legacy skill user `{user_name}`: {error}"))?;
            let skills_root = user_path.join("skills");
            for entry in fs::read_dir(&user_path)
                .with_context(|| format!("read legacy user {}", user_path.display()))?
            {
                let entry = entry.with_context(|| {
                    format!("read entry under legacy user {}", user_path.display())
                })?;
                if entry.file_name() != "skills" {
                    bail!(
                        "unknown entry `{}` under legacy user {}; only the skills tree is adoptable",
                        entry.file_name().to_string_lossy(),
                        user_path.display()
                    );
                }
                require_ordinary_directory(&entry.path())?;
            }
            if skills_root.exists() {
                validate_ordinary_tree(&skills_root)?;
                has_content |= directory_has_content(&skills_root)?;
            }
        }
    }
    Ok(has_content)
}

pub(in super::super) fn system_tree_has_content(root: &Path) -> anyhow::Result<bool> {
    for entry in
        fs::read_dir(root).with_context(|| format!("read system content {}", root.display()))?
    {
        let entry = entry.with_context(|| format!("read system entry under {}", root.display()))?;
        if directory_has_content(&entry.path())? {
            return Ok(true);
        }
    }
    Ok(false)
}
