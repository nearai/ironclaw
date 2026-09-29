use super::*;
use crate::{CapabilityDescriptorView, CapabilitySurfaceVersion};
use ironclaw_host_api::runtime::RuntimeKind;

fn surface(choice: AdvertisedToolChoice, ids: &[&str]) -> VisibleCapabilitySurface {
    VisibleCapabilitySurface {
        version: CapabilitySurfaceVersion::new("surface:test").expect("valid version"),
        descriptors: ids
            .iter()
            .map(|id| CapabilityDescriptorView {
                capability_id: CapabilityId::new(*id).expect("valid capability id"),
                provider: None,
                runtime: RuntimeKind::FirstParty,
                safe_name: id.to_string(),
                safe_description: String::new(),
                description_trust: Default::default(),
                parameters_schema: serde_json::Value::Null,
            })
            .collect(),
        callable_capability_ids: None,
        advertised_choice: choice,
    }
}

#[test]
fn an_ordinary_surface_names_every_tool() {
    let tools =
        AdvertisedTools::from_surface(&surface(AdvertisedToolChoice::Ordinary, &["builtin.time"]));
    assert_eq!(tools, AdvertisedTools::Ordinary);
    assert!(!tools.is_selected());
    assert!(
        tools.may_name(&["builtin.skill_activate"]),
        "an ordinary surface keeps today's prompt text, advertised or not"
    );
}

#[test]
fn a_selected_surface_names_only_what_it_advertises() {
    let tools = AdvertisedTools::from_surface(&surface(
        AdvertisedToolChoice::TurnStartSelection,
        &["builtin.time", "ironclaw.tool_search"],
    ));
    assert!(tools.is_selected());
    assert!(tools.may_name(&["builtin.time"]));
    assert!(tools.may_name(&["builtin.time", "ironclaw.tool_search"]));
    assert!(tools.may_name(&[]), "a block naming no tool always renders");
    assert!(
        !tools.may_name(&["builtin.time", "builtin.skill_activate"]),
        "one unadvertised tool is enough to withhold the block"
    );
}

#[test]
fn the_choice_defaults_to_ordinary_on_the_wire() {
    let surface: VisibleCapabilitySurface = serde_json::from_value(serde_json::json!({
        "version": "surface:test",
        "descriptors": [],
    }))
    .expect("a surface serialized before the field existed still parses");
    assert_eq!(surface.advertised_choice, AdvertisedToolChoice::Ordinary);
    assert_eq!(
        serde_json::to_value(AdvertisedToolChoice::TurnStartSelection).expect("serializes"),
        serde_json::json!("turn_start_selection")
    );
}
