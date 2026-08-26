use super::*;

#[cfg(unix)]
#[test]
fn manifest_create_race_keeps_convergence_failure_in_the_error_chain() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let missing_target = temp.path().join("missing-layout-target");
    let manifest_path = temp.path().join(LAYOUT_MANIFEST_FILE);
    symlink(&missing_target, &manifest_path).expect("dangling competing manifest");
    let manifest = LayoutManifest::new(embedded_single_user_requirement());

    let error = write_manifest_last(temp.path(), &manifest)
        .expect_err("failed convergence must remain the returned source error");
    let chain = format!("{error:#}");

    assert!(
        chain.contains("initial manifest publication failed"),
        "create failure should be retained as context: {chain}"
    );
    assert!(
        chain.contains("expected an ordinary non-symlink file"),
        "convergence failure should remain in the source chain: {chain}"
    );
}

#[cfg(unix)]
#[test]
fn atomic_layout_write_stays_with_admitted_parent_after_path_replacement() {
    use ironclaw_filesystem::DiskDirectoryCapability;

    #[cfg(target_os = "macos")]
    let temp = tempfile::Builder::new()
        .tempdir_in("/private/tmp")
        .expect("tempdir");
    #[cfg(not(target_os = "macos"))]
    let temp = tempfile::tempdir().expect("tempdir");
    let parent = temp.path().join("runtime");
    let admitted_parent = temp.path().join("admitted-runtime");
    let replacement_parent = temp.path().join("replacement-runtime");
    fs::create_dir(&parent).expect("runtime parent");
    fs::create_dir(&replacement_parent).expect("replacement parent");
    let capability =
        DiskDirectoryCapability::admit_or_create(&parent).expect("admit runtime parent");

    fs::rename(&parent, &admitted_parent).expect("move admitted parent");
    std::os::unix::fs::symlink(&replacement_parent, &parent).expect("replace ambient parent");

    write_atomic_synced_at(
        &capability,
        Path::new(MIGRATION_RECORD_FILE),
        &parent.join(MIGRATION_RECORD_FILE),
        "phase = \"complete\"\n",
        false,
    )
    .expect("publish through retained parent capability");

    assert_eq!(
        fs::read_to_string(admitted_parent.join(MIGRATION_RECORD_FILE))
            .expect("record in admitted parent"),
        "phase = \"complete\"\n"
    );
    assert!(
        !replacement_parent.join(MIGRATION_RECORD_FILE).exists(),
        "ambient replacement must not receive migration records"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn migration_lock_rejects_an_existing_non_file_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("home root");
    fs::create_dir(home.join(MIGRATION_LOCK_FILE)).expect("non-file lock path");

    let error = match acquire_named_lock(&home, MIGRATION_LOCK_FILE, "storage layout migration") {
        Ok(_) => panic!("non-file lock path must fail closed"),
        Err(error) => error,
    };

    assert!(
        format!("{error:#}").contains("ordinary non-symlink file")
            || format!("{error:#}").contains("ordinary non-reparse-point file"),
        "{error:#}"
    );
}

#[test]
fn startup_admission_rejects_legacy_skills_deeper_than_the_adoption_bound() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let skills_root = temp.path().join("local-dev/skills");
    fs::create_dir_all(&skills_root).expect("legacy skills root");
    let mut deepest = skills_root;
    for level in 0..=ironclaw_filesystem::MAX_ORDINARY_HOST_TREE_DEPTH {
        deepest = deepest.join(format!("level-{level}"));
        fs::create_dir(&deepest).expect("nested source directory");
    }
    fs::write(deepest.join("payload.txt"), b"payload").expect("nested source file");

    let error = admit_startup_layout(&home, embedded_single_user_requirement())
        .expect_err("startup admission must fail closed on an over-depth legacy tree");
    assert!(format!("{error:#}").contains("depth"), "{error:#}");
    assert!(!temp.path().join(LAYOUT_MANIFEST_FILE).exists());
}

#[cfg(unix)]
#[test]
fn symlinked_legacy_database_is_rejected_without_source_mutation() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let legacy = temp.path().join("local-dev");
    fs::create_dir_all(&legacy).expect("legacy root");
    let external_root = tempfile::tempdir().expect("external root");
    let external = external_root.path().join("outside.db");
    fs::write(&external, b"outside").expect("external database");
    symlink(&external, legacy.join("reborn-local-dev.db")).expect("legacy symlink");

    let error = admit_startup_layout(&home, embedded_single_user_requirement())
        .expect_err("symlink must not be followed");

    assert!(error.to_string().contains("symlink"), "{error:#}");
    assert!(legacy.join("reborn-local-dev.db").is_symlink());
    assert!(!temp.path().join(LAYOUT_MANIFEST_FILE).exists());
    assert!(!temp.path().join("state").exists());
}

#[cfg(unix)]
#[test]
fn legacy_master_key_accepts_stricter_owner_only_mode_and_rejects_shared_mode() {
    use std::os::unix::fs::PermissionsExt as _;

    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let legacy = temp.path().join("local-dev");
    seed_legacy_embedded_store(&legacy);
    let key = legacy.join(MASTER_KEY_FILE);

    fs::set_permissions(&key, fs::Permissions::from_mode(0o400)).expect("read-only owner key");
    assert!(matches!(
        admit_startup_layout(&home, embedded_single_user_requirement())
            .expect("stricter owner-only mode remains adoptable"),
        StartupLayoutAdmission::MigrationRequired(_)
    ));

    fs::set_permissions(&key, fs::Permissions::from_mode(0o640)).expect("shared key mode");
    let error = admit_startup_layout(&home, embedded_single_user_requirement())
        .expect_err("group-readable master key must fail closed");
    assert!(error.to_string().contains("group or world"), "{error:#}");
}

// APFS rejects invalid UTF-8 names before the application can inspect them;
// Linux permits the fixture and therefore exercises the explicit rejection.
#[cfg(target_os = "linux")]
#[test]
fn legacy_skill_scope_rejects_non_utf8_directory_names() {
    use std::os::unix::ffi::OsStringExt as _;

    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let tenants = temp.path().join("local-dev/tenants");
    fs::create_dir_all(&tenants).expect("tenants root");
    fs::create_dir(tenants.join(std::ffi::OsString::from_vec(vec![0xff])))
        .expect("non-UTF-8 tenant");

    let error = admit_startup_layout(&home, embedded_single_user_requirement())
        .expect_err("lossy scope names must not be admitted");

    assert!(error.to_string().contains("not valid UTF-8"), "{error:#}");
    assert!(!temp.path().join(LAYOUT_MANIFEST_FILE).exists());
}

#[cfg(unix)]
#[test]
fn migration_rejects_a_symlinked_runtime_ancestor_before_any_write() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let home = reborn_home(temp.path());
    let requirement = embedded_single_user_requirement();
    let legacy = temp.path().join("local-dev");
    seed_legacy_embedded_store(&legacy);
    let outside_root = tempfile::tempdir().expect("outside tempdir");
    let outside = outside_root.path().join("runtime");
    fs::create_dir(&outside).expect("outside runtime");
    symlink(&outside, temp.path().join("runtime")).expect("runtime symlink");

    let error = admit_startup_layout(&home, requirement)
        .expect_err("runtime symlink must fail during pre-migration admission");

    assert!(
        format!("{error:#}").contains("ordinary non-symlink directory"),
        "{error:#}"
    );
    assert!(legacy.join("reborn-local-dev.db").is_file());
    assert!(
        fs::read_dir(&outside)
            .expect("outside remains readable")
            .next()
            .is_none(),
        "migration must not create runtime artifacts through a symlink"
    );
}
