use ironclaw_reborn_composition::{
    RebornRuntimeComponentStatus, reborn_runtime_readiness_snapshot,
};

#[test]
fn readiness_snapshot_reports_subagent_driver_wiring() {
    let snapshot = reborn_runtime_readiness_snapshot();

    assert_eq!(
        snapshot.text_only_driver,
        RebornRuntimeComponentStatus::Initialized
    );
    assert_eq!(
        snapshot.planned_driver,
        RebornRuntimeComponentStatus::Initialized
    );
    assert_eq!(
        snapshot.subagent_planned_driver,
        RebornRuntimeComponentStatus::Initialized
    );
    assert_eq!(
        snapshot.planned_default_profile,
        RebornRuntimeComponentStatus::Initialized
    );
}
