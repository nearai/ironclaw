use crate::code_challenge::{
    CodeChallengeFlow, PendingCodeChallenge, VerificationChallenge, generate_code,
    normalize_submitted_code,
};

const PAIRING_CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Shared flow for DM pairing code issuance and redemption.
#[derive(Debug, Clone)]
pub struct PairingCodeChallenge {
    channel: String,
}

impl PairingCodeChallenge {
    pub fn new(channel: impl Into<String>) -> Self {
        Self {
            channel: channel.into(),
        }
    }

    pub fn instructions(&self, code: &str) -> String {
        format!(
            "Enter this code in IronClaw to pair your {} account: `{}`. CLI fallback: `ironclaw pairing approve {} {}`",
            self.channel, code, self.channel, code
        )
    }

    pub fn reply_text(&self, code: &str) -> String {
        self.instructions(code)
    }
}

impl CodeChallengeFlow for PairingCodeChallenge {
    type Meta = ();

    fn issue_code(&self) -> String {
        generate_code(8, PAIRING_CODE_ALPHABET)
    }

    fn render_challenge(
        &self,
        pending: &PendingCodeChallenge<Self::Meta>,
    ) -> VerificationChallenge {
        VerificationChallenge {
            code: pending.code.clone(),
            instructions: self.instructions(&pending.code),
            deep_link: None,
        }
    }

    fn normalize_submission(&self, submission: &str) -> Option<String> {
        normalize_submitted_code(submission).map(|code| code.to_ascii_uppercase())
    }

    fn matches_submission(
        &self,
        pending: &PendingCodeChallenge<Self::Meta>,
        submission: &str,
    ) -> bool {
        self.normalize_submission(submission)
            .map(|code| code == pending.code)
            .unwrap_or(false)
    }
}
