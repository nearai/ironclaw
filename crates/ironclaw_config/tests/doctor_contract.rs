use ironclaw_config::{IronClawBootConfig, IronClawDoctorReport, IronClawProfile};

#[test]
fn doctor_report_is_side_effect_free_and_states_v1_is_not_used() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config = IronClawBootConfig::resolve_from_env_parts(
        Some(temp.path().join("ironclaw-home").into_os_string()),
        None,
        None,
        Some("migration-dry-run".into()),
    )
    .expect("boot config should resolve");

    let report = IronClawDoctorReport::from_config(config);

    assert_eq!(report.profile(), IronClawProfile::MigrationDryRun);
    assert_eq!(report.home_source_label(), "IRONCLAW_HOME");
    assert_eq!(report.v1_state(), "not-used");
    assert!(!report.home_path().exists());
}
