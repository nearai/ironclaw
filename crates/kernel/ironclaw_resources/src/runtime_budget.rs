//! The kernel side of the lane-facing budget port.
//!
//! Runtime lanes (`ironclaw_mcp`, `ironclaw_sandbox`) hold no budget
//! authority. They see only [`RuntimeResourceBudget`] — reserve, reconcile,
//! release — declared in `ironclaw_host_api::resource`. This module is the
//! only place that trait meets a real [`ResourceGovernor`]: it adapts the
//! authority to the port and projects [`ResourceError`] onto the port's
//! narrow denial vocabulary.
//!
//! The projection is deliberately lossy in one direction only. Classification
//! survives whole — `LimitExceeded` and `RequiresApproval` stay distinct, so a
//! caller above the lane cannot mistake "ask the user" for "refuse" — while
//! the account, limit, dimension, and threshold *values* stop here, in the
//! crate that owns them.

use ironclaw_host_api::{
    ids::ResourceReservationId,
    resource::{
        ResourceEstimate, ResourceReceipt, ResourceReservation, ResourceScope, ResourceUsage,
        RuntimeResourceBudget, RuntimeResourceError, RuntimeResourceErrorKind,
    },
};

use crate::{ResourceError, ResourceGovernor};

impl From<ResourceError> for RuntimeResourceError {
    fn from(error: ResourceError) -> Self {
        // The rendered reason is the authority's own message — the same string
        // lanes already forwarded as the model-visible dispatch cause before
        // this port existed. Only the structure behind it stops here.
        let reason = error.to_string();
        let kind = match error {
            ResourceError::LimitExceeded { .. } => RuntimeResourceErrorKind::LimitExceeded,
            ResourceError::RequiresApproval { .. } => RuntimeResourceErrorKind::RequiresApproval,
            ResourceError::ReservationAlreadyExists { .. } => {
                RuntimeResourceErrorKind::ReservationAlreadyExists
            }
            ResourceError::InvalidEstimate { .. } => RuntimeResourceErrorKind::InvalidEstimate,
            ResourceError::ReservationMismatch { .. } => {
                RuntimeResourceErrorKind::ReservationMismatch
            }
            ResourceError::UnknownReservation { .. } => {
                RuntimeResourceErrorKind::UnknownReservation
            }
            ResourceError::ReservationClosed { .. } => RuntimeResourceErrorKind::ReservationClosed,
            ResourceError::Storage { .. } => RuntimeResourceErrorKind::Storage,
        };
        RuntimeResourceError::new(kind, reason)
    }
}

/// Adapts a [`ResourceGovernor`] to the lane-facing [`RuntimeResourceBudget`]
/// port.
///
/// Borrowing rather than owning is deliberate: the governor is a long-lived
/// host service and a lane invocation is a borrow of it for one call, which is
/// exactly the lifetime the existing lane call sites already had.
#[derive(Debug, Clone, Copy)]
pub struct GovernorRuntimeBudget<'a, G: ?Sized> {
    governor: &'a G,
}

impl<'a, G> GovernorRuntimeBudget<'a, G>
where
    G: ResourceGovernor + ?Sized,
{
    pub fn new(governor: &'a G) -> Self {
        Self { governor }
    }
}

impl<G> RuntimeResourceBudget for GovernorRuntimeBudget<'_, G>
where
    G: ResourceGovernor + ?Sized,
{
    fn reserve(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ResourceReservation, RuntimeResourceError> {
        self.governor
            .reserve(scope, estimate)
            .map_err(RuntimeResourceError::from)
    }

    fn reconcile(
        &self,
        reservation_id: ResourceReservationId,
        actual: ResourceUsage,
    ) -> Result<ResourceReceipt, RuntimeResourceError> {
        self.governor
            .reconcile(reservation_id, actual)
            .map_err(RuntimeResourceError::from)
    }

    fn release(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReceipt, RuntimeResourceError> {
        self.governor
            .release(reservation_id)
            .map_err(RuntimeResourceError::from)
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::ids::ResourceReservationId;

    use super::*;
    use crate::{ResourceAccount, ResourceDimension, ResourceValue};

    fn account() -> ResourceAccount {
        ResourceAccount::tenant(ironclaw_host_api::ids::TenantId::new("acme").expect("tenant id"))
    }

    fn denial(dimension: ResourceDimension) -> Box<crate::ResourceDenial> {
        Box::new(crate::ResourceDenial {
            account: account(),
            dimension,
            limit: ResourceValue::Integer(1),
            current_usage: ResourceValue::Integer(0),
            active_reserved: ResourceValue::Integer(0),
            requested: ResourceValue::Integer(2),
        })
    }

    /// Every kernel denial must map onto a distinct port classification. A
    /// budget *pause* and a budget *stop* produce different user-facing
    /// outcomes — approval gate versus refusal — so collapsing them across the
    /// port would silently downgrade the lane's denial semantics.
    #[test]
    fn projection_keeps_limit_and_approval_denials_distinct() {
        let limit: RuntimeResourceError = ResourceError::LimitExceeded {
            denial: denial(ResourceDimension::Usd),
            warnings: Vec::new(),
        }
        .into();
        assert_eq!(limit.kind(), RuntimeResourceErrorKind::LimitExceeded);

        let approval: RuntimeResourceError = ResourceError::RequiresApproval {
            needed: Box::new(crate::ResourceApprovalNeeded {
                account: account(),
                dimension: ResourceDimension::Usd,
                limit: ResourceValue::Integer(1),
                current_usage: ResourceValue::Integer(0),
                active_reserved: ResourceValue::Integer(0),
                requested: ResourceValue::Integer(2),
                utilization: 0.9,
                period_end: None,
            }),
            warnings: Vec::new(),
        }
        .into();
        assert_eq!(approval.kind(), RuntimeResourceErrorKind::RequiresApproval);
        assert_ne!(limit.kind(), approval.kind());
    }

    /// The projection preserves the authority's rendered message verbatim:
    /// lanes forward it as the model-visible dispatch cause, and the port must
    /// not change what the model is told about a denial.
    #[test]
    fn projection_preserves_the_authority_message() {
        let source = ResourceError::UnknownReservation {
            id: ResourceReservationId::new(),
        };
        let expected = source.to_string();
        let projected: RuntimeResourceError = source.into();
        assert_eq!(
            projected.kind(),
            RuntimeResourceErrorKind::UnknownReservation
        );
        assert_eq!(projected.to_string(), expected);
    }

    /// Storage failures must classify as their own kind rather than as a
    /// denial: the governor doc contract requires callers to fail closed on
    /// them, and a caller that read them as `LimitExceeded` would report a
    /// budget stop that never happened.
    #[test]
    fn projection_keeps_storage_failures_out_of_the_denial_kinds() {
        let projected: RuntimeResourceError = ResourceError::Storage {
            reason: "snapshot unreadable".to_string(),
        }
        .into();
        assert_eq!(projected.kind(), RuntimeResourceErrorKind::Storage);
    }

    /// The adapter must forward through to the real authority — a reservation
    /// opened through the port is a reservation the governor holds, and
    /// releasing through the port frees it.
    #[test]
    fn adapter_forwards_reserve_and_release_to_the_governor() {
        let governor = crate::InMemoryResourceGovernor::new();
        let budget = GovernorRuntimeBudget::new(&governor);
        let scope = ResourceScope::local_default(
            ironclaw_host_api::ids::UserId::new("alice").expect("user id"),
            ironclaw_host_api::ids::InvocationId::new(),
        )
        .expect("scope");
        let account = ResourceAccount::tenant(scope.tenant_id.clone());

        let reservation = budget
            .reserve(scope, ResourceEstimate::default().set_output_bytes(16))
            .expect("reserve through the port");
        assert_eq!(governor.reserved_for(&account).output_bytes, 16);

        budget.release(reservation.id).expect("release");
        assert_eq!(
            governor.reserved_for(&account),
            crate::ResourceTally::default()
        );
        assert_eq!(
            governor.usage_for(&account),
            crate::ResourceTally::default()
        );
    }
}
