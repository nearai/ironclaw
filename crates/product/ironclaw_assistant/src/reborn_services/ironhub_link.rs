use ironclaw_product_contracts::ironhub::IronhubLinkError;
use ironclaw_product_contracts::surface::{
    ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind,
    ProductSurfaceValidationCode,
};

pub(super) fn ironhub_link_unavailable() -> ProductSurfaceError {
    ProductSurfaceError::service_unavailable(false)
}

pub(super) fn map_ironhub_link_error(error: IronhubLinkError) -> ProductSurfaceError {
    match error {
        IronhubLinkError::InvalidSignature => ProductSurfaceError::from_status_kind(
            ProductSurfaceErrorCode::Forbidden,
            ProductSurfaceErrorKind::ParticipantDenied,
            403,
            false,
        ),
        IronhubLinkError::StaleTimestamp => ProductSurfaceError::from_status_kind(
            ProductSurfaceErrorCode::Forbidden,
            ProductSurfaceErrorKind::Expired,
            403,
            false,
        ),
        IronhubLinkError::Replay => ProductSurfaceError::from_status_kind(
            ProductSurfaceErrorCode::Forbidden,
            ProductSurfaceErrorKind::Duplicate,
            403,
            false,
        ),
        IronhubLinkError::Install { reason } => {
            tracing::error!(%reason, "ironhub link install failed");
            ProductSurfaceError::internal_invariant()
        }
        IronhubLinkError::InvalidInput { reason } => {
            tracing::debug!(%reason, "ironhub link request rejected");
            ProductSurfaceError::validation("input", ProductSurfaceValidationCode::InvalidValue)
        }
        IronhubLinkError::Unavailable => ProductSurfaceError::service_unavailable(false),
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_product_contracts::surface::{ProductSurfaceErrorCode, ProductSurfaceErrorKind};

    use super::*;

    #[test]
    fn link_errors_map_to_redacted_product_surface_categories() {
        for source in [
            IronhubLinkError::InvalidSignature,
            IronhubLinkError::StaleTimestamp,
            IronhubLinkError::Replay,
        ] {
            let error = map_ironhub_link_error(source);
            assert_eq!(error.code, ProductSurfaceErrorCode::Forbidden);
            assert_eq!(error.status_code, 403);
            assert!(!error.retryable);
        }

        let invalid = map_ironhub_link_error(IronhubLinkError::InvalidInput {
            reason: "sensitive request detail".to_string(),
        });
        assert_eq!(invalid.code, ProductSurfaceErrorCode::InvalidRequest);
        assert_eq!(invalid.kind, ProductSurfaceErrorKind::Validation);
        assert_eq!(invalid.field.as_deref(), Some("input"));

        let install = map_ironhub_link_error(IronhubLinkError::Install {
            reason: "sensitive backend detail".to_string(),
        });
        assert_eq!(install.code, ProductSurfaceErrorCode::Internal);
        assert_eq!(install.status_code, 500);

        let unavailable = map_ironhub_link_error(IronhubLinkError::Unavailable);
        assert_eq!(unavailable.code, ProductSurfaceErrorCode::Unavailable);
        assert_eq!(unavailable.status_code, 503);
        assert!(!unavailable.retryable);
    }

    #[test]
    fn each_rejected_link_reason_stays_distinguishable_on_the_wire() {
        let signature = map_ironhub_link_error(IronhubLinkError::InvalidSignature).kind;
        let stale = map_ironhub_link_error(IronhubLinkError::StaleTimestamp).kind;
        let replay = map_ironhub_link_error(IronhubLinkError::Replay).kind;

        assert_eq!(signature, ProductSurfaceErrorKind::ParticipantDenied);
        assert_eq!(stale, ProductSurfaceErrorKind::Expired);
        assert_eq!(replay, ProductSurfaceErrorKind::Duplicate);

        assert_ne!(signature, stale);
        assert_ne!(stale, replay);
        assert_ne!(signature, replay);
    }
}
