use super::build_host_access;
use ironclaw_config::RebornStoragePaths;

#[test]
fn host_access_initializes_a_missing_installation_root() {
    let temp = tempfile::tempdir().expect("temporary parent");
    let home = temp.path().join("reborn-home");

    build_host_access(
        RebornStoragePaths::from_installation_root(&home),
        None,
        None,
        None,
        false,
    )
    .expect("first boot must initialize a missing installation root");

    assert!(home.join("state").is_dir());
    assert!(home.join("system/prompts").is_dir());
    assert!(home.join("workspaces").is_dir());
}

#[test]
fn host_access_rejects_canonical_namespace_aliases() {
    let temp = tempfile::tempdir().expect("temporary installation root");
    let home = temp.path().join("reborn-home");
    std::fs::create_dir_all(home.join("system")).expect("create system root");
    std::os::unix::fs::symlink(home.join("system"), home.join("state"))
        .expect("alias state to system");

    let error = match build_host_access(
        RebornStoragePaths::from_installation_root(&home),
        None,
        None,
        None,
        false,
    ) {
        Ok(_) => panic!("state and system must not resolve to the same canonical directory"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("state root must not overlap system root"),
        "unexpected path-validation error: {error}"
    );
}

#[test]
fn host_access_rejects_an_external_system_symlink_before_creating_children() {
    let temp = tempfile::tempdir().expect("temporary installation root");
    let home = temp.path().join("reborn-home");
    let outside = temp.path().join("outside-system");
    std::fs::create_dir_all(&home).expect("create installation root");
    std::fs::create_dir_all(&outside).expect("create outside system root");
    std::os::unix::fs::symlink(&outside, home.join("system"))
        .expect("alias system to outside directory");

    let error = match build_host_access(
        RebornStoragePaths::from_installation_root(&home),
        None,
        None,
        None,
        false,
    ) {
        Ok(_) => panic!("external system aliases must fail before initialization"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("system root"), "{error}");
    for child in ["extensions", "prompts", "skills"] {
        assert!(
            !outside.join(child).exists(),
            "host access must not create {child} through an external system symlink"
        );
    }
}

#[test]
fn host_access_accepts_a_symlinked_installation_root() {
    let temp = tempfile::tempdir().expect("temporary parent");
    let target = temp.path().join("volume-backed-reborn-home");
    let alias = temp.path().join("reborn-home");
    std::fs::create_dir_all(&target).expect("create installation target");
    std::os::unix::fs::symlink(&target, &alias).expect("alias installation root");

    build_host_access(
        RebornStoragePaths::from_installation_root(&alias),
        None,
        None,
        None,
        false,
    )
    .expect("an operator-managed installation-root symlink must be accepted");

    assert!(target.join("state").is_dir());
    assert!(target.join("system/prompts").is_dir());
    assert!(target.join("workspaces").is_dir());
}
