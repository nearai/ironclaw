//! Inbound message filtering for the DingTalk channel.
//!
//! Three independent filters are composed in the processing pipeline:
//! 1. **Deduplication** — drop replayed messages (60 s TTL keyed by DingTalk msg_id).
//! 2. **Access control** — DM / group allowlists driven by `DingTalkConfig`.
//! 3. **@mention gate** — discard group messages that don't @-mention the bot (optional).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::config::{DingTalkConfig, DmPolicy, GroupPolicy};

const DEDUP_TTL: Duration = Duration::from_secs(60);

// ─── Deduplication ────────────────────────────────────────────────────────────

/// In-memory deduplication cache keyed by DingTalk `msg_id`.
///
/// Entries expire after [`DEDUP_TTL`] (60 seconds). Call [`DedupFilter::cleanup`]
/// periodically to reclaim memory from expired entries.
pub struct DedupFilter {
    seen: HashMap<String, Instant>,
}

impl DedupFilter {
    pub fn new() -> Self {
        Self {
            seen: HashMap::new(),
        }
    }

    /// Returns `true` if `msg_id` was already seen within the last 60 seconds.
    ///
    /// If the id is new (or its entry expired), it is inserted and `false` is returned.
    pub fn is_duplicate(&mut self, msg_id: &str) -> bool {
        let now = Instant::now();
        if let Some(&first_seen) = self.seen.get(msg_id) {
            if now.duration_since(first_seen) < DEDUP_TTL {
                tracing::debug!(msg_id, "DingTalk dedup: duplicate message dropped");
                return true;
            }
            // Expired entry — treat as fresh
        }
        self.seen.insert(msg_id.to_string(), now);
        false
    }

    /// Remove entries whose TTL has elapsed. Call this periodically to bound memory.
    pub fn cleanup(&mut self) {
        let now = Instant::now();
        self.seen
            .retain(|_, first_seen| now.duration_since(*first_seen) < DEDUP_TTL);
    }
}

impl Default for DedupFilter {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Access control ───────────────────────────────────────────────────────────

/// Returns `true` if the message should be allowed through the access-control gate.
///
/// # DM policy
/// - `Open` → always allow.
/// - `Allowlist` → `sender_id` must appear in `config.allow_from`; `*` acts as wildcard.
///
/// # Group policy
/// - `Open` → always allow.
/// - `Allowlist` → `conversation_id` must appear in `config.group_allow_from`; `*` wildcard.
/// - `Disabled` → always deny.
pub fn check_access(
    is_group: bool,
    conversation_id: &str,
    sender_id: &str,
    config: &DingTalkConfig,
) -> bool {
    if is_group {
        match config.group_policy {
            GroupPolicy::Open => {
                tracing::debug!(conversation_id, "DingTalk access: group open — allowed");
                true
            }
            GroupPolicy::Allowlist => {
                let allowed = config
                    .group_allow_from
                    .iter()
                    .any(|entry| entry == "*" || entry == conversation_id);
                tracing::debug!(
                    conversation_id,
                    allowed,
                    "DingTalk access: group allowlist check"
                );
                allowed
            }
            GroupPolicy::Disabled => {
                tracing::debug!(
                    conversation_id,
                    "DingTalk access: group disabled — denied"
                );
                false
            }
        }
    } else {
        match config.dm_policy {
            DmPolicy::Open => {
                tracing::debug!(sender_id, "DingTalk access: DM open — allowed");
                true
            }
            DmPolicy::Allowlist => {
                let allowed = config
                    .allow_from
                    .iter()
                    .any(|entry| entry == "*" || entry == sender_id);
                tracing::debug!(sender_id, allowed, "DingTalk access: DM allowlist check");
                allowed
            }
        }
    }
}

// ─── @mention gate ────────────────────────────────────────────────────────────

/// Returns `true` if the message should be forwarded to the agent.
///
/// - DMs are always processed regardless of `require_mention`.
/// - Group messages: if `require_mention` is `true` the bot must be in the
///   `is_in_at_list`; otherwise always processed.
pub fn should_process(
    is_in_at_list: Option<bool>,
    is_group: bool,
    require_mention: bool,
) -> bool {
    if !is_group {
        // DMs are always processed
        return true;
    }
    if !require_mention {
        return true;
    }
    let mentioned = is_in_at_list.unwrap_or(false);
    tracing::debug!(mentioned, "DingTalk mention gate check");
    mentioned
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    use crate::config::{CardStreamMode, DingTalkConfig, DmPolicy, GroupPolicy};

    fn make_config(
        dm_policy: DmPolicy,
        group_policy: GroupPolicy,
        allow_from: Vec<String>,
        group_allow_from: Vec<String>,
    ) -> DingTalkConfig {
        DingTalkConfig {
            enabled: true,
            client_id: "test-id".to_string(),
            client_secret: SecretString::from("test-secret".to_string()),
            robot_code: None,
            card_template_id: None,
            card_stream_mode: CardStreamMode::Off,
            card_stream_interval_ms: 1000,
            require_mention: false,
            dm_policy,
            group_policy,
            allow_from,
            group_allow_from,
            max_reconnect_cycles: 3,
            reconnect_deadline_ms: 10_000,
            additional_accounts: vec![],
        }
    }

    // ── DedupFilter ──────────────────────────────────────────────────────────

    #[test]
    fn dedup_first_message_not_duplicate() {
        let mut f = DedupFilter::new();
        assert!(!f.is_duplicate("msg-001"));
    }

    #[test]
    fn dedup_second_call_is_duplicate() {
        let mut f = DedupFilter::new();
        f.is_duplicate("msg-002");
        assert!(f.is_duplicate("msg-002"));
    }

    #[test]
    fn dedup_different_ids_not_duplicate() {
        let mut f = DedupFilter::new();
        f.is_duplicate("msg-a");
        assert!(!f.is_duplicate("msg-b"));
    }

    #[test]
    fn dedup_cleanup_removes_expired() {
        let mut f = DedupFilter::new();
        // Manually insert an expired entry
        f.seen
            .insert("old-msg".to_string(), Instant::now() - Duration::from_secs(61));
        f.cleanup();
        assert!(!f.seen.contains_key("old-msg"));
    }

    #[test]
    fn dedup_cleanup_keeps_fresh() {
        let mut f = DedupFilter::new();
        f.is_duplicate("fresh-msg");
        f.cleanup();
        assert!(f.seen.contains_key("fresh-msg"));
    }

    // ── check_access — DM ────────────────────────────────────────────────────

    #[test]
    fn dm_open_allows_any_sender() {
        let cfg = make_config(DmPolicy::Open, GroupPolicy::Disabled, vec![], vec![]);
        assert!(check_access(false, "", "user-123", &cfg));
    }

    #[test]
    fn dm_allowlist_permits_listed_sender() {
        let cfg = make_config(
            DmPolicy::Allowlist,
            GroupPolicy::Disabled,
            vec!["user-abc".to_string()],
            vec![],
        );
        assert!(check_access(false, "", "user-abc", &cfg));
    }

    #[test]
    fn dm_allowlist_blocks_unlisted_sender() {
        let cfg = make_config(
            DmPolicy::Allowlist,
            GroupPolicy::Disabled,
            vec!["user-abc".to_string()],
            vec![],
        );
        assert!(!check_access(false, "", "user-xyz", &cfg));
    }

    #[test]
    fn dm_allowlist_wildcard_allows_all() {
        let cfg = make_config(
            DmPolicy::Allowlist,
            GroupPolicy::Disabled,
            vec!["*".to_string()],
            vec![],
        );
        assert!(check_access(false, "", "anyone", &cfg));
    }

    // ── check_access — Group ─────────────────────────────────────────────────

    #[test]
    fn group_open_allows_any_conversation() {
        let cfg = make_config(DmPolicy::Open, GroupPolicy::Open, vec![], vec![]);
        assert!(check_access(true, "conv-999", "user-x", &cfg));
    }

    #[test]
    fn group_disabled_blocks_all() {
        let cfg = make_config(DmPolicy::Open, GroupPolicy::Disabled, vec![], vec![]);
        assert!(!check_access(true, "conv-999", "user-x", &cfg));
    }

    #[test]
    fn group_allowlist_permits_listed_conversation() {
        let cfg = make_config(
            DmPolicy::Open,
            GroupPolicy::Allowlist,
            vec![],
            vec!["conv-allowed".to_string()],
        );
        assert!(check_access(true, "conv-allowed", "user-x", &cfg));
    }

    #[test]
    fn group_allowlist_blocks_unlisted_conversation() {
        let cfg = make_config(
            DmPolicy::Open,
            GroupPolicy::Allowlist,
            vec![],
            vec!["conv-allowed".to_string()],
        );
        assert!(!check_access(true, "conv-other", "user-x", &cfg));
    }

    #[test]
    fn group_allowlist_wildcard_allows_all() {
        let cfg = make_config(
            DmPolicy::Open,
            GroupPolicy::Allowlist,
            vec![],
            vec!["*".to_string()],
        );
        assert!(check_access(true, "any-conv", "user-x", &cfg));
    }

    // ── should_process ───────────────────────────────────────────────────────

    #[test]
    fn dm_always_processed_regardless_of_mention() {
        assert!(should_process(None, false, true));
        assert!(should_process(Some(false), false, true));
    }

    #[test]
    fn group_no_require_mention_always_processed() {
        assert!(should_process(None, true, false));
        assert!(should_process(Some(false), true, false));
    }

    #[test]
    fn group_require_mention_passes_when_in_at_list() {
        assert!(should_process(Some(true), true, true));
    }

    #[test]
    fn group_require_mention_blocked_when_not_in_at_list() {
        assert!(!should_process(Some(false), true, true));
    }

    #[test]
    fn group_require_mention_blocked_when_at_list_absent() {
        assert!(!should_process(None, true, true));
    }
}
