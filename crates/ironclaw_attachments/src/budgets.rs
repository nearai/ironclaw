/// Attachment-count and decoded-byte budgets shared by every product surface.
///
/// Serde-capable so wire DTOs can embed it with `#[serde(flatten)]` instead of
/// re-declaring the same three fields and hand-copying them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttachmentBudgets {
    pub max_count: usize,
    pub max_file_bytes: usize,
    pub max_total_bytes: usize,
}

/// Current WebUI-compatible attachment budgets.
pub const DEFAULT_ATTACHMENT_BUDGETS: AttachmentBudgets = AttachmentBudgets {
    max_count: 10,
    max_file_bytes: 10 * 1024 * 1024,
    max_total_bytes: 10 * 1024 * 1024,
};

/// Browser-facing inline-attachment contract.
///
/// Carries the `accept` tokens generated from the shared
/// [`ironclaw_common`] format registry (so a file picker can never drift from
/// the server's allowed MIME set) plus the budgets the server-side decode
/// enforces. A surface uses this only for pre-submit hints; the server-side
/// decode remains the sole authority on what is accepted.
///
/// It lives beside [`AttachmentBudgets`] because this crate is the one home for
/// attachment size ceilings (PROPOSAL §6.4.9): a transport that advertises a
/// ceiling and the routine that enforces it must read the same constant.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttachmentCapabilities {
    /// HTML file-input `accept` tokens from the shared registry: exact MIME
    /// types plus extensions, e.g. `["image/png", ".png", "application/pdf",
    /// ".pdf"]` — never `image/*` wildcards (which would advertise unsupported
    /// formats, and which break folder navigation in the native macOS picker).
    pub accept: Vec<String>,
    /// The count/byte budgets the decode enforces. Flattened, so the wire shape
    /// is unchanged and a new budget field reaches the browser without an
    /// intermediate edit here.
    #[serde(flatten)]
    pub budgets: AttachmentBudgets,
}

/// The inline-attachment contract advertised to browsers. Generated from the
/// shared format registry and the budgets the decode enforces, so the picker
/// and the server stay in lockstep by construction.
pub fn attachment_capabilities() -> AttachmentCapabilities {
    AttachmentCapabilities {
        accept: ironclaw_common::accept_tokens(),
        budgets: DEFAULT_ATTACHMENT_BUDGETS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertised_capabilities_carry_the_enforced_budgets_and_registry_tokens() {
        let advertised = attachment_capabilities();
        assert_eq!(
            advertised.budgets, DEFAULT_ATTACHMENT_BUDGETS,
            "the advertised ceiling must be the enforced ceiling"
        );
        assert_eq!(
            advertised.accept,
            ironclaw_common::accept_tokens(),
            "accept tokens come from the shared registry, never a local list"
        );
        assert!(
            !advertised.accept.iter().any(|token| token.contains('*')),
            "wildcards would advertise unsupported formats: {:?}",
            advertised.accept
        );
    }

    /// The budgets are `#[serde(flatten)]`ed, so the browser sees one flat
    /// object. A nested `budgets` key would silently break every client that
    /// reads `max_file_bytes` at the top level.
    #[test]
    fn advertised_capabilities_serialize_the_budgets_flat() {
        let json = serde_json::to_value(attachment_capabilities()).expect("serialize");
        let object = json.as_object().expect("object");
        assert!(object.contains_key("accept"));
        assert!(object.contains_key("max_count"));
        assert!(object.contains_key("max_file_bytes"));
        assert!(object.contains_key("max_total_bytes"));
        assert!(
            !object.contains_key("budgets"),
            "budgets must stay flattened"
        );
    }

    #[test]
    fn default_budgets_match_webui_contract() {
        assert_eq!(DEFAULT_ATTACHMENT_BUDGETS.max_count, 10);
        assert_eq!(DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes, 10 * 1024 * 1024);
        assert_eq!(DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes, 10 * 1024 * 1024);
    }
}
