//! WASM-tool rate limiting — re-exports the shared rate limiter.
//!
//! The implementation lives in `crate::tools::rate_limiter`. WASM host
//! functions import the types from here so existing call-sites don't change.

pub use crate::tools::rate_limiter::{LimitType, RateLimitError, RateLimitResult, RateLimiter};
