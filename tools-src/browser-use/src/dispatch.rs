use serde_json::{json, Value};

use crate::constants::*;
use crate::error::{self, DispatchFailure, DispatchSuccess, StructuredError};
use crate::session::{dispatch_page_action, dispatch_session_action, is_session_action};

fn dispatch_with_retries_impl(
    action: &str,
    params_obj: &serde_json::Map<String, Value>,
    backend_url: &str,
    mut dispatch_page: impl FnMut(
        &str,
        &serde_json::Map<String, Value>,
        &str,
    ) -> Result<DispatchSuccess, DispatchFailure>,
) -> Result<DispatchSuccess, DispatchFailure> {
    if is_session_action(action) {
        return dispatch_session_action(action, params_obj, backend_url);
    }

    let mut attempts = 0;

    while attempts < MAX_ATTEMPTS {
        attempts += 1;

        match dispatch_page(action, params_obj, backend_url) {
            Ok(mut success) => {
                success.attempts = attempts;
                return Ok(success);
            }
            Err(failure)
                if error::retryable_for_code(failure.error.code) && attempts < MAX_ATTEMPTS =>
            {
                // Retryable failure and retry budget remains.
            }
            Err(failure) if error::retryable_for_code(failure.error.code) => {
                let exhausted = StructuredError::new(
                    ERR_RETRY_EXHAUSTED,
                    format!("Retries exhausted after {attempts} attempts"),
                )
                .with_retryable(false)
                .with_hint("Retry later or reduce action complexity.")
                .with_details(json!({
                    "last_error": {
                        "code": failure.error.code,
                        "message": failure.error.message,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn success_stub() -> DispatchSuccess {
        DispatchSuccess {
            data: json!({"ok": true}),
            session_id: None,
            snapshot_id: None,
            attempts: 1,
            backend_status: 200,
            warnings: vec![],
        }
    }

    #[test]
    fn test_dispatch_with_retries_impl_retries_then_succeeds() {
        let params = serde_json::Map::new();
        let mut calls = 0u8;

        let result = dispatch_with_retries_impl("open", &params, "http://localhost", |_, _, _| {
            calls += 1;
            if calls < 3 {
                return Err(DispatchFailure {
                    error: StructuredError::new(ERR_TIMEOUT, format!("timeout-{calls}")),
                    attempts: 1,
                });
            }
            Ok(success_stub())
        })
        .expect("dispatch should eventually succeed");

        assert_eq!(calls, 3);
        assert_eq!(result.attempts, 3);
    }

    #[test]
    fn test_dispatch_with_retries_impl_returns_non_retryable_immediately() {
        let params = serde_json::Map::new();
        let mut calls = 0u8;

        let failure = dispatch_with_retries_impl("click", &params, "http://localhost", |_, _, _| {
            calls += 1;
            Err(DispatchFailure {
                error: StructuredError::new(ERR_INVALID_PARAMS, "bad input"),
                attempts: 1,
            })
        })
        .expect_err("non-retryable failures should return immediately");

        assert_eq!(calls, 1);
        assert_eq!(failure.error.code, ERR_INVALID_PARAMS);
        assert_eq!(failure.attempts, 1);
    }

    #[test]
    fn test_dispatch_with_retries_impl_wraps_retry_exhausted_with_last_error() {
        let params = serde_json::Map::new();

        let failure = dispatch_with_retries_impl("open", &params, "http://localhost", |_, _, _| {
            Err(DispatchFailure {
                error: StructuredError::new(ERR_TIMEOUT, "still timing out"),
                attempts: 1,
            })
        })
        .expect_err("retryable failures should exhaust retries");

        assert_eq!(failure.error.code, ERR_RETRY_EXHAUSTED);
        assert_eq!(failure.attempts, MAX_ATTEMPTS);
        assert_eq!(failure.error.retryable, false);
        assert_eq!(
            failure.error.details,
            Some(json!({
                "last_error": {
                    "code": ERR_TIMEOUT,
                    "message": "still timing out"
                }
            }))
        );
    }
}

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

    dispatch_with_retries_impl(action, params_obj, backend_url, dispatch_page_action)
}
