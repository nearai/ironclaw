use serde_json::{json, Value};

use crate::constants::*;
use crate::error::{self, DispatchFailure, DispatchSuccess, StructuredError};
use crate::session::{dispatch_page_action, dispatch_session_action, is_session_action};

pub fn dispatch_with_retries(
    action: &str,
    params: &Value,
    backend_url: &str,
    _timeout_ms: u32,
) -> Result<DispatchSuccess, DispatchFailure> {
    let params_obj = params.as_object().ok_or_else(|| DispatchFailure {
        error: StructuredError::new(ERR_INVALID_PARAMS, "Parameters must be a JSON object"),
        attempts: 1,
    })?;

    if is_session_action(action) {
        return dispatch_session_action(action, params_obj, backend_url);
    }

    // All page actions go through CDP WebSocket
    let mut attempts = 0;
    let mut last_retryable: Option<StructuredError> = None;

    while attempts < MAX_ATTEMPTS {
        attempts += 1;

        match dispatch_page_action(action, params_obj, backend_url) {
            Ok(mut success) => {
                success.attempts = attempts;
                return Ok(success);
            }
            Err(failure)
                if error::retryable_for_code(failure.error.code) && attempts < MAX_ATTEMPTS =>
            {
                last_retryable = Some(failure.error);
            }
            Err(failure) if error::retryable_for_code(failure.error.code) => {
                let base = last_retryable.unwrap_or(failure.error);
                let exhausted = StructuredError::new(
                    ERR_RETRY_EXHAUSTED,
                    format!("Retries exhausted after {attempts} attempts"),
                )
                .with_retryable(false)
                .with_hint("Retry later or reduce action complexity.")
                .with_details(json!({
                    "last_error": {
                        "code": base.code,
                        "message": base.message,
                    }
                }));

                return Err(DispatchFailure {
                    error: exhausted,
                    attempts,
                });
            }
            Err(mut failure) => {
                failure.attempts = attempts;
                return Err(failure);
            }
        }
    }

    Err(DispatchFailure {
        error: StructuredError::new(ERR_RETRY_EXHAUSTED, "Retries exhausted"),
        attempts,
    })
}
