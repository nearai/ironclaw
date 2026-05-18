use std::io::{self, BufRead, IsTerminal, Write};

use async_trait::async_trait;

use crate::secrets::{Approval, ApprovalRequest, SignerApprovalGate, SignerClassification};

pub struct TerminalSignerGate;

impl TerminalSignerGate {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TerminalSignerGate {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SignerApprovalGate for TerminalSignerGate {
    async fn request_approval(&self, request: &ApprovalRequest) -> Approval {
        let stdin = io::stdin();
        if !stdin.is_terminal() {
            tracing::error!(
                tool = %request.tool_name,
                secret = %request.secret_name,
                "non-interactive stdin and no gate impl can reach the user; denying signing"
            );
            return Approval::Denied;
        }

        let mut stdout = io::stdout().lock();
        let _ = writeln!(stdout);
        let _ = writeln!(stdout, "[signing approval requested]");
        let _ = writeln!(
            stdout,
            "  tool:           {} (secret '{}')",
            request.tool_name, request.secret_name
        );
        let _ = writeln!(stdout, "  endpoint:       {}{}", request.host, request.path);
        let _ = writeln!(
            stdout,
            "  classification: {}",
            classification_label(request.classification)
        );
        let _ = writeln!(stdout, "  signer:         {}", request.summary);
        let _ = writeln!(
            stdout,
            "Approve this signature? Type 'approve' (or 'a'/'yes'/'y') to allow,"
        );
        let _ = writeln!(stdout, "anything else (e.g. 'deny'/'no') to refuse.");
        let _ = write!(stdout, "> ");
        let _ = stdout.flush();
        drop(stdout);

        let mut input = String::new();
        if stdin.lock().read_line(&mut input).is_err() {
            return Approval::Denied;
        }
        let normalized = input.trim().to_ascii_lowercase();
        if matches!(normalized.as_str(), "approve" | "a" | "yes" | "y") {
            Approval::Approved
        } else {
            Approval::Denied
        }
    }
}

fn classification_label(c: SignerClassification) -> &'static str {
    match c {
        SignerClassification::Low => "low",
        SignerClassification::Medium => "medium",
        SignerClassification::High => "high",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_label_covers_all_levels() {
        assert_eq!(classification_label(SignerClassification::Low), "low");
        assert_eq!(classification_label(SignerClassification::Medium), "medium");
        assert_eq!(classification_label(SignerClassification::High), "high");
    }

    #[tokio::test]
    async fn terminal_gate_denies_when_stdin_not_a_tty() {
        let gate = TerminalSignerGate::new();
        let request = ApprovalRequest {
            tool_name: "polymarket-clob".to_string(),
            host: "clob.polymarket.com".to_string(),
            path: "/auth/api-key".to_string(),
            secret_name: "polymarket_l1_pk".to_string(),
            classification: SignerClassification::High,
            summary: "EIP-712 typed message: ClobAuth".to_string(),
        };
        let approval = gate.request_approval(&request).await;
        assert_eq!(approval, Approval::Denied);
    }
}
