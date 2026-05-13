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
}

/// Sanitized, depth- and size-bounded view of capability arguments handed to
/// `before_capability` hooks.
///
/// The inner representation is sealed: only this crate can construct it. The
/// only public surface is querying for resolved/unresolved state and
/// extracting a numeric value at a JSON-pointer-like path. Hook authors must
/// not get back raw [`serde_json::Value`] handles, and they must treat the
/// "unresolved" state as a hard failure for any predicate that depends on
/// argument contents.
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

    /// Construct an unresolved view. Sealed to this crate.
    pub(crate) fn unresolved() -> Self {
        Self {
            inner: SanitizedArgumentsInner::Unresolved,
        }
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
        let (key, rest) = split_indexer(segment);
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

/// Split `items[0][1]` into ("items", [0, 1]). For a segment like `[0]`,
/// returns ("", [0]).
fn split_indexer(segment: &str) -> (&str, Vec<usize>) {
    let bracket = match segment.find('[') {
        Some(i) => i,
        None => return (segment, Vec::new()),
    };
    let key = &segment[..bracket];
    let mut rest = &segment[bracket..];
    let mut indices = Vec::new();
    while let Some(stripped) = rest.strip_prefix('[') {
        let close = match stripped.find(']') {
            Some(i) => i,
            None => return (key, Vec::new()), // malformed; treat as no-match
        };
        let idx_str = &stripped[..close];
        let Ok(idx) = idx_str.parse::<usize>() else {
            return (key, Vec::new());
        };
        indices.push(idx);
        rest = &stripped[close + 1..];
    }
    if !rest.is_empty() {
        // Trailing garbage after the last `]` — malformed.
        return (key, Vec::new());
    }
    (key, indices)
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
