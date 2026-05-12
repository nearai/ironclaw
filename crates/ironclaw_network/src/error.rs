use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkHttpErrorKind {
    InvalidUrl,
    PolicyDenied,
    DnsFailed,
    TransportFailed,
    ResponseBodyLimitExceeded,
}

impl NetworkHttpErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidUrl => "invalid_url",
            Self::PolicyDenied => "policy_denied",
            Self::DnsFailed => "dns_failed",
            Self::TransportFailed => "transport_failed",
            Self::ResponseBodyLimitExceeded => "response_body_limit_exceeded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NetworkHttpError {
    #[error("invalid network URL: {reason}")]
    InvalidUrl {
        reason: String,
        request_bytes: u64,
        response_bytes: u64,
    },
    #[error("network policy denied request: {reason}")]
    PolicyDenied {
        reason: String,
        request_bytes: u64,
        response_bytes: u64,
    },
    #[error("network DNS resolution failed: {reason}")]
    Dns {
        reason: String,
        request_bytes: u64,
        response_bytes: u64,
    },
    #[error("network transport failed: {reason}")]
    Transport {
        reason: String,
        request_bytes: u64,
        response_bytes: u64,
    },
    #[error("network response body exceeded limit {limit}")]
    ResponseBodyLimit {
        limit: u64,
        request_bytes: u64,
        response_bytes: u64,
    },
}

impl NetworkHttpError {
    pub fn kind(&self) -> NetworkHttpErrorKind {
        match self {
            Self::InvalidUrl { .. } => NetworkHttpErrorKind::InvalidUrl,
            Self::PolicyDenied { .. } => NetworkHttpErrorKind::PolicyDenied,
            Self::Dns { .. } => NetworkHttpErrorKind::DnsFailed,
            Self::Transport { .. } => NetworkHttpErrorKind::TransportFailed,
            Self::ResponseBodyLimit { .. } => NetworkHttpErrorKind::ResponseBodyLimitExceeded,
        }
    }

    pub fn stable_reason(&self) -> &'static str {
        self.kind().as_str()
    }

    pub fn request_bytes(&self) -> u64 {
        match self {
            Self::InvalidUrl { request_bytes, .. }
            | Self::PolicyDenied { request_bytes, .. }
            | Self::Dns { request_bytes, .. }
            | Self::Transport { request_bytes, .. }
            | Self::ResponseBodyLimit { request_bytes, .. } => *request_bytes,
        }
    }

    pub fn response_bytes(&self) -> u64 {
        match self {
            Self::InvalidUrl { response_bytes, .. }
            | Self::PolicyDenied { response_bytes, .. }
            | Self::Dns { response_bytes, .. }
            | Self::Transport { response_bytes, .. }
            | Self::ResponseBodyLimit { response_bytes, .. } => *response_bytes,
        }
    }
}
