//! Content hashing helpers shared across the memory contract.

use sha2::{Digest, Sha256};

/// Compute a SHA-256 content hash using the current workspace format.
pub fn content_sha256(content: &str) -> String {
    content_bytes_sha256(content.as_bytes())
}

pub fn content_bytes_sha256(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("sha256:{:x}", hasher.finalize())
}
