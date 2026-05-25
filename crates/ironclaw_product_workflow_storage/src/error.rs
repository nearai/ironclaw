//! Storage error mapping for the product workflow storage crate.
//!
//! Internal backend errors are converted to `ProductWorkflowError::Transient`
//! so the trait surface stays clean and webhook retries can replay the action.
//! Backend detail (host paths, driver-specific reasons) does not cross the
//! workflow boundary — see the per-impl `map_*_error` helpers.

use ironclaw_product_workflow::ProductWorkflowError;

pub(crate) fn transient(reason: impl Into<String>) -> ProductWorkflowError {
    ProductWorkflowError::Transient {
        reason: reason.into(),
    }
}
