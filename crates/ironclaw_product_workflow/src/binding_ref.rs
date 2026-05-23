use ironclaw_turns::{IdempotencyKey, ReplyTargetBindingRef, SourceBindingRef};
use uuid::Uuid;

// Binding/idempotency newtypes cap at 256 bytes. The default leaves room for
// short product prefixes; auth continuations reserve extra space because their
// raw material combines flow, run, and gate identifiers.
pub(crate) const DEFAULT_BINDING_REF_RAW_MAX_BYTES: usize = 240;
pub(crate) const AUTH_CONTINUATION_BINDING_REF_RAW_MAX_BYTES: usize = 220;

pub(crate) fn bounded_source_binding_ref(
    prefix: &str,
    raw: &str,
    max_raw_len: usize,
) -> Result<SourceBindingRef, String> {
    SourceBindingRef::new(bounded_binding_ref_value(prefix, raw, max_raw_len))
}

pub(crate) fn bounded_reply_target_binding_ref(
    prefix: &str,
    raw: &str,
    max_raw_len: usize,
) -> Result<ReplyTargetBindingRef, String> {
    ReplyTargetBindingRef::new(bounded_binding_ref_value(prefix, raw, max_raw_len))
}

pub(crate) fn bounded_idempotency_key(
    prefix: &str,
    raw: &str,
    max_raw_len: usize,
) -> Result<IdempotencyKey, String> {
    IdempotencyKey::new(bounded_binding_ref_value(prefix, raw, max_raw_len))
}

fn bounded_binding_ref_value(prefix: &str, raw: &str, max_raw_len: usize) -> String {
    if raw.len() <= max_raw_len && !raw.chars().any(|c| c == '\0' || c.is_control()) {
        format!("{prefix}:{raw}")
    } else {
        let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, raw.as_bytes());
        format!("{prefix}:{id}")
    }
}
