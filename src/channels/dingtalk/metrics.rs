//! Anxiety-proxy telemetry helpers for the DingTalk anti-silence UX.
//!
//! These are thin pure-Rust helpers — counters live in tracing spans per
//! the `ironclaw-runtime-logging-pattern` contract (see
//! `docs/solutions/ironclaw-runtime-logging-pattern.md`). This module's
//! primary export is [`classify_repeat_ping`], which detects the short
//! "are you there?" messages that signal user anxiety when an in-flight
//! agent turn is taking too long.
//!
//! Plan reference: `docs/plans/2026-04-18-001-feat-dingtalk-anti-silence-ux-plan.md` (Unit 12).

use std::sync::LazyLock;

use regex::Regex;

/// Regex matching short "are you there?" style pings in both English and
/// zh-CN. Intentionally narrow — real questions that happen to contain
/// "hello" aren't counted (matches must be the entire short message).
static RE_REPEAT_PING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(hello\??|hi\??|you\s+there\??|still\s+there\??|anyone\??|在吗[?？]*|还在吗[?？]*|在不在[?？]*|有人吗[?？]*|在[?？]+|[?？]+)\s*$",
    )
    .unwrap()
});

/// Detect a short "repeat-ping" message — i.e. a user tapping the bot
/// because the bot appears frozen. Used by the stream ingest path to
/// increment an anxiety-proxy counter when the message arrives during
/// an in-flight turn.
pub fn classify_repeat_ping(msg: &str) -> bool {
    RE_REPEAT_PING.is_match(msg.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zh_cn_short_pings_match() {
        assert!(classify_repeat_ping("在吗?"));
        assert!(classify_repeat_ping("在吗"));
        assert!(classify_repeat_ping("还在吗？"));
        assert!(classify_repeat_ping("在不在?"));
        assert!(classify_repeat_ping("有人吗？"));
    }

    #[test]
    fn en_short_pings_match() {
        assert!(classify_repeat_ping("hello?"));
        assert!(classify_repeat_ping("Hi"));
        assert!(classify_repeat_ping("you there?"));
        assert!(classify_repeat_ping("still there?"));
        assert!(classify_repeat_ping("anyone"));
    }

    #[test]
    fn questions_alone_match() {
        assert!(classify_repeat_ping("??"));
        assert!(classify_repeat_ping("？？"));
    }

    #[test]
    fn real_questions_do_not_match() {
        // Substring matches must NOT trigger.
        assert!(!classify_repeat_ping("hello how are you"));
        assert!(!classify_repeat_ping("hi can you help me with X"));
        assert!(!classify_repeat_ping("在吗,我想问一下订单状态"));
        assert!(!classify_repeat_ping("what's up with the new api?"));
    }

    #[test]
    fn whitespace_tolerated() {
        assert!(classify_repeat_ping("  hello?  "));
        assert!(classify_repeat_ping("\n在吗?\n"));
    }

    #[test]
    fn empty_does_not_match() {
        assert!(!classify_repeat_ping(""));
        assert!(!classify_repeat_ping("    "));
    }
}
