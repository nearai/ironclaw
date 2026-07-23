use ironclaw_composition::{IronClawRuntimeComponentStatus, ironclaw_runtime_readiness_snapshot};

#[test]
fn readiness_snapshot_reports_subagent_driver_wiring() {
    let snapshot = ironclaw_runtime_readiness_snapshot();

    assert_eq!(
        snapshot.text_only_driver,
        IronClawRuntimeComponentStatus::Initialized
    );
    assert_eq!(
        snapshot.planned_driver,
        IronClawRuntimeComponentStatus::Initialized
    );
    assert_eq!(
        snapshot.subagent_planned_driver,
        IronClawRuntimeComponentStatus::Initialized
    );
    assert_eq!(
        snapshot.planned_default_profile,
        IronClawRuntimeComponentStatus::Initialized
    );
}
