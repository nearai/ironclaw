//! Consumer hook to observe an agent run's trajectory as it happens.
//!
//! The reborn runtime is intentionally sealed — the high-level
//! [`crate::RebornRuntime`] hands back only the final assistant reply, and the
//! per-step capability (tool) calls + their results live in internal stores not
//! otherwise exposed. A downstream caller (e.g. a benchmark harness that wants
//! to render a full step-by-step trajectory, or a UI/debugger) can install a
//! [`RebornTrajectoryObserver`] via
//! [`RebornRuntimeInput::with_trajectory_observer`](crate::RebornRuntimeInput::with_trajectory_observer)
//! to receive those events live.
//!
//! # Facade ownership
//!
//! [`RebornTrajectoryObserver`] is a composition-owned trait. The capability
//! port that drives the input hook lives in `ironclaw_loop_support` and speaks
//! its own [`ironclaw_loop_support::CapabilityTrajectoryObserver`]; rather than
//! re-export that substrate trait (which would commit this facade to a
//! loop-support contract — see `CLAUDE.md`: "expose facade-shaped handles only;
//! keep lower substrate handles private"), we define our own trait and adapt it
//! to the substrate one in [`as_capability_observer`]. Loop-support contract
//! changes therefore stay internal to the adapter instead of breaking the
//! public Reborn API.
//!
//! # Data exposure
//!
//! Capability inputs are the model's own tool arguments and results can contain
//! file contents, command output, or other sensitive material. The default
//! install ([`with_trajectory_observer`](crate::RebornRuntimeInput::with_trajectory_observer))
//! forwards a **bounded safe preview** (see [`safe_preview_value`]) so an
//! observer that projects events to logs/UI/telemetry stays within the same
//! truncation boundary the model-visible display path already enforces. A
//! consumer that genuinely needs the raw payloads (e.g. a trajectory recorder
//! that owns its own redaction + access control) must opt in explicitly via
//! [`with_raw_trajectory_observer`](crate::RebornRuntimeInput::with_raw_trajectory_observer).
//!
//! # Threading contract
//!
//! Callbacks fire inline on the per-capability hot path and are best-effort:
//! implementations must never block and must not rely on being called (a
//! panicking observer is caught and dropped at the call site — see
//! `HostRuntimeLoopCapabilityPort` / `LocalDevCapabilityIo`). An observer that
//! needs to do I/O or contend on a lock must hand the event to its own
//! non-blocking queue and return immediately.

use std::sync::Arc;

use ironclaw_loop_support::CapabilityTrajectoryObserver;

/// Receives each capability (tool) call's resolved input and result during a
/// reborn run. See the [module docs](self) for the data-exposure and threading
/// contract. `call_id` is the capability input ref and correlates the two
/// callbacks.
pub trait RebornTrajectoryObserver: std::fmt::Debug + Send + Sync {
    /// A model tool call resolved to a capability invocation. `capability_id`
    /// is the resolved capability (e.g. `builtin.shell`); `arguments` is the
    /// raw model-emitted tool-call input (before schema normalization).
    fn on_capability_input(
        &self,
        call_id: &str,
        capability_id: &str,
        arguments: &serde_json::Value,
    );
    /// The capability completed; `output` is the result JSON staged for the
    /// model.
    fn on_capability_result(&self, call_id: &str, capability_id: &str, output: &serde_json::Value);
}

/// Per-string truncation cap for the safe-preview projection. Mirrors the
/// bounded model-visible display boundary: enough to make a result legible in a
/// trajectory view without forwarding an unbounded blob.
const SAFE_PREVIEW_MAX_STRING_BYTES: usize = 512;
/// Per-array element cap for the safe-preview projection.
const SAFE_PREVIEW_MAX_ARRAY_ITEMS: usize = 32;
/// Per-object entry cap — objects are bounded the same way arrays are, so a
/// map with thousands of keys can't force unbounded clone/traversal.
const SAFE_PREVIEW_MAX_OBJECT_ENTRIES: usize = 32;
/// Maximum container nesting the preview descends before collapsing a subtree
/// to a marker. Bounds recursion depth on the capability hot path.
const SAFE_PREVIEW_MAX_DEPTH: usize = 8;
/// Hard ceiling on total nodes visited across the whole projection. Once spent,
/// the remaining subtree collapses to a marker — bounds total work for a value
/// that is wide *and* deep regardless of any single per-level cap.
const SAFE_PREVIEW_MAX_NODES: usize = 4096;

/// Bound a trajectory payload to a safe preview: long string leaves are
/// truncated (with a byte-count marker), and large/deep containers are capped
/// (per-level entry caps, a max depth, and a total-node budget) so neither a
/// wide nor a deeply nested value can force unbounded traversal/allocation on
/// the capability hot path. Object keys and structure are otherwise preserved
/// so a downstream view stays meaningful.
///
/// This is a size boundary, not a secret scrubber — a short credential still
/// passes — but it matches the existing display-preview truncation boundary so
/// the observer no longer bypasses it by forwarding the unbounded raw payload.
pub(crate) fn safe_preview_value(value: &serde_json::Value) -> serde_json::Value {
    let mut budget = SAFE_PREVIEW_MAX_NODES;
    safe_preview_inner(value, 0, &mut budget)
}

fn safe_preview_inner(
    value: &serde_json::Value,
    depth: usize,
    budget: &mut usize,
) -> serde_json::Value {
    use serde_json::Value;
    if *budget == 0 {
        return Value::String("[… preview node budget exhausted]".to_string());
    }
    *budget -= 1;
    match value {
        Value::String(s) => Value::String(truncate_string(s)),
        Value::Array(items) => {
            if depth >= SAFE_PREVIEW_MAX_DEPTH {
                return Value::String(format!("[… {} array items at max depth]", items.len()));
            }
            let mut bounded: Vec<Value> = Vec::new();
            for item in items.iter().take(SAFE_PREVIEW_MAX_ARRAY_ITEMS) {
                if *budget == 0 {
                    break;
                }
                bounded.push(safe_preview_inner(item, depth + 1, budget));
            }
            if items.len() > bounded.len() {
                bounded.push(Value::String(format!(
                    "[… {} more items omitted]",
                    items.len() - bounded.len()
                )));
            }
            Value::Array(bounded)
        }
        Value::Object(map) => {
            if depth >= SAFE_PREVIEW_MAX_DEPTH {
                return Value::String(format!("[… {} object entries at max depth]", map.len()));
            }
            let mut bounded = serde_json::Map::new();
            for (k, v) in map.iter().take(SAFE_PREVIEW_MAX_OBJECT_ENTRIES) {
                if *budget == 0 {
                    break;
                }
                bounded.insert(k.clone(), safe_preview_inner(v, depth + 1, budget));
            }
            if map.len() > bounded.len() {
                bounded.insert(
                    "…".to_string(),
                    Value::String(format!(
                        "{} more entries omitted",
                        map.len() - bounded.len()
                    )),
                );
            }
            Value::Object(bounded)
        }
        // Numbers, booleans, null are already bounded.
        other => other.clone(),
    }
}

fn truncate_string(s: &str) -> String {
    if s.len() <= SAFE_PREVIEW_MAX_STRING_BYTES {
        return s.to_string();
    }
    // Truncate on a char boundary so the result is valid UTF-8.
    let mut end = SAFE_PREVIEW_MAX_STRING_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… [truncated {} bytes]", &s[..end], s.len() - end)
}

/// Wraps a [`RebornTrajectoryObserver`] so it receives [`safe_preview_value`]
/// projections instead of raw payloads. Installed by the default
/// `with_trajectory_observer`; bypassed by `with_raw_trajectory_observer`.
#[derive(Debug)]
pub(crate) struct SafePreviewTrajectoryObserver {
    inner: Arc<dyn RebornTrajectoryObserver>,
}

impl SafePreviewTrajectoryObserver {
    pub(crate) fn wrap(
        inner: Arc<dyn RebornTrajectoryObserver>,
    ) -> Arc<dyn RebornTrajectoryObserver> {
        Arc::new(Self { inner })
    }
}

impl RebornTrajectoryObserver for SafePreviewTrajectoryObserver {
    fn on_capability_input(
        &self,
        call_id: &str,
        capability_id: &str,
        arguments: &serde_json::Value,
    ) {
        self.inner
            .on_capability_input(call_id, capability_id, &safe_preview_value(arguments));
    }

    fn on_capability_result(&self, call_id: &str, capability_id: &str, output: &serde_json::Value) {
        self.inner
            .on_capability_result(call_id, capability_id, &safe_preview_value(output));
    }
}

/// Adapts a composition-owned [`RebornTrajectoryObserver`] to the substrate
/// [`CapabilityTrajectoryObserver`] the loop-support capability port consumes,
/// so the facade trait never appears in this crate's loop-support boundary. The
/// substrate trait is input-only; the composition observer's result half is
/// driven separately by `LocalDevCapabilityIo`.
#[derive(Debug)]
struct CapabilityTrajectoryObserverAdapter {
    inner: Arc<dyn RebornTrajectoryObserver>,
}

impl CapabilityTrajectoryObserver for CapabilityTrajectoryObserverAdapter {
    fn on_capability_input(
        &self,
        call_id: &str,
        capability_id: &str,
        arguments: &serde_json::Value,
    ) {
        self.inner
            .on_capability_input(call_id, capability_id, arguments);
    }
}

/// Adapt a composition observer to the substrate observer the loop-support
/// capability port (the input hook) drives. The result hook lives on
/// `LocalDevCapabilityIo`, which calls the composition trait directly.
pub(crate) fn as_capability_observer(
    observer: Arc<dyn RebornTrajectoryObserver>,
) -> Arc<dyn CapabilityTrajectoryObserver> {
    Arc::new(CapabilityTrajectoryObserverAdapter { inner: observer })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn safe_preview_truncates_long_strings() {
        let long = "x".repeat(SAFE_PREVIEW_MAX_STRING_BYTES + 100);
        let preview = safe_preview_value(&json!({ "stdout": long }));
        let projected = preview["stdout"].as_str().expect("string leaf");
        assert!(
            projected.len() < SAFE_PREVIEW_MAX_STRING_BYTES + 100,
            "long string leaf should be truncated"
        );
        assert!(
            projected.contains("[truncated 100 bytes]"),
            "truncation marker should report the dropped byte count, got {projected}"
        );
    }

    #[test]
    fn safe_preview_caps_large_arrays() {
        let items: Vec<_> = (0..SAFE_PREVIEW_MAX_ARRAY_ITEMS + 10)
            .map(|i| json!(i))
            .collect();
        let preview = safe_preview_value(&json!(items));
        let arr = preview.as_array().expect("array");
        assert_eq!(arr.len(), SAFE_PREVIEW_MAX_ARRAY_ITEMS + 1);
        assert!(
            arr.last()
                .unwrap()
                .as_str()
                .unwrap()
                .contains("10 more items omitted"),
            "array cap should report the dropped count"
        );
    }

    #[test]
    fn safe_preview_preserves_small_payloads() {
        let payload = json!({"message": "hello", "count": 3, "ok": true});
        assert_eq!(safe_preview_value(&payload), payload);
    }

    #[test]
    fn safe_preview_caps_large_objects() {
        let mut map = serde_json::Map::new();
        for i in 0..SAFE_PREVIEW_MAX_OBJECT_ENTRIES + 10 {
            map.insert(format!("k{i}"), json!(i));
        }
        let preview = safe_preview_value(&Value::Object(map));
        let obj = preview.as_object().expect("object");
        // capped entries + one "…" marker entry
        assert_eq!(obj.len(), SAFE_PREVIEW_MAX_OBJECT_ENTRIES + 1);
        assert!(
            obj.get("…")
                .and_then(|v| v.as_str())
                .unwrap()
                .contains("10 more entries"),
            "object cap should report the dropped count"
        );
    }

    #[test]
    fn safe_preview_bounds_depth() {
        // Build a nesting deeper than the max-depth cap.
        let mut v = json!("leaf");
        for _ in 0..SAFE_PREVIEW_MAX_DEPTH + 5 {
            v = json!({ "next": v });
        }
        let preview = safe_preview_value(&v);
        // Descend to the cap; the subtree there must be collapsed to a marker
        // string rather than recursed further.
        let mut cur = &preview;
        let mut levels = 0;
        while let Some(next) = cur.get("next") {
            cur = next;
            levels += 1;
            assert!(
                levels <= SAFE_PREVIEW_MAX_DEPTH,
                "must not exceed max depth"
            );
        }
        assert!(
            cur.as_str()
                .is_some_and(|s| s.contains("at max depth") || s == "leaf"),
            "deepest visited node should be a collapse marker or the leaf, got {cur}"
        );
    }

    #[test]
    fn safe_preview_truncates_on_char_boundary() {
        // A multi-byte char straddling the cap must not panic or split a char.
        let s = format!("{}é", "a".repeat(SAFE_PREVIEW_MAX_STRING_BYTES - 1));
        let preview = safe_preview_value(&json!(s));
        // Round-trips as valid UTF-8 (no panic, no invalid slice).
        assert!(preview.as_str().is_some());
    }
}
