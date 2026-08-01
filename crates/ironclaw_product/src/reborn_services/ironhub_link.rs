use ironclaw_product_contracts::ironhub::IronhubLinkError;
use ironclaw_product_contracts::surface::{
    ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceValidationCode,
};

pub(super) fn ironhub_link_unavailable() -> ProductSurfaceError {
    ProductSurfaceError::service_unavailable(false)
}

pub(super) fn map_ironhub_link_error(error: IronhubLinkError) -> ProductSurfaceError {
    match error {
        IronhubLinkError::InvalidSignature
        | IronhubLinkError::StaleTimestamp
        | IronhubLinkError::Replay => {
            ProductSurfaceError::from_status(ProductSurfaceErrorCode::Forbidden, 403, false)
        }
        IronhubLinkError::Install { .. } => ProductSurfaceError::internal_invariant(),
        IronhubLinkError::InvalidInput { .. } => {
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
}
