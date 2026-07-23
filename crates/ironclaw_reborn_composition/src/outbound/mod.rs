pub(crate) mod outbound_preferences_capability;

pub(crate) use ironclaw_outbound::{
    DeliveryTargetCapabilities, MutableOutboundDeliveryTargetRegistry, OutboundDeliveryTargetEntry,
    OutboundDeliveryTargetId, OutboundDeliveryTargetOwner, OutboundDeliveryTargetProvider,
    OutboundDeliveryTargetRegistry, OutboundDeliveryTargetScope, OutboundDeliveryTargetSummary,
};
pub(crate) use ironclaw_product_workflow::{
    OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID, OUTBOUND_DELIVERY_TARGET_SET_DESCRIPTION,
    OUTBOUND_DELIVERY_TARGET_SET_PROVIDER_TOOL_NAME, OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID,
    OUTBOUND_DELIVERY_TARGETS_LIST_DESCRIPTION, OUTBOUND_DELIVERY_TARGETS_LIST_PROVIDER_TOOL_NAME,
    OutboundDeliveryCapabilityInputError, RebornOutboundPreferencesFacade,
    list_outbound_delivery_targets_for_model, outbound_delivery_synthetic_provider,
    outbound_delivery_target_set_input_schema, outbound_delivery_target_set_operator_tool_info,
    outbound_delivery_targets_list_input_schema, parse_outbound_delivery_target_set_input,
    parse_outbound_delivery_targets_list_input, set_outbound_delivery_target_for_model,
};
