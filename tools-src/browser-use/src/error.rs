use serde_json::Value;

use crate::constants::*;

#[derive(Debug)]
pub struct StructuredError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
    pub hint: Option<String>,
    pub details: Option<Value>,
}

impl StructuredError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: retryable_for_code(code),
            hint: None,
            details: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }
}

#[derive(Debug)]
pub struct DispatchSuccess {
    pub data: Value,
    pub session_id: Option<String>,
    pub snapshot_id: Option<String>,
    pub attempts: u8,
    pub backend_status: u16,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub struct DispatchFailure {
    pub error: StructuredError,
    pub attempts: u8,
}

pub fn retryable_for_code(code: &str) -> bool {
    matches!(code, ERR_NETWORK_FAILURE | ERR_TIMEOUT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retryable_flags() {
        assert!(retryable_for_code(ERR_NETWORK_FAILURE));
        assert!(retryable_for_code(ERR_TIMEOUT));
        assert!(!retryable_for_code(ERR_INVALID_PARAMS));
    }
}
