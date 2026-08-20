use thiserror::Error;

#[derive(Debug, Error)]
pub enum NotificationInboxError {
    #[error("notification inbox backend unavailable")]
    Backend { reason: String },
    #[error("notification inbox serialization failed")]
    Serialization { reason: String },
    #[error("notification inbox request rejected: {reason}")]
    InvalidRequest { reason: &'static str },
    #[error("notification inbox access denied")]
    AccessDenied,
    #[error("notification not found")]
    NotificationNotFound,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_and_serialization_errors_retain_causes_without_exposing_them() {
        let backend = NotificationInboxError::Backend {
            reason: "backend detail".to_string(),
        };
        assert_eq!(
            backend.to_string(),
            "notification inbox backend unavailable"
        );
        assert!(matches!(
            backend,
            NotificationInboxError::Backend { ref reason } if reason == "backend detail"
        ));

        let serialization = NotificationInboxError::Serialization {
            reason: "serialization detail".to_string(),
        };
        assert_eq!(
            serialization.to_string(),
            "notification inbox serialization failed"
        );
        assert!(matches!(
            serialization,
            NotificationInboxError::Serialization { ref reason }
                if reason == "serialization detail"
        ));
    }
}
