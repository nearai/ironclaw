use ironclaw_host_api::MountPermissions;
use ironclaw_reborn_composition::{
    RebornCompositionProfile, RebornLocalRuntimeProfileOptions, build_reborn_services,
    local_runtime_build_input_with_options,
};

#[tokio::test]
async fn local_dev_yolo_uses_platform_appropriate_mount_aliases() {
    let dir = tempfile::tempdir().expect("tempdir");
    let host_home = dir.path().join("home");
    std::fs::create_dir_all(&host_home).expect("host home root");

    let input = local_runtime_build_input_with_options(
        RebornCompositionProfile::LocalDevYolo,
        "windows-local-dev-yolo-integration",
        dir.path().join("local-dev"),
        RebornLocalRuntimeProfileOptions {
            confirm_host_access: true,
        },
    )
    .expect("local-dev-yolo input")
    .with_local_dev_confirmed_host_home_root(host_home.clone());
    let services = build_reborn_services(input)
        .await
        .expect("local-dev-yolo services build");

    let mounts = services
        .local_dev_workspace_mounts_for_test()
        .expect("local-dev workspace mounts");
    let aliases = mounts
        .mounts
        .iter()
        .map(|mount| mount.alias.as_str())
        .collect::<Vec<_>>();

    #[cfg(windows)]
    assert_eq!(aliases, vec!["/workspace", "/host"]);

    #[cfg(unix)]
    {
        let raw_host_home = host_home
            .canonicalize()
            .expect("canonical host home")
            .to_string_lossy()
            .into_owned();
        assert!(aliases.contains(&raw_host_home.as_str()));
    }
    assert!(
        mounts
            .mounts
            .iter()
            .all(|mount| mount.permissions == MountPermissions::read_write())
    );
}
