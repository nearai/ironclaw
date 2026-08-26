use super::*;

#[test]
fn fresh_home_initializes_canonical_namespaces_and_commits_manifest_last() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());

    ensure_ready_layout(&home, embedded_single_user_requirement()).expect("fresh home initializes");

    assert!(temp.path().join("layout.toml").is_file());
    assert!(temp.path().join("state").is_dir());
    assert!(temp.path().join("system").is_dir());
    assert!(temp.path().join("workspaces").is_dir());
    assert!(temp.path().join("runtime").is_dir());
    assert!(temp.path().join("logs").is_dir());
    assert!(temp.path().join("cache").is_dir());
    assert!(temp.path().join("tmp").is_dir());
    let manifest = read_manifest(&temp.path().join(LAYOUT_MANIFEST_FILE)).expect("manifest");
    assert_eq!(
        manifest.memory_provider_app_id(),
        Some(ironclaw_config::canonical_memory_provider_app_id(temp.path()).as_str())
    );
}

#[test]
fn fresh_home_with_unknown_file_fails_before_initialization() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let unknown = temp.path().join("operator-notes.txt");
    fs::write(&unknown, b"preserve me").expect("unknown file");

    let error = admit_startup_layout(&home, embedded_single_user_requirement())
        .expect_err("unknown home content must fail closed");

    assert!(
        error.to_string().contains("operator-notes.txt"),
        "{error:#}"
    );
    assert_eq!(
        fs::read(&unknown).expect("unknown file retained"),
        b"preserve me"
    );
    assert!(!temp.path().join(LAYOUT_MANIFEST_FILE).exists());
    assert!(!temp.path().join("state").exists());
}

#[test]
fn fresh_home_with_unknown_empty_directory_fails_before_initialization() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let unknown = temp.path().join("archive");
    fs::create_dir(&unknown).expect("unknown directory");

    let error = admit_startup_layout(&home, embedded_single_user_requirement())
        .expect_err("unknown empty directory must not be reinterpreted");

    assert!(error.to_string().contains("archive"), "{error:#}");
    assert!(unknown.is_dir());
    assert!(!temp.path().join(LAYOUT_MANIFEST_FILE).exists());
}

#[test]
fn bare_home_candidate_with_unknown_content_fails_without_mutation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    seed_legacy_embedded_store(temp.path());
    let unknown = temp.path().join("unclassified-state.bin");
    fs::write(&unknown, b"unknown").expect("unknown state");

    let error = admit_startup_layout(&home, embedded_single_user_requirement())
        .expect_err("bare-home migration must reject extra content");

    assert!(
        error.to_string().contains("unclassified-state.bin"),
        "{error:#}"
    );
    assert!(temp.path().join(DB_FILE).is_file());
    assert!(temp.path().join(MASTER_KEY_FILE).is_file());
    assert!(unknown.is_file());
    assert!(!temp.path().join(LAYOUT_MANIFEST_FILE).exists());
    assert!(!temp.path().join("state").exists());
}

#[test]
fn known_operator_files_do_not_block_fresh_initialization() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    for name in [
        "config.toml",
        "providers.json",
        "webui-token",
        ".onboard-completed.json",
    ] {
        fs::write(temp.path().join(name), b"operator content").expect("known operator file");
    }

    admit_startup_layout(&home, embedded_single_user_requirement())
        .expect("known operator files remain compatible");

    assert!(temp.path().join(LAYOUT_MANIFEST_FILE).is_file());
}

#[cfg(unix)]
#[test]
fn symlinked_known_operator_file_fails_before_initialization() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let outside = tempfile::NamedTempFile::new().expect("outside file");
    symlink(outside.path(), temp.path().join("config.toml")).expect("config symlink");

    let error = admit_startup_layout(&home, embedded_single_user_requirement())
        .expect_err("known operator files must still be ordinary files");

    assert!(error.to_string().contains("config.toml"), "{error:#}");
    assert!(!temp.path().join(LAYOUT_MANIFEST_FILE).exists());
}

#[test]
fn concurrent_fresh_initializers_admit_the_identical_manifest() {
    use std::sync::{Arc, Barrier};

    let temp = tempfile::tempdir().expect("tempdir");
    let home = Arc::new(temp.path().to_path_buf());
    let barrier = Arc::new(Barrier::new(16));
    let manifest = LayoutManifest::new(embedded_single_user_requirement());
    let mut workers = Vec::new();
    for _ in 0..16 {
        let home = Arc::clone(&home);
        let barrier = Arc::clone(&barrier);
        let manifest = manifest.clone();
        workers.push(thread::spawn(move || {
            barrier.wait();
            write_manifest_last(&home, &manifest)
        }));
    }
    for worker in workers {
        worker
            .join()
            .expect("initializer thread")
            .expect("identical concurrent manifest is admitted");
    }
    assert_eq!(
        super::read_manifest(&home.join(LAYOUT_MANIFEST_FILE)).expect("manifest"),
        manifest
    );
}

#[test]
fn concurrent_workspace_floor_admissions_never_overwrite_a_stronger_floor() {
    use std::sync::{Arc, Barrier};

    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let weak = embedded_single_user_requirement();
    let strong = LayoutRequirement {
        security: DeploymentSecurityEnvelope {
            workspace_access_floor: WorkspaceAccessFloor::PerCallerIsolated,
            ..weak.security
        },
        ..weak
    };
    ensure_ready_layout(&home, weak).expect("initialize weak workspace floor");
    let root = Arc::new(temp.path().to_path_buf());
    let barrier = Arc::new(Barrier::new(16));
    let mut workers = Vec::new();
    for worker in 0..16 {
        let root = Arc::clone(&root);
        let barrier = Arc::clone(&barrier);
        workers.push(thread::spawn(move || {
            let home = reborn_home(root.as_ref());
            barrier.wait();
            ensure_ready_layout(&home, if worker % 2 == 0 { strong } else { weak })
        }));
    }
    for (worker, result) in workers.into_iter().enumerate() {
        let result = result.join().expect("admission thread");
        if worker % 2 == 0 {
            result.expect("every stronger admission succeeds");
        } else if let Err(error) = result {
            assert!(
                error
                    .to_string()
                    .contains("workspace access floor cannot weaken"),
                "a racing weak admission may only lose to the stronger floor: {error:#}"
            );
        }
    }

    let manifest = read_manifest(&root.join(LAYOUT_MANIFEST_FILE)).expect("upgraded manifest");
    assert_eq!(
        manifest.requirement().security.workspace_access_floor,
        WorkspaceAccessFloor::PerCallerIsolated,
        "the final durable floor must be the strongest admitted requirement"
    );
    ensure_ready_layout(&home, weak).expect_err("the stronger floor remains monotonic");
}

#[test]
fn fresh_home_initialization_resumes_after_partial_empty_namespace_creation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    fs::create_dir(temp.path().join("state")).expect("interrupted state namespace");
    fs::create_dir(temp.path().join("system")).expect("interrupted system namespace");

    let paths = ensure_ready_layout(&home, embedded_single_user_requirement())
        .expect("fresh initialization resumes idempotently");

    for path in [
        paths.state_root(),
        paths.system_root(),
        paths.workspace_root(),
        paths.runtime_root(),
        paths.logs_root(),
        paths.cache_root(),
        paths.temp_root(),
    ] {
        assert!(
            path.is_dir(),
            "canonical namespace exists: {}",
            path.display()
        );
    }
    assert!(temp.path().join("layout.toml").is_file());
}

#[test]
fn dry_run_layout_admission_refuses_a_fresh_home_without_creating_namespaces() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());

    let error = inspect_ready_layout(&home, embedded_single_user_requirement())
        .expect_err("dry-run admission must not initialize a fresh layout");

    assert!(error.to_string().contains("not ready"));
    assert!(!temp.path().join("layout.toml").exists());
    assert!(!temp.path().join("state").exists());
    assert!(!temp.path().join("system").exists());
    assert!(!temp.path().join("workspaces").exists());
    assert!(!temp.path().join("runtime").exists());
    assert!(!temp.path().join("logs").exists());
    assert!(!temp.path().join("cache").exists());
    assert!(!temp.path().join("tmp").exists());
}

#[test]
fn dry_run_stronger_requirement_leaves_a_ready_manifest_byte_unchanged() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let weak = embedded_single_user_requirement();
    ensure_ready_layout(&home, weak).expect("initialize weak layout");
    let manifest_path = temp.path().join(LAYOUT_MANIFEST_FILE);
    let before = fs::read(&manifest_path).expect("manifest bytes before inspection");
    let strong = LayoutRequirement {
        security: DeploymentSecurityEnvelope {
            workspace_access_floor: WorkspaceAccessFloor::PerCallerIsolated,
            ..weak.security
        },
        ..weak
    };

    inspect_ready_layout(&home, strong).expect("stronger requirement is safe to inspect");

    assert_eq!(
        fs::read(&manifest_path).expect("manifest bytes after inspection"),
        before,
        "dry-run inspection must never persist an otherwise valid upgrade"
    );
}

#[test]
fn ready_manifest_requires_every_canonical_namespace_to_remain_an_ordinary_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let requirement = embedded_single_user_requirement();
    admit_startup_layout(&home, requirement).expect("initialize fresh layout");
    fs::remove_dir_all(temp.path().join("state")).expect("remove state namespace");

    let error = admit_startup_layout(&home, requirement)
        .expect_err("a ready manifest without state must fail closed");

    assert!(error.to_string().contains("state"), "{error:#}");
}

#[test]
fn invalid_namespace_blocks_workspace_floor_upgrade_without_mutating_manifest() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let weak = embedded_single_user_requirement();
    ensure_ready_layout(&home, weak).expect("initialize weak layout");
    let manifest_path = temp.path().join(LAYOUT_MANIFEST_FILE);
    let before = fs::read(&manifest_path).expect("weak manifest bytes");
    fs::remove_dir_all(temp.path().join("state")).expect("remove state namespace");
    let strong = LayoutRequirement {
        security: DeploymentSecurityEnvelope {
            workspace_access_floor: WorkspaceAccessFloor::PerCallerIsolated,
            ..weak.security
        },
        ..weak
    };

    let error = ensure_ready_layout(&home, strong)
        .expect_err("unsafe namespaces must fail before a manifest upgrade");

    assert!(error.to_string().contains("state"), "{error:#}");
    assert_eq!(
        fs::read(&manifest_path).expect("manifest after rejected admission"),
        before
    );
}

#[test]
fn startup_classifies_one_legacy_root_for_migration_without_mutating_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let legacy = temp.path().join("local-dev");
    seed_legacy_embedded_store(&legacy);

    let admission = admit_startup_layout(&home, embedded_single_user_requirement())
        .expect("one supported source has a typed migration decision");

    match admission {
        StartupLayoutAdmission::MigrationRequired(candidates) => {
            assert_eq!(candidates.len(), 1);
            assert_eq!(candidates[0].kind, LegacyStorageSource::LocalDev);
        }
        StartupLayoutAdmission::Ready(_) => panic!("legacy home cannot be admitted as ready"),
    }
    assert!(legacy.join("reborn-local-dev.db").exists());
    assert!(!temp.path().join("layout.toml").exists());
    assert!(!temp.path().join("state").exists());
}

#[test]
fn cli_command_admission_reports_the_startup_remedy_for_a_legacy_home() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let legacy = temp.path().join("local-dev");
    seed_legacy_embedded_store(&legacy);

    let error = ensure_ready_layout(&home, embedded_single_user_requirement())
        .expect_err("stateful CLI commands never migrate");

    assert!(error.to_string().contains("ironclaw serve"), "{error:#}");
    assert!(legacy.join("reborn-local-dev.db").exists());
    assert!(!temp.path().join("layout.toml").exists());
}

#[test]
fn startup_and_dry_run_reject_the_same_manifest_record_disagreement() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let requirement = embedded_single_user_requirement();
    let legacy = temp.path().join("local-dev");
    seed_legacy_embedded_store(&legacy);
    let candidates = match admit_startup_layout(&home, requirement).expect("classify") {
        StartupLayoutAdmission::MigrationRequired(candidates) => candidates,
        StartupLayoutAdmission::Ready(_) => panic!("legacy home cannot be ready"),
    };
    migrate_legacy_layout(
        &home,
        requirement,
        StorageMigrationPolicy::Automatic,
        candidates,
    )
    .expect("migrate legacy layout");

    let paths = RebornStoragePaths::from_home(&home);
    let record_path = migration_record_path(&paths);
    let mut record = read_migration_record(&record_path).expect("completed record");
    record.phase = MigrationPhase::InProgress;
    let contents = toml::to_string(&record).expect("serialize record");
    write_atomic_synced(&record_path, &contents, true).expect("persist disagreement");

    let startup_error = admit_startup_layout(&home, requirement)
        .expect_err("startup must reject a manifest with an incomplete migration record");
    let dry_run_error = inspect_ready_layout(&home, requirement)
        .expect_err("dry-run must reject the same manifest and record disagreement");

    assert_eq!(startup_error.to_string(), dry_run_error.to_string());
    assert!(
        startup_error
            .to_string()
            .contains("ready layout manifest and migration record disagree"),
        "{startup_error:#}"
    );
}

#[test]
fn interrupted_migration_record_without_manifest_requires_backup_restore() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let requirement = embedded_single_user_requirement();
    let legacy = temp.path().join("local-dev");
    seed_legacy_embedded_store(&legacy);
    let paths = RebornStoragePaths::from_home(&home);
    fs::create_dir_all(paths.runtime_root()).expect("runtime namespace");
    let record = MigrationRecord {
        schema_version: MIGRATION_RECORD_SCHEMA_VERSION,
        phase: MigrationPhase::InProgress,
        source: LegacyStorageSource::LocalDev,
        source_root: legacy.clone(),
        target_manifest: LayoutManifest::new(requirement)
            .with_memory_provider_app_id(ironclaw_config::legacy_memory_provider_app_id(&legacy)),
        has_legacy_skills: false,
        ignored: Vec::new(),
    };
    let contents = toml::to_string(&record).expect("serialize record");
    write_atomic_synced(&migration_record_path(&paths), &contents, false).expect("record");

    let error = admit_startup_layout(&home, requirement)
        .expect_err("an interrupted migration must not be resumed by guessing");

    assert!(error.to_string().contains("backup"), "{error:#}");
    assert!(legacy.join("reborn-local-dev.db").exists());
    assert!(!temp.path().join("layout.toml").exists());
}

#[test]
fn multiple_legacy_candidates_are_classified_without_selection_or_mutation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let local_dev = temp.path().join("local-dev");
    let hosted = temp.path().join("hosted-single-tenant-volume");
    seed_legacy_embedded_store(&local_dev);
    seed_legacy_embedded_store(&hosted);

    let admission = admit_startup_layout(&home, embedded_single_user_requirement())
        .expect("classification is typed, not an error");

    match admission {
        StartupLayoutAdmission::MigrationRequired(candidates) => {
            assert_eq!(candidates.len(), 2);
        }
        StartupLayoutAdmission::Ready(_) => panic!("legacy home cannot be ready"),
    }
    assert!(local_dev.join("reborn-local-dev.db").exists());
    assert!(hosted.join("reborn-local-dev.db").exists());
    assert!(!temp.path().join("layout.toml").exists());
}

#[test]
fn populated_unreleased_sandbox_root_fails_admission_closed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let sandbox = temp.path().join("hosted-single-tenant-volume-sandboxed");
    fs::create_dir_all(&sandbox).expect("sandbox root");
    fs::write(sandbox.join("unmigrated.txt"), b"data").expect("sandbox content");

    let error = admit_startup_layout(&home, embedded_single_user_requirement())
        .expect_err("populated sandbox root must be archived explicitly");

    assert!(error.to_string().contains("sandbox"), "{error:#}");
    assert!(!temp.path().join("layout.toml").exists());
}

#[test]
fn ready_memory_provider_app_id_round_trips_through_the_manifest() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    ensure_ready_layout(&home, embedded_single_user_requirement()).expect("fresh home");

    let app_id = ready_memory_provider_app_id(&home).expect("manifest app id");

    assert_eq!(
        app_id,
        Some(ironclaw_config::canonical_memory_provider_app_id(
            temp.path()
        ))
    );
}
