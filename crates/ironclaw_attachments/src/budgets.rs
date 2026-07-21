/// Attachment-count and decoded-byte budgets shared by every product surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachmentBudgets {
    pub max_count: usize,
    pub max_file_bytes: usize,
    pub max_total_bytes: usize,
}

/// Current WebUI-compatible attachment budgets.
pub const DEFAULT_ATTACHMENT_BUDGETS: AttachmentBudgets = AttachmentBudgets {
    max_count: 10,
    max_file_bytes: 5 * 1024 * 1024,
    max_total_bytes: 10 * 1024 * 1024,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_budgets_match_webui_contract() {
        assert_eq!(DEFAULT_ATTACHMENT_BUDGETS.max_count, 10);
        assert_eq!(DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes, 5 * 1024 * 1024);
        assert_eq!(DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes, 10 * 1024 * 1024);
    }
}
