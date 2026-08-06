//! Channel-neutral in-chat gate-command grammar.
//!
//! The shared channel delivery driver advertises these commands in its
//! busy/prompt copy ("Reply `approve`/`deny` …", "Reply `auth deny
//! gate:<ref>` to decline it here"), so every channel adapter that carries
//! that copy must recognize the same grammar. One definition here — the
//! crate that owns the resolution payload types — keeps the advertised copy
//! and the parsing from drifting per channel: the 2026-07-17 second-channel
//! regression shipped exactly that drift (the busy hint advertised `auth
//! deny` on a channel with no parser, so the reply bounced off the busy
//! thread forever).
//!
//! Vendor-specific normalization (mention stripping, leading
//! @botname stripping) stays in each adapter, in front of this parser.
//! [`strip_wrapping_inline_code`] is exposed separately because every chat
//! surface renders the advertised command in backticks, so users on any
//! channel paste them back.

use crate::inbound::{
    ApprovalDecision, ApprovalResolutionPayload, AuthResolutionPayload, AuthResolutionResult,
    ProductInboundPayload, ScopedApprovalResolutionPayload,
};
use ironclaw_extension_contracts::channel_adapter::ProductTriggerReason;
use ironclaw_host_api::product_adapter_error::ProductAdapterError;
use ironclaw_host_api::product_adapter_error::RedactedString;

/// Strip symmetric wrapping backticks (repeatedly, with trimming) so a
/// pasted `` `approve gate:x` `` parses like the bare command.
pub fn strip_wrapping_inline_code(text: &str) -> &str {
    let mut rest = text.trim();
    while rest.len() >= 2 && rest.starts_with('`') && rest.ends_with('`') {
        rest = rest[1..rest.len() - 1].trim();
    }
    rest
}

/// Parse an already-normalized message text as an in-chat gate command.
///
/// Only a *confident* gate command — the reserved shape the system advertises:
/// a bare `approve`/`deny`, or any verb carrying a nonempty `gate:<ref>` (`approve
/// gate:<ref>`, `auth deny gate:<ref>`) — is pulled out of normal turn handling
/// and returned as `Some(payload)`.
///
/// Returns `Ok(None)` for everything else so it routes as a normal user
/// message: text that is not an interaction command at all, and — crucially —
/// ambiguous natural language that merely *starts* with a command verb but is
/// not the reserved shape (`"approve this design"`, a bare `auth deny` with no
/// ref, `auth deny gate:x extra`). Falling such text through to a turn is the
/// safe default; classifying it out of the conversation as a no-op would
/// silently swallow a real user message. (`Err` is still returned only when a
/// confident command carries a hostile/invalid ref that fails payload
/// validation.)
pub fn parse_interaction_resolution_text(
    text: &str,
    source_trigger: ProductTriggerReason,
) -> Result<Option<ProductInboundPayload>, ProductAdapterError> {
    let mut parts = text.split_whitespace();
    let Some(first) = parts.next() else {
        return Ok(None);
    };
    match first.to_ascii_lowercase().as_str() {
        "approve" => parse_approval_resolution(
            parts.next(),
            parts.next().is_some(),
            ApprovalDecision::ApproveOnce,
            source_trigger,
        ),
        "deny" => parse_approval_resolution(
            parts.next(),
            parts.next().is_some(),
            ApprovalDecision::Deny,
            source_trigger,
        ),
        "auth" => {
            let Some(action) = parts.next() else {
                return ambiguous_interaction_falls_through();
            };
            if action.eq_ignore_ascii_case("deny") {
                let Some(auth_request_ref) = parts.next() else {
                    return ambiguous_interaction_falls_through();
                };
                if parts.next().is_some() {
                    return ambiguous_interaction_falls_through();
                }
                let Some(auth_request_ref) = explicit_gate_ref(auth_request_ref)? else {
                    return ambiguous_interaction_falls_through();
                };
                AuthResolutionPayload::new(auth_request_ref, AuthResolutionResult::Denied)
                    .map(|payload| payload.with_source_trigger(source_trigger))
                    .map(ProductInboundPayload::AuthResolution)
                    .map(Some)
            } else {
                ambiguous_interaction_falls_through()
            }
        }
        _ => Ok(None),
    }
}

fn parse_approval_resolution(
    gate_ref: Option<&str>,
    has_trailing_input: bool,
    decision: ApprovalDecision,
    source_trigger: ProductTriggerReason,
) -> Result<Option<ProductInboundPayload>, ProductAdapterError> {
    match gate_ref {
        Some(gate_ref) => {
            // Any token that is not a `gate:<ref>` means this is not a
            // targeted resolution but ambiguous natural language that merely
            // starts with a verb ("approve this design").
            let Some(gate_ref) = explicit_gate_ref(gate_ref)? else {
                return ambiguous_interaction_falls_through();
            };
            // Authority-bearing targeted forms are exact. Extra words can
            // qualify or reverse the leading verb, so they must remain an
            // ordinary user message instead of being silently ignored.
            if has_trailing_input {
                return ambiguous_interaction_falls_through();
            }
            ApprovalResolutionPayload::new(gate_ref, decision)
                .map(|payload| payload.with_source_trigger(source_trigger))
                .map(ProductInboundPayload::ApprovalResolution)
                .map(Some)
        }
        None => ScopedApprovalResolutionPayload::new(decision)
            .map(|payload| payload.with_source_trigger(source_trigger))
            .map(ProductInboundPayload::ScopedApprovalResolution)
            .map(Some),
    }
}

fn explicit_gate_ref(token: &str) -> Result<Option<&str>, ProductAdapterError> {
    let Some(reference) = token.strip_prefix("gate:") else {
        return Ok(None);
    };
    if reference.is_empty() {
        return Err(ProductAdapterError::MalformedInboundPayload {
            reason: RedactedString::new("gate reference must not be empty"),
        });
    }
    Ok(Some(token))
}

/// Ambiguous input — a phrase that merely *starts* with a command verb but is
/// not the reserved confident gate-command shape (no `gate:<ref>`, or extra /
/// garbled args) — routes as a normal user message. Returning `Ok(None)` makes
/// the ingress fall through to normal turn handling instead of pulling the
/// message out of the conversation as a silent no-op (a lost user message).
fn ambiguous_interaction_falls_through()
-> Result<Option<ProductInboundPayload>, ProductAdapterError> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Option<ProductInboundPayload> {
        parse_interaction_resolution_text(
            strip_wrapping_inline_code(text),
            ProductTriggerReason::DirectChat,
        )
        .expect("grammar parse never errors on plain text")
    }

    #[test]
    fn auth_deny_with_gate_ref_parses_to_denied_resolution() {
        match parse("auth deny gate:auth-abc123") {
            Some(ProductInboundPayload::AuthResolution(payload)) => {
                assert_eq!(payload.auth_request_ref, "gate:auth-abc123");
                assert_eq!(payload.result, AuthResolutionResult::Denied);
            }
            other => panic!("expected AuthResolution, got {other:?}"),
        }
    }

    #[test]
    fn auth_deny_with_non_gate_text_falls_through_to_a_user_message() {
        assert!(
            parse("auth deny this").is_none(),
            "non-gate auth text is natural language, not a confident resolution"
        );
    }

    #[test]
    fn reserved_gate_tokens_require_a_nonempty_reference() {
        for text in ["auth deny gate:", "approve gate:"] {
            assert!(
                parse_interaction_resolution_text(text, ProductTriggerReason::DirectChat).is_err(),
                "{text:?} is malformed reserved syntax and must fail validation"
            );
        }
    }

    #[test]
    fn backtick_wrapped_paste_parses_like_bare_command() {
        // Every channel's busy hint renders the command in backticks; users
        // paste them back.
        assert!(matches!(
            parse("`auth deny gate:auth-abc123`"),
            Some(ProductInboundPayload::AuthResolution(_))
        ));
        assert!(matches!(
            parse("`approve gate:approval-1`"),
            Some(ProductInboundPayload::ApprovalResolution(_))
        ));
    }

    #[test]
    fn approve_and_deny_parse_targeted_and_scoped_forms() {
        assert!(matches!(
            parse("approve gate:approval-1"),
            Some(ProductInboundPayload::ApprovalResolution(_))
        ));
        assert!(matches!(
            parse("deny gate:approval-1"),
            Some(ProductInboundPayload::ApprovalResolution(_))
        ));
        assert!(matches!(
            parse("approve"),
            Some(ProductInboundPayload::ScopedApprovalResolution(_))
        ));
        assert!(matches!(
            parse("deny"),
            Some(ProductInboundPayload::ScopedApprovalResolution(_))
        ));
    }

    #[test]
    fn targeted_approval_and_denial_require_end_of_input() {
        for text in [
            "approve gate:approval-1 but do not run it",
            "deny gate:approval-1 because the scope changed",
        ] {
            assert!(
                parse(text).is_none(),
                "{text:?} is natural language, not an exact authority resolution"
            );
        }
    }

    #[test]
    fn ambiguous_verb_first_text_falls_through_to_a_user_message() {
        // A message that merely *starts* with a command verb but is not the
        // reserved gate-command shape (no `gate:` ref, or extra/garbled args)
        // is ambiguous natural language, not a confident gate command. It must
        // fall through to normal turn handling (route as a user message)
        // rather than being silently pulled out of the conversation as a
        // no-op — otherwise a real chat message like "approve this design" is
        // lost with no turn and no user-visible feedback.
        for text in [
            "auth",
            "auth revoke x",
            "auth deny",
            "auth deny gate:x extra",
            "approve this",
            "approve this design",
            "deny that idea",
        ] {
            assert!(
                parse(text).is_none(),
                "{text:?} is ambiguous natural language and must route as a user message"
            );
        }
    }

    #[test]
    fn ordinary_text_is_not_an_interaction_command() {
        for text in ["hello", "can you approve my PR tomorrow?", ""] {
            assert!(parse(text).is_none(), "{text:?} must route as user message");
        }
    }
}
