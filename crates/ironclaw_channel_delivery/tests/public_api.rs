use ironclaw_channel_delivery::FinalReplyDeliverySettings;

#[test]
fn default_delivery_bounds_are_non_zero() {
    let settings = FinalReplyDeliverySettings::default();

    assert!(!settings.poll_interval.is_zero());
    assert!(!settings.max_wait.is_zero());
    assert!(settings.max_concurrent_deliveries.get() > 0);
    assert!(settings.max_pending_deliveries.get() > 0);
}
