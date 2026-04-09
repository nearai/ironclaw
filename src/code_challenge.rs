//! Shared helpers for one-time code verification flows.
//!
//! This module centralizes the common pieces used by code-based flows such as
//! DM pairing and any future manual verification flows:
//! - one-time code generation
//! - challenge presentation
//! - submission normalization
//! - pending challenge bookkeeping

use rand::Rng;
use serde::{Deserialize, Serialize};

/// User-facing payload for a code-based verification flow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationChallenge {
    /// One-time code the user must send back to the integration.
    pub code: String,
    /// Human-readable instructions for completing verification.
    pub instructions: String,
    /// Deep-link or shortcut URL that prefills the verification payload when supported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deep_link: Option<String>,
}

/// Pending one-time challenge plus flow-specific metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCodeChallenge<M> {
    pub code: String,
    pub meta: M,
    pub expires_at_unix: u64,
}

impl<M> PendingCodeChallenge<M> {
    pub fn new(code: String, meta: M, expires_at_unix: u64) -> Self {
        Self {
            code,
            meta,
            expires_at_unix,
        }
    }

    pub fn is_expired(&self, now_unix: u64) -> bool {
        self.expires_at_unix <= now_unix
    }
}

/// Shared seam for code-driven verification flows.
pub trait CodeChallengeFlow {
    type Meta: Clone;

    /// Issue a new one-time code for this flow.
    fn issue_code(&self) -> String;

    /// Render user-facing instructions for a pending challenge.
    fn render_challenge(&self, pending: &PendingCodeChallenge<Self::Meta>)
    -> VerificationChallenge;

    /// Normalize a submitted code before validation.
    fn normalize_submission(&self, submission: &str) -> Option<String> {
        normalize_submitted_code(submission)
    }

    /// Validate whether a submission satisfies the pending challenge.
    fn matches_submission(
        &self,
        pending: &PendingCodeChallenge<Self::Meta>,
        submission: &str,
    ) -> bool;

    /// Build a pending challenge with a flow-generated code.
    fn issue_challenge(
        &self,
        meta: Self::Meta,
        expires_at_unix: u64,
    ) -> PendingCodeChallenge<Self::Meta> {
        PendingCodeChallenge::new(self.issue_code(), meta, expires_at_unix)
    }
}

/// Trim user input and reject blank codes before hitting storage.
pub fn normalize_submitted_code(submission: &str) -> Option<String> {
    let trimmed = submission.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Generate a fixed-length code from the provided alphabet.
pub fn generate_code(len: usize, alphabet: &[u8]) -> String {
    if len == 0 || alphabet.is_empty() {
        return String::new();
    }

    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| {
            let idx = rng.gen_range(0..alphabet.len());
            alphabet[idx] as char
        })
        .collect()
}
