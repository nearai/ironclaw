use super::*;

fn classify(home: &RebornHome, requirement: LayoutRequirement) -> Vec<LegacyCandidate> {
    match admit_startup_layout(home, requirement).expect("classify legacy home") {
        StartupLayoutAdmission::MigrationRequired(candidates) => candidates,
        StartupLayoutAdmission::Ready(_) => panic!("legacy home cannot be ready"),
    }
}

#[test]
fn local_dev_home_migrates_into_canonical_layout_and_reopens() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let requirement = embedded_single_user_requirement();
    let legacy = temp.path().join("local-dev");
    seed_legacy_embedded_store(&legacy);

    let candidates = classify(&home, requirement);
    migrate_legacy_layout(
        &home,
        requirement,
        StorageMigrationPolicy::Automatic,
        candidates,
    )
    .expect("automatic migration");

    let paths = RebornStoragePaths::from_home(&home);
    assert!(paths.state_root().join(DB_FILE).is_file());
    assert!(paths.state_root().join(MASTER_KEY_FILE).is_file());
    assert!(!legacy.join(DB_FILE).exists());
    assert!(!legacy.join(MASTER_KEY_FILE).exists());
    let manifest = read_manifest(&temp.path().join(LAYOUT_MANIFEST_FILE)).expect("manifest");
    assert_eq!(
        manifest.memory_provider_app_id(),
        Some(ironclaw_config::legacy_memory_provider_app_id(&legacy).as_str()),
        "the migrated manifest preserves the legacy external-memory namespace"
    );
    let record = read_migration_record(&migration_record_path(&paths)).expect("provenance record");
    assert_eq!(record.phase, MigrationPhase::Complete);
    assert_eq!(record.source, LegacyStorageSource::LocalDev);
    assert!(record.ignored.is_empty());

    // The next startup admits the published layout without migration work.
    match admit_startup_layout(&home, requirement).expect("ready admission") {
        StartupLayoutAdmission::Ready(_) => {}
        StartupLayoutAdmission::MigrationRequired(_) => {
            panic!("a migrated home must admit as ready")
        }
    }

    // The moved store and key reopen through the production opener.
    crate::runtime::block_on_cli({
        let state_root = paths.state_root().to_path_buf();
        async move {
            ironclaw_composition::open_standalone_secret_store(&state_root)
                .await
                .map(|_| ())
        }
    })
    .expect("reopen migrated embedded store");
}

#[test]
fn stale_prelock_candidates_readmit_a_layout_completed_by_another_migrator() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let requirement = embedded_single_user_requirement();
    let legacy = temp.path().join("local-dev");
    seed_legacy_embedded_store(&legacy);

    let stale_candidates = classify(&home, requirement);
    migrate_legacy_layout(
        &home,
        requirement,
        StorageMigrationPolicy::Automatic,
        stale_candidates.clone(),
    )
    .expect("first migrator completes");

    migrate_legacy_layout(
        &home,
        requirement,
        StorageMigrationPolicy::Automatic,
        stale_candidates,
    )
    .expect("second migrator re-admits the ready layout under the lock");
    ensure_ready_layout(&home, requirement).expect("completed layout remains ready");
}

#[test]
fn stale_prelock_candidates_recover_a_complete_record_without_a_manifest() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let requirement = embedded_single_user_requirement();
    let legacy = temp.path().join("local-dev");
    seed_legacy_embedded_store(&legacy);

    let stale_candidates = classify(&home, requirement);
    migrate_legacy_layout(
        &home,
        requirement,
        StorageMigrationPolicy::Automatic,
        stale_candidates.clone(),
    )
    .expect("first migrator completes");
    fs::remove_file(temp.path().join(LAYOUT_MANIFEST_FILE))
        .expect("simulate complete-record crash window");

    migrate_legacy_layout(
        &home,
        requirement,
        StorageMigrationPolicy::Automatic,
        stale_candidates,
    )
    .expect("second migrator recovers the completed record under the lock");
    ensure_ready_layout(&home, requirement).expect("recovered layout remains ready");
}

#[test]
fn migrated_layout_provenance_allows_only_a_monotonic_workspace_floor_upgrade() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let weak = embedded_single_user_requirement();
    let legacy = temp.path().join("local-dev");
    seed_legacy_embedded_store(&legacy);

    let candidates = classify(&home, weak);
    migrate_legacy_layout(&home, weak, StorageMigrationPolicy::Automatic, candidates)
        .expect("migration");
    let strong = LayoutRequirement {
        security: DeploymentSecurityEnvelope {
            workspace_access_floor: WorkspaceAccessFloor::PerCallerIsolated,
            ..weak.security
        },
        ..weak
    };

    ensure_ready_layout(&home, strong).expect("migrated manifest floor strengthens");
    let error = ensure_ready_layout(&home, weak)
        .expect_err("migration provenance must not make the floor weakenable again");
    assert!(
        error
            .to_string()
            .contains("workspace access floor cannot weaken"),
        "unexpected admission error: {error:#}"
    );
}

#[test]
fn completed_migration_record_republishes_its_exact_manifest_after_crash_window() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let requirement = embedded_single_user_requirement();
    let legacy = temp.path().join("local-dev");
    seed_legacy_embedded_store(&legacy);

    let candidates = classify(&home, requirement);
    migrate_legacy_layout(
        &home,
        requirement,
        StorageMigrationPolicy::Automatic,
        candidates,
    )
    .expect("migration");
    let manifest_path = temp.path().join(LAYOUT_MANIFEST_FILE);
    let expected = read_manifest(&manifest_path).expect("committed manifest");
    fs::remove_file(&manifest_path).expect("simulate crash before manifest publication");

    assert!(matches!(
        admit_startup_layout(&home, requirement).expect("completed record resumes final publish"),
        StartupLayoutAdmission::Ready(_)
    ));
    assert_eq!(
        read_manifest(&manifest_path).expect("republished manifest"),
        expected
    );
}

#[test]
fn completed_record_recovery_strengthens_only_the_workspace_floor() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let weak = embedded_single_user_requirement();
    let legacy = temp.path().join("local-dev");
    seed_legacy_embedded_store(&legacy);

    let candidates = classify(&home, weak);
    migrate_legacy_layout(&home, weak, StorageMigrationPolicy::Automatic, candidates)
        .expect("migration");
    let manifest_path = temp.path().join(LAYOUT_MANIFEST_FILE);
    let recorded = read_manifest(&manifest_path).expect("recorded manifest");
    fs::remove_file(&manifest_path).expect("simulate crash before manifest publication");
    let strong = LayoutRequirement {
        security: DeploymentSecurityEnvelope {
            workspace_access_floor: WorkspaceAccessFloor::PerCallerIsolated,
            ..weak.security
        },
        ..weak
    };

    admit_startup_layout(&home, strong).expect("recover with the stronger floor");

    assert_eq!(
        read_manifest(&manifest_path).expect("strengthened recovered manifest"),
        recorded.with_stronger_workspace_access_floor(WorkspaceAccessFloor::PerCallerIsolated),
        "recovery must preserve every recorded provenance field except the allowed floor edge"
    );
    let error = admit_startup_layout(&home, weak)
        .expect_err("a later weak reopen must not lower the recovered floor");
    assert!(
        error
            .to_string()
            .contains("workspace access floor cannot weaken"),
        "{error:#}"
    );
}

#[test]
fn completed_migration_record_does_not_reconstruct_manifest_from_changed_requirement() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let requirement = embedded_single_user_requirement();
    let legacy = temp.path().join("local-dev");
    seed_legacy_embedded_store(&legacy);

    let candidates = classify(&home, requirement);
    migrate_legacy_layout(
        &home,
        requirement,
        StorageMigrationPolicy::Automatic,
        candidates,
    )
    .expect("migration");
    let manifest_path = temp.path().join(LAYOUT_MANIFEST_FILE);
    fs::remove_file(&manifest_path).expect("simulate crash before manifest publication");
    let changed = LayoutRequirement {
        durable_state: DurableStateKind::ExternalPostgres,
        security: requirement.security,
    };

    let error = admit_startup_layout(&home, changed)
        .expect_err("recovery must admit the recorded target against the current request");

    assert!(error.to_string().contains("storage migration"), "{error:#}");
    assert!(!manifest_path.exists());
}

#[test]
fn completed_migration_record_rejects_a_different_published_manifest() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let requirement = embedded_single_user_requirement();
    let legacy = temp.path().join("local-dev");
    seed_legacy_embedded_store(&legacy);

    let candidates = classify(&home, requirement);
    migrate_legacy_layout(
        &home,
        requirement,
        StorageMigrationPolicy::Automatic,
        candidates,
    )
    .expect("migration");

    let paths = RebornStoragePaths::from_home(&home);
    let record_path = migration_record_path(&paths);
    let mut record = read_migration_record(&record_path).expect("completed record");
    record.target_manifest = LayoutManifest::new(LayoutRequirement {
        durable_state: requirement.durable_state,
        security: DeploymentSecurityEnvelope {
            tenancy: requirement.security.tenancy,
            workspace_access_floor: WorkspaceAccessFloor::PerCallerIsolated,
        },
    });
    let contents = toml::to_string(&record).expect("serialize changed record");
    write_atomic_synced(&record_path, &contents, true).expect("replace migration record");

    let error = admit_startup_layout(&home, requirement)
        .expect_err("published manifest must match completed migration provenance exactly");

    assert!(
        error
            .to_string()
            .contains("ready layout manifest and migration record disagree"),
        "{error:#}"
    );
}

#[test]
fn manual_policy_defers_migration_without_touching_the_source() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let requirement = embedded_single_user_requirement();
    let legacy = temp.path().join("local-dev");
    seed_legacy_embedded_store(&legacy);

    let candidates = classify(&home, requirement);
    let error = migrate_legacy_layout(
        &home,
        requirement,
        StorageMigrationPolicy::Manual,
        candidates,
    )
    .expect_err("manual policy defers migration");

    assert!(
        error.to_string().contains(StorageMigrationPolicy::ENV),
        "{error:#}"
    );
    assert!(legacy.join(DB_FILE).is_file());
    assert!(!temp.path().join(LAYOUT_MANIFEST_FILE).exists());
    assert!(!temp.path().join("state").exists());
}

#[test]
fn recency_picks_the_most_recently_used_candidate_and_reports_the_loser() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let requirement = embedded_single_user_requirement();
    let stale = temp.path().join("hosted-single-tenant-volume");
    let fresh = temp.path().join("local-dev");
    seed_legacy_embedded_store(&stale);
    seed_legacy_embedded_store(&fresh);
    age_legacy_store(&stale, 3600);
    let stale_db_bytes = fs::read(stale.join(DB_FILE)).expect("stale db bytes");

    let candidates = classify(&home, requirement);
    migrate_legacy_layout(
        &home,
        requirement,
        StorageMigrationPolicy::Automatic,
        candidates,
    )
    .expect("recency selection migrates the fresh source");

    let paths = RebornStoragePaths::from_home(&home);
    assert!(paths.state_root().join(DB_FILE).is_file());
    assert!(!fresh.join(DB_FILE).exists(), "winner was moved");
    assert_eq!(
        fs::read(stale.join(DB_FILE)).expect("loser db bytes"),
        stale_db_bytes,
        "the losing candidate stays byte-for-byte untouched"
    );
    let record = read_migration_record(&migration_record_path(&paths)).expect("record");
    assert_eq!(record.source, LegacyStorageSource::LocalDev);
    assert_eq!(record.ignored.len(), 1);
    assert_eq!(
        record.ignored[0].source,
        LegacyStorageSource::HostedSingleTenantVolume
    );
}

#[test]
fn unordered_recency_tie_between_distinct_sources_fails_closed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let requirement = embedded_single_user_requirement();
    let first = temp.path().join("local-dev");
    let second = temp.path().join("hosted-single-tenant-volume");
    seed_legacy_embedded_store(&first);
    seed_legacy_embedded_store(&second);
    // Stamp both stores to the identical instant so recency carries no signal.
    let stamp = std::time::SystemTime::now();
    for root in [&first, &second] {
        for entry in fs::read_dir(root).expect("read legacy root") {
            let entry = entry.expect("legacy entry");
            if entry.file_type().expect("entry type").is_file() {
                fs::OpenOptions::new()
                    .append(true)
                    .open(entry.path())
                    .expect("open for timestamp")
                    .set_modified(stamp)
                    .expect("stamp mtime");
            }
        }
    }

    let candidates = classify(&home, requirement);
    let error = migrate_legacy_layout(
        &home,
        requirement,
        StorageMigrationPolicy::Automatic,
        candidates,
    )
    .expect_err("an unorderable tie must not guess");

    assert!(
        error.to_string().contains("nearly the same time"),
        "{error:#}"
    );
    assert!(first.join(DB_FILE).is_file());
    assert!(second.join(DB_FILE).is_file());
    assert!(!temp.path().join(LAYOUT_MANIFEST_FILE).exists());
}

#[test]
fn newer_bare_home_artifact_does_not_mask_a_tie_between_two_profile_roots() {
    // Regression: ranking used to demote a tied bare-home entry by swapping the
    // top two and never re-checking, so a bare-home artifact that was merely
    // newer than two genuinely unorderable profile directories silently
    // selected one of them instead of failing closed.
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let requirement = embedded_single_user_requirement();
    let first = temp.path().join("local-dev");
    let second = temp.path().join("hosted-single-tenant-volume");
    seed_legacy_embedded_store(&first);
    seed_legacy_embedded_store(&second);
    // Both profile roots land inside the tie window of each other...
    let tied = std::time::SystemTime::now() - std::time::Duration::from_secs(30);
    for root in [&first, &second] {
        for entry in fs::read_dir(root).expect("read legacy root") {
            let entry = entry.expect("legacy entry");
            if entry.file_type().expect("entry type").is_file() {
                fs::OpenOptions::new()
                    .append(true)
                    .open(entry.path())
                    .expect("open for timestamp")
                    .set_modified(tied)
                    .expect("stamp mtime");
            }
        }
    }
    // ...while a bare-home artifact is strictly newer than both.
    let key = ironclaw_secrets::keychain::generate_master_key_hex();
    fs::write(temp.path().join(MASTER_KEY_FILE), key).expect("bare-home key");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(
            temp.path().join(MASTER_KEY_FILE),
            fs::Permissions::from_mode(0o600),
        )
        .expect("owner-only bare key");
    }

    let candidates = classify(&home, requirement);
    assert_eq!(
        candidates.len(),
        3,
        "two profile roots plus the bare artifact"
    );
    let error = migrate_legacy_layout(
        &home,
        requirement,
        StorageMigrationPolicy::Automatic,
        candidates,
    )
    .expect_err("a newer bare-home artifact must not mask the profile-root tie");

    assert!(
        error.to_string().contains("nearly the same time"),
        "{error:#}"
    );
    assert!(first.join(DB_FILE).is_file());
    assert!(second.join(DB_FILE).is_file());
    assert!(temp.path().join(MASTER_KEY_FILE).is_file());
    assert!(!temp.path().join(LAYOUT_MANIFEST_FILE).exists());
    assert!(!temp.path().join("state").exists());
}

#[test]
fn bare_home_artifacts_lose_a_tie_against_a_real_profile_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let requirement = embedded_single_user_requirement();
    let legacy = temp.path().join("local-dev");
    seed_legacy_embedded_store(&legacy);
    // A historical resolver bug could leave a bare-home master key beside the
    // real profile directory; the profile directory must win the selection.
    let key = ironclaw_secrets::keychain::generate_master_key_hex();
    fs::write(temp.path().join(MASTER_KEY_FILE), key).expect("bare-home key");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(
            temp.path().join(MASTER_KEY_FILE),
            fs::Permissions::from_mode(0o600),
        )
        .expect("owner-only bare key");
    }

    let candidates = classify(&home, requirement);
    assert_eq!(candidates.len(), 2, "bare-home artifact is a candidate");
    migrate_legacy_layout(
        &home,
        requirement,
        StorageMigrationPolicy::Automatic,
        candidates,
    )
    .expect("profile directory wins the tie");

    let paths = RebornStoragePaths::from_home(&home);
    let record = read_migration_record(&migration_record_path(&paths)).expect("record");
    assert_eq!(record.source, LegacyStorageSource::LocalDev);
    assert_eq!(record.ignored.len(), 1);
    assert_eq!(record.ignored[0].source, LegacyStorageSource::BareHome);
    assert!(
        temp.path().join(MASTER_KEY_FILE).is_file(),
        "the ignored bare-home key stays in place"
    );
}

#[test]
fn legacy_skill_trees_stage_for_the_boot_importer() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let requirement = embedded_single_user_requirement();
    let legacy = temp.path().join("local-dev");
    seed_legacy_embedded_store(&legacy);
    fs::create_dir_all(legacy.join("skills/my-skill")).expect("legacy flat skill");
    fs::write(legacy.join("skills/my-skill/SKILL.md"), b"# skill").expect("skill file");
    fs::create_dir_all(legacy.join("tenants/tenant-a/users/user-a/skills/scoped-skill"))
        .expect("legacy scoped skill");
    fs::write(
        legacy.join("tenants/tenant-a/users/user-a/skills/scoped-skill/SKILL.md"),
        b"# scoped",
    )
    .expect("scoped skill file");

    let candidates = classify(&home, requirement);
    migrate_legacy_layout(
        &home,
        requirement,
        StorageMigrationPolicy::Automatic,
        candidates,
    )
    .expect("migration stages skills");

    let paths = RebornStoragePaths::from_home(&home);
    let staging = LegacyStorageSource::LocalDev.snapshot_root(&paths);
    assert!(staging.join("skills/my-skill/SKILL.md").is_file());
    assert!(
        staging
            .join("tenants/tenant-a/users/user-a/skills/scoped-skill/SKILL.md")
            .is_file()
    );
    assert_eq!(
        ready_legacy_skill_snapshot_source(&home).expect("staged skill source"),
        Some(LegacyStorageSource::LocalDev)
    );
}

#[test]
fn homes_without_staged_skills_report_no_snapshot_source() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    ensure_ready_layout(&home, embedded_single_user_requirement()).expect("fresh home");

    assert_eq!(
        ready_legacy_skill_snapshot_source(&home).expect("no staged skills"),
        None
    );
}

#[test]
fn migration_refuses_populated_canonical_namespaces() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let requirement = embedded_single_user_requirement();
    let legacy = temp.path().join("local-dev");
    seed_legacy_embedded_store(&legacy);
    fs::create_dir_all(temp.path().join("state")).expect("state namespace");
    fs::write(temp.path().join("state/unexplained.db"), b"data").expect("unexplained state");

    let candidates = classify(&home, requirement);
    let error = migrate_legacy_layout(
        &home,
        requirement,
        StorageMigrationPolicy::Automatic,
        candidates,
    )
    .expect_err("populated canonical namespaces must not be merged");

    assert!(error.to_string().contains("never overwrites"), "{error:#}");
    assert!(legacy.join(DB_FILE).is_file());
    assert!(!temp.path().join(LAYOUT_MANIFEST_FILE).exists());
}

#[cfg(unix)]
#[test]
fn legacy_db_lock_holder_subprocess() {
    use std::os::unix::io::AsRawFd as _;

    let Ok(db_path) = std::env::var("IRONCLAW_TEST_LEGACY_DB_LOCK_PATH") else {
        return;
    };
    let ready = std::env::var("IRONCLAW_TEST_LEGACY_DB_LOCK_READY").expect("ready path");
    let release = std::env::var("IRONCLAW_TEST_LEGACY_DB_LOCK_RELEASE").expect("release path");
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&db_path)
        .expect("open legacy db");
    // Hold a shared lock on SQLite's locking range, exactly as a live reader
    // connection would.
    // SAFETY: `flock` is plain-old-data; zero-initialization is valid before
    // the probe parameters are assigned.
    let mut lock: libc::flock = unsafe { std::mem::zeroed() };
    lock.l_type = libc::F_RDLCK as libc::c_short;
    lock.l_whence = libc::SEEK_SET as libc::c_short;
    lock.l_start = 0x4000_0002;
    lock.l_len = 510;
    let rc = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &lock) };
    assert_eq!(rc, 0, "subprocess acquires the SQLite shared range");
    fs::write(ready, b"ready").expect("signal held db lock");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !std::path::Path::new(&release).is_file() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        std::path::Path::new(&release).is_file(),
        "parent did not release db lock holder within the bounded test interval"
    );
}

#[cfg(unix)]
#[test]
fn migration_refuses_while_another_process_holds_the_legacy_database() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let requirement = embedded_single_user_requirement();
    let legacy = temp.path().join("local-dev");
    seed_legacy_embedded_store(&legacy);
    // Synchronization sentinels are test-process state, not legacy home
    // content. Keep them outside the fail-closed adoption root.
    let synchronization = tempfile::tempdir().expect("synchronization directory");
    let ready = synchronization.path().join("db-lock-ready");
    let release = synchronization.path().join("db-lock-release");
    let test_binary = std::env::current_exe().expect("test binary");
    let mut child = Command::new(test_binary)
        .args([
            "--exact",
            "runtime::storage_layout::tests::mover_migration::legacy_db_lock_holder_subprocess",
            "--nocapture",
        ])
        .env("IRONCLAW_TEST_LEGACY_DB_LOCK_PATH", legacy.join(DB_FILE))
        .env("IRONCLAW_TEST_LEGACY_DB_LOCK_READY", &ready)
        .env("IRONCLAW_TEST_LEGACY_DB_LOCK_RELEASE", &release)
        .spawn()
        .expect("spawn db lock holder");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    if !ready.is_file() {
        let _ = child.kill();
        let _ = child.wait();
        panic!("db lock holder reached its critical section");
    }

    let candidates = classify(&home, requirement);
    let migration_result = migrate_legacy_layout(
        &home,
        requirement,
        StorageMigrationPolicy::Automatic,
        candidates,
    );
    let legacy_database_remains = legacy.join(DB_FILE).is_file();
    let manifest_was_published = temp.path().join(LAYOUT_MANIFEST_FILE).exists();

    fs::write(&release, b"release").expect("release db lock holder");
    let status = child.wait().expect("wait for released db lock holder");

    let error = migration_result.expect_err("a live database holder must block migration");

    assert!(
        error.to_string().contains("another ironclaw process"),
        "{error:#}"
    );
    assert!(legacy_database_remains);
    assert!(!manifest_was_published);
    assert!(status.success(), "db lock holder exits cleanly");

    let candidates = classify(&home, requirement);
    migrate_legacy_layout(
        &home,
        requirement,
        StorageMigrationPolicy::Automatic,
        candidates,
    )
    .expect("migration proceeds after the holder exits");
}
