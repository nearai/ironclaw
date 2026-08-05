#[test]
fn production_writer_workers_remain_behind_the_completed_migration_barrier() {
    let source = include_str!("../src/factory/production_backend_assembly.rs");
    let production = source
        .split_once("pub(super) async fn build_backend_production(")
        .expect("production builder exists")
        .1;
    let channel_migration = production
        .find("migrate_rc1_channel_state")
        .expect("channel state migration remains in production startup");
    let completion = production
        .find("release_pair_lease\n        .complete")
        .expect("release-pair completion barrier remains in production startup");
    let first_worker = production
        .find("let credential_refresh_worker")
        .expect("credential refresh worker remains in production startup");
    assert!(channel_migration < completion);
    assert!(completion < first_worker);
    assert!(
        !production[..completion].contains("tokio::spawn"),
        "no background writer may spawn before migration readback completes"
    );
}
