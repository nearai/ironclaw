//! Context for the `before_capability` hook point.

use ironclaw_host_api::{ExtensionId, TenantId};
use rust_decimal::Decimal;
use std::str::FromStr;

/// Maximum byte length for any single string value retained in
/// [`SanitizedArguments`]. Longer strings are truncated.
const MAX_STRING_BYTES: usize = 256;

/// Maximum nesting depth retained in [`SanitizedArguments`]. Deeper values are
/// replaced with `serde_json::Value::Null`.
const MAX_DEPTH: usize = 8;

/// Read-only context handed to a `before_capability` hook.
///
/// Marked `#[non_exhaustive]` so additional fields can be added (capability
/// arguments digest, run id, iteration, surface version, etc.) without
/// breaking existing hook authors when this crate composes with the rest of
/// the Reborn loop wiring.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BeforeCapabilityHookContext {
    pub tenant_id: TenantId,
    pub capability_name: String,
    /// The dispatcher's *opaque* digest of the capability arguments. Hook
    /// authors can compare this digest across calls (e.g., for repetition
    /// detection) but cannot read the underlying args; raw args never reach
    /// hook scope.
    pub arguments_digest: [u8; 32],
    /// Sanitized view of the capability arguments. Whether resolution
    /// succeeded depends on the middleware's configured
    /// [`crate::middleware::CapabilityInputResolver`]; predicate evaluation
    /// that requires numeric extraction fails closed when this is
    /// [`SanitizedArguments::is_resolved`] = `false`.
    pub arguments: SanitizedArguments,
    /// Capability provider extension, when known. `None` means the middleware
    /// could not resolve a provider for this capability (e.g. host-supplied
    /// builtin, or no resolver wired in). Hook scope enforcement treats the
    /// `None` case conservatively: an `OwnCapabilities`-scoped Installed hook
    /// will not fire when the provider is unknown.
    pub provider: Option<ExtensionId>,
    /// Stable per-invocation identity used by predicate state backends for
    /// replay/idempotency dedup. The middleware threads this through from
    /// the calling layer (e.g. a digest of the runtime event id or the
    /// capability-invocation surface) so that retried/replayed invocations
    /// produce the same id — letting the backend's `event_id`
    /// UNIQUE-constraint short-circuit duplicate counter writes.
    ///
    /// When `None`, the evaluator synthesizes a per-call-unique id and
    /// dedup degrades to "every evaluation counts" semantics — appropriate
    /// for the in-memory backend without replay, but **not** for durable
    /// backends. Durable callers MUST supply a value.
    pub caller_event_id: Option<crate::predicate_state::PredicateEventId>,
}

impl BeforeCapabilityHookContext {
    /// Construct a context with an explicit [`SanitizedArguments`] view and
    /// resolved capability provider.
    pub fn new(
        tenant_id: TenantId,
        capability_name: String,
        arguments_digest: [u8; 32],
        arguments: SanitizedArguments,
        provider: Option<ExtensionId>,
    ) -> Self {
        Self {
            tenant_id,
            capability_name,
            arguments_digest,
            arguments,
            provider,
            caller_event_id: None,
        }
    }

    /// Convenience constructor for callers (mostly tests and middleware
    /// without a configured resolver) where the arguments view is
    /// intentionally unresolved and the provider is unknown.
    pub fn new_unresolved(
        tenant_id: TenantId,
        capability_name: String,
        arguments_digest: [u8; 32],
    ) -> Self {
        Self::new(
            tenant_id,
            capability_name,
            arguments_digest,
            SanitizedArguments::unresolved(),
            None,
        )
    }

    /// Builder-style setter for the stable per-invocation event id used by
    /// predicate-state replay dedup. See [`Self::caller_event_id`].
    ///
    /// Rejects empty strings and ids containing NUL bytes (henrypark133
    /// must-fix #3 on PR #3635). Durable backends that constrain ids to
    /// UUIDs or other shapes would fail cryptically on such values; rather
    /// than ship an inconsistent surface, the in-memory path enforces a
    /// minimum format invariant at construction. If validation fails, the
    /// returned context has `caller_event_id == None` and predicate
    /// dedup falls through to the synth path.
    #[must_use]
    pub fn with_caller_event_id(
        mut self,
        caller_event_id: crate::predicate_state::PredicateEventId,
    ) -> Self {
        if caller_event_id.as_str().is_empty() || caller_event_id.as_str().as_bytes().contains(&0) {
            tracing::warn!("caller_event_id rejected: must be non-empty and contain no NUL bytes");
            return self;
        }
        self.caller_event_id = Some(caller_event_id);
        self
    }
}

/// Sanitized, depth- and size-bounded view of capability arguments handed to
/// `before_capability` hooks.
///
/// The inner representation is sealed: only this crate can construct it. The
/// only public extraction surface is [`Self::is_resolved`] and
/// [`Self::extract_numeric`] — a hook can ask "what's the numeric value at
/// this named path?", and **nothing else**. There is no `as_json`, no
/// iteration, no key listing, no string accessor. Hook authors who want a
/// path's value must (a) know its name and (b) accept it as a numeric.
///
/// # Field-level scope (threat-model finding I2)
///
/// The narrow surface is the mitigation: even an Installed-tier hook with
/// broad scope cannot enumerate or exfiltrate full capability arguments,
/// because the API only resolves one named numeric at a time. A hook
/// querying many paths is observable in the audit log (each
/// `extract_numeric` call goes through this crate, not the inner JSON).
///
/// A future programmatic-hook surface (WASM) that wants richer arg access
/// must thread the *manifest-declared* `field_path` allowlist through the
/// resolver, not bypass it; the current predicate path enforces this by
/// construction because the predicate spec itself names the field.
#[derive(Debug, Clone)]
pub struct SanitizedArguments {
    inner: SanitizedArgumentsInner,
}

#[derive(Debug, Clone)]
pub(crate) enum SanitizedArgumentsInner {
    /// No arguments resolved — middleware didn't have a resolver, or
    /// resolution failed. `NumericSum` predicates with this state fail
    /// closed (return Deny / PauseApproval per `on_exceeded`).
    Unresolved,
    /// Resolved sanitized JSON. Strings are truncated to
    /// [`MAX_STRING_BYTES`]; objects/arrays nested deeper than
    /// [`MAX_DEPTH`] are replaced with null.
    Resolved(serde_json::Value),
}

impl SanitizedArguments {
    /// True iff the middleware was able to resolve capability arguments.
    pub fn is_resolved(&self) -> bool {
        matches!(self.inner, SanitizedArgumentsInner::Resolved(_))
    }

    /// Extract a numeric value at `field_path`. Path syntax supports dotted
    /// keys (`order.amount`) and zero-based array indexing
    /// (`items[0].price`). Returns `None` when:
    ///
    /// - arguments are unresolved
    /// - the path doesn't exist
    /// - the value at the path is not numeric or numeric-string parseable
    ///   as [`Decimal`]
    pub fn extract_numeric(&self, field_path: &str) -> Option<Decimal> {
        let value = match &self.inner {
            SanitizedArgumentsInner::Unresolved => return None,
            SanitizedArgumentsInner::Resolved(v) => v,
        };
        let target = resolve_path(value, field_path)?;
        value_to_decimal(target)
    }

    /// Construct an unresolved view. This is the safe default — predicates
    /// that need to inspect arguments must fail closed against it, so
    /// exposing the constructor cannot weaken any trust property. External
    /// hook authors call this when building `BeforeCapabilityHookContext`
    /// values for testing their own predicates without standing up the
    /// full resolver wiring.
    ///
    /// The mirror constructor `from_json` stays sealed: that one performs
    /// the sanitization (depth + size bounds) that is the trust boundary,
    /// and external callers must not be able to bypass it.
    pub fn unresolved() -> Self {
        Self {
            inner: SanitizedArgumentsInner::Unresolved,
        }
    }

    /// Construct a resolved view from a serde_json value **for tests
    /// only**, applying the same sanitization (depth + size bounds) as
    /// the production path. Exposed under the `test-support` feature so
    /// hook authors can TDD `NumericSum`-style predicates without
    /// standing up the full Reborn resolver wiring.
    ///
    /// Production builds must NOT enable `test-support`; the constructor
    /// it exposes lets callers bypass the path that the production
    /// resolver would otherwise own, breaking the "resolved args came
    /// from a trusted resolver" invariant.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_tests(value: serde_json::Value) -> Self {
        Self::from_json(value)
    }

    /// Construct a resolved view, applying sanitization (string truncation,
    /// depth capping). Sealed to this crate so external callers can't bypass
    /// the bounds.
    pub(crate) fn from_json(value: serde_json::Value) -> Self {
        Self {
            inner: SanitizedArgumentsInner::Resolved(sanitize(value, 0)),
        }
    }
}

fn sanitize(value: serde_json::Value, depth: usize) -> serde_json::Value {
    if depth >= MAX_DEPTH {
        // At max depth, primitives are kept; nested compounds are dropped.
        return match value {
            serde_json::Value::Object(_) | serde_json::Value::Array(_) => serde_json::Value::Null,
            other => sanitize_leaf(other),
        };
    }
    match value {
        serde_json::Value::String(s) => serde_json::Value::String(truncate_string(s)),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(|v| sanitize(v, depth + 1)).collect())
        }
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(truncate_string(k), sanitize(v, depth + 1));
            }
            serde_json::Value::Object(out)
        }
        other => other,
    }
}

fn sanitize_leaf(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => serde_json::Value::String(truncate_string(s)),
        other => other,
    }
}

fn truncate_string(s: String) -> String {
    if s.len() <= MAX_STRING_BYTES {
        return s;
    }
    // Truncate on a UTF-8 char boundary, not raw byte offset.
    let mut end = MAX_STRING_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::with_capacity(end);
    out.push_str(&s[..end]);
    out
}

/// Navigate `value` using a `foo.bar[0].baz`-style path.
///
/// Returns `None` for any malformed segment — in particular, a segment
/// containing a malformed indexer such as `amount[foo]` or `amount[`
/// fails the whole resolution rather than silently falling back to the
/// parent field. Failing closed on malformed paths prevents a typo in a
/// manifest from quietly evaluating a `NumericSum` predicate against the
/// wrong field (which would otherwise allow calls that should fail
/// closed as unresolved).
fn resolve_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    if path.is_empty() {
        return Some(value);
    }
    let mut current = value;
    for segment in path.split('.') {
        if segment.is_empty() {
            return None;
        }
        // Split a segment like `items[0][1]` into key=`items`, indices=[0,1].
        // `split_indexer` returns `None` on malformed bracket syntax; we
        // propagate that as a path-resolution failure.
        let (key, rest) = split_indexer(segment)?;
        if !key.is_empty() {
            current = current.as_object()?.get(key)?;
        } else if rest.is_empty() {
            // Leading [..] with no key is invalid except as the very first
            // segment applied to an array — and even then the user should
            // write `[0]` as a path segment, which split_indexer handles.
            return None;
        }
        for idx in rest {
            current = current.as_array()?.get(idx)?;
        }
    }
    Some(current)
}

/// Split `items[0][1]` into `Some(("items", [0, 1]))`. For a segment like
/// `[0]`, returns `Some(("", [0]))`. Returns `None` on any malformed
/// bracket syntax — non-numeric index, missing close-bracket, trailing
/// garbage. Callers must treat a `None` here as a path-resolution failure
/// (fail-closed for predicates), not as the parent field.
fn split_indexer(segment: &str) -> Option<(&str, Vec<usize>)> {
    let bracket = match segment.find('[') {
        Some(i) => i,
        None => return Some((segment, Vec::new())),
    };
    let key = &segment[..bracket];
    let mut rest = &segment[bracket..];
    let mut indices = Vec::new();
    while let Some(stripped) = rest.strip_prefix('[') {
        let close = stripped.find(']')?;
        let idx_str = &stripped[..close];
        let idx = idx_str.parse::<usize>().ok()?;
        indices.push(idx);
        rest = &stripped[close + 1..];
    }
    if !rest.is_empty() {
        // Trailing garbage after the last `]` — malformed.
        return None;
    }
    Some((key, indices))
}

fn value_to_decimal(value: &serde_json::Value) -> Option<Decimal> {
    match value {
        serde_json::Value::Number(n) => {
            // Prefer exact decimal parse via the string form to avoid f64
            // round-trip surprises with money-like values.
            Decimal::from_str(&n.to_string()).ok()
        }
        serde_json::Value::String(s) => Decimal::from_str(s.trim()).ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_extract_returns_none() {
        let args = SanitizedArguments::unresolved();
        assert!(!args.is_resolved());
        assert_eq!(args.extract_numeric("any.path"), None);
    }

    #[test]
    fn extract_numeric_from_top_level() {
        let args = SanitizedArguments::from_json(serde_json::json!({"amount": 42}));
        assert!(args.is_resolved());
        assert_eq!(args.extract_numeric("amount"), Some(Decimal::from(42)));
    }

    #[test]
    fn extract_numeric_from_nested_object() {
        let args = SanitizedArguments::from_json(serde_json::json!({
            "order": { "amount": "100.50" }
        }));
        assert_eq!(
            args.extract_numeric("order.amount"),
            Some(Decimal::from_str("100.50").expect("ok"))
        );
    }

    #[test]
    fn extract_numeric_from_array_index() {
        let args = SanitizedArguments::from_json(serde_json::json!({
            "items": [{"price": 10}, {"price": 20}]
        }));
        assert_eq!(
            args.extract_numeric("items[0].price"),
            Some(Decimal::from(10))
        );
        assert_eq!(
            args.extract_numeric("items[1].price"),
            Some(Decimal::from(20))
        );
    }

    #[test]
    fn extract_numeric_missing_path_is_none() {
        let args = SanitizedArguments::from_json(serde_json::json!({"a": 1}));
        assert_eq!(args.extract_numeric("b"), None);
        assert_eq!(args.extract_numeric("a.b"), None);
        assert_eq!(args.extract_numeric("a[0]"), None);
    }

    #[test]
    fn extract_non_numeric_value_is_none() {
        let args = SanitizedArguments::from_json(serde_json::json!({"a": "not-a-number"}));
        assert_eq!(args.extract_numeric("a"), None);
    }

    /// Malformed bracket syntax must fail closed (`None`), not silently
    /// fall back to the parent field. Without this guarantee, a typo'd
    /// `NumericSum` predicate field path would evaluate against the wrong
    /// value and allow calls that should have been rejected as
    /// unresolved/malformed input.
    #[test]
    fn malformed_indexer_returns_none_not_parent_value() {
        let args = SanitizedArguments::from_json(serde_json::json!({"amount": 42}));
        // Non-numeric index — must NOT resolve to the parent `amount`.
        assert_eq!(args.extract_numeric("amount[foo]"), None);
        // Unterminated bracket.
        assert_eq!(args.extract_numeric("amount["), None);
        // Trailing garbage after the closing bracket.
        assert_eq!(args.extract_numeric("amount[0]xyz"), None);
    }

    #[test]
    fn long_strings_are_truncated() {
        let big = "x".repeat(MAX_STRING_BYTES + 100);
        let args = SanitizedArguments::from_json(serde_json::json!({"note": big.clone()}));
        // Original would be 356 bytes; sanitized is 256.
        let SanitizedArgumentsInner::Resolved(v) = &args.inner else {
            panic!("expected resolved");
        };
        let stored = v.get("note").and_then(|s| s.as_str()).expect("note");
        assert_eq!(stored.len(), MAX_STRING_BYTES);
    }

    #[test]
    fn deeply_nested_objects_are_capped() {
        // Build a 12-deep nested object; depth 8+ should collapse to null.
        let mut v = serde_json::json!({"leaf": 1});
        for _ in 0..12 {
            v = serde_json::json!({"n": v});
        }
        let args = SanitizedArguments::from_json(v);
        // Walking 12 deep no longer reaches an object at the bottom.
        let path = "n.n.n.n.n.n.n.n.n.n.n.n.leaf";
        assert_eq!(args.extract_numeric(path), None);
    }
}
