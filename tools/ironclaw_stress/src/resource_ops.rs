use ironclaw_host_api::{ResourceEstimate, ResourceUsage};
use ironclaw_resources::ResourceError;
use rust_decimal_macros::dec;

use crate::{Scenario, summary::FailureCause};

pub(crate) fn estimate() -> ResourceEstimate {
    ResourceEstimate {
        usd: Some(dec!(0.000001)),
        input_tokens: Some(8),
        output_tokens: Some(4),
        wall_clock_ms: Some(1),
        output_bytes: Some(16),
        network_egress_bytes: Some(0),
        process_count: Some(0),
        concurrency_slots: Some(1),
    }
}

pub(crate) fn usage() -> ResourceUsage {
    ResourceUsage {
        usd: dec!(0.000001),
        input_tokens: 8,
        output_tokens: 4,
        wall_clock_ms: 1,
        output_bytes: 16,
        network_egress_bytes: 0,
        process_count: 0,
    }
}

pub(crate) fn failure(scenario: Scenario, error: ResourceError) -> FailureCause {
    failure_for_stage(scenario_stage(scenario), error)
}

pub(crate) fn failure_for_stage(stage: &'static str, error: ResourceError) -> FailureCause {
    FailureCause::new(classify_error(&error), stage, format!("{error:?}"))
}

fn classify_error(error: &ResourceError) -> String {
    match error {
        ResourceError::Storage { reason } if reason.contains("cross-process CAS contention") => {
            "storage_cross_process_cas_contention".to_string()
        }
        ResourceError::Storage { .. } => "storage".to_string(),
        ResourceError::LimitExceeded { .. } => "limit_exceeded".to_string(),
        ResourceError::RequiresApproval { .. } => "requires_approval".to_string(),
        ResourceError::ReservationAlreadyExists { .. } => "reservation_already_exists".to_string(),
        ResourceError::InvalidEstimate { .. } => "invalid_estimate".to_string(),
        ResourceError::ReservationMismatch { .. } => "reservation_mismatch".to_string(),
        ResourceError::UnknownReservation { .. } => "unknown_reservation".to_string(),
        ResourceError::ReservationClosed { .. } => "reservation_closed".to_string(),
    }
}

fn scenario_stage(scenario: Scenario) -> &'static str {
    match scenario {
        Scenario::ReserveRelease => "reserve_release",
        Scenario::ReserveReconcile => "reserve_reconcile",
        Scenario::ChatTurn => "chat_turn",
        Scenario::TurnLifecycleChurn => "turn_lifecycle_churn",
        Scenario::ThreadList => "thread_list",
        Scenario::MixedUserSession => "mixed_user_session",
        Scenario::ContextGrowth => "context_growth",
        Scenario::ToolSession => "tool_session",
        Scenario::ApiUserCapacity => "api_user_capacity",
        Scenario::SecretConsume => "secret_consume",
        Scenario::CpuBurn => "cpu_burn",
        Scenario::MemoryChurn => "memory_churn",
    }
}
