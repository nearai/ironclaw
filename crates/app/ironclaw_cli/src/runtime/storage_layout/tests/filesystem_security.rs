use super::*;

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
fn ordinary_tree_operations_reject_input_deeper_than_the_adoption_bound() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = temp.path().join("source");
    fs::create_dir(&source).expect("source root");
    let mut deepest = source.clone();
    for level in 0..=ironclaw_filesystem::MAX_ORDINARY_HOST_TREE_DEPTH {
        deepest = deepest.join(format!("level-{level}"));
        fs::create_dir(&deepest).expect("nested source directory");
    }
    fs::write(deepest.join("payload.txt"), b"payload").expect("nested source file");

    let validation_error = validate_ordinary_tree(&source)
        .expect_err("validation must fail closed instead of traversing unbounded input");
    assert!(format!("{validation_error:#}").contains("depth"));

    let content_error = directory_has_content(&source)
        .expect_err("content detection must share the traversal depth bound");
    assert!(format!("{content_error:#}").contains("depth"));
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
