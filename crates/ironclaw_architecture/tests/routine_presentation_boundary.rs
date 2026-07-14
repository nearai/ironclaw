use std::{fs, path::PathBuf};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn routine_presentation_policy_stays_with_the_first_party_capability_owner() {
    let root = repository_root();
    let composition = fs::read_to_string(
        root.join("crates/ironclaw_reborn_composition/src/projection/display_preview.rs"),
    )
    .expect("composition display preview source");
    let owner = fs::read_to_string(
        root.join("crates/ironclaw_host_runtime/src/first_party_tools/trigger_presentation.rs"),
    )
    .expect("first-party trigger presentation source");

    for forbidden in [
        "enum RoutineCapability",
        "fn routine_capability",
        "fn routine_list_preview_lines",
        "fn routine_schedule_label",
    ] {
        assert!(
            !composition.contains(forbidden),
            "composition must not own routine presentation policy: {forbidden}"
        );
    }
    for required in [
        "enum RoutineCapability",
        "fn routine_capability",
        "pub fn routine_output_presentation",
    ] {
        assert!(
            owner.contains(required),
            "first-party capability owner must retain routine policy: {required}"
        );
    }
}
