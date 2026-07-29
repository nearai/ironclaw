//! Retired delivery-router journey target.
//!
//! These journeys covered the old product workflow facade and
//! event-router path. Current channel delivery coverage lives in
//! `reborn_integration_extension_delivery`, and trigger-selected delivery is
//! covered by the group trigger scenarios over the codec-based
//! `TriggeredRunDeliveryDriver`.

#[test]
fn delivery_user_journeys_target_is_retired_with_router_facade() {
    // Keep the historical Cargo test target valid for CI shards while the
    // obsolete router/facade-specific journey body stays retired.
}
