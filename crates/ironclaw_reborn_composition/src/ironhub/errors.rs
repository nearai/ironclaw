use ironclaw_product_workflow::ProductWorkflowError;

use super::model::IronHubCommandError;

pub(super) fn invalid_input(error: impl std::fmt::Display) -> IronHubCommandError {
    IronHubCommandError::InvalidInput {
        reason: error.to_string(),
    }
}

pub(super) fn catalog_error(reason: impl Into<String>) -> IronHubCommandError {
    IronHubCommandError::Catalog {
        reason: reason.into(),
    }
}

pub(super) fn install_error(reason: impl Into<String>) -> IronHubCommandError {
    IronHubCommandError::Install {
        reason: reason.into(),
    }
}

pub(super) fn product_error(error: impl std::fmt::Display) -> IronHubCommandError {
    IronHubCommandError::Product(ProductWorkflowError::InvalidBindingRequest {
        reason: error.to_string(),
    })
}
