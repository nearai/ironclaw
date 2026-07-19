//! Slice-C kernel vocabulary — the closed runtime-lane set.
//!
//! Part of the "less `dyn`" move (arch-simplification §4.2, §5.5, §5.9). Today
//! `RuntimeAdapter` is an open trait with a closed, enumerable set of impls
//! (`FirstParty`, `Wasm`, `Mcp`, script/process, plus a resolver wrapper) used as
//! a trait object — "an open trait buys extensibility exactly where it is not
//! needed" (§1.3). The target replaces that `dyn` with a **closed
//! `enum RuntimeLane`**: adding a lane becomes a compile error until every
//! `match` handles it.
//!
//! ## Why a closed enum here, when deployment *modes* become data
//!
//! Both are small, closed, security-relevant sets, but they differ in *who may
//! branch on them* (§4.4). A `RuntimeLane` is branched on in exactly one place —
//! `dispatch()`'s match — and **exhaustiveness there is the safety property**: a
//! new lane must confront every arm. (A deployment mode, by contrast, must be
//! branched on in *zero* places past the composition edge, so it stays data, not
//! a type.)
//!
//! ## `RuntimeLane` is not [`crate::RuntimeKind`]
//!
//! They look similar but are different concepts the doc deliberately separates
//! (§5.10): `RuntimeLane` is the **execution/trust boundary** — the untrusted
//! surface `dispatch()` hands mediated handles to (§5.4) — a closed set of four.
//! [`crate::RuntimeKind`] is a **loading detail / taxonomy** (`Wasm`/`Mcp`/
//! `Script`/`FirstParty`/`System`); e.g. a `Script` runtime executes *on* the
//! `Process` lane, and `System` is host-internal, not an untrusted lane. WASM
//! extensions are *data behind* the `Wasm` lane, not new lanes — so the closed
//! set costs no real extensibility (§4.2). Introduced additively ahead of the
//! `RuntimeAdapter` `dyn`→enum migration (§9); nothing matches on it yet.

use serde::{Deserialize, Serialize};

/// Single source of truth for the lane set: the enum, [`RuntimeLane::ALL`],
/// and [`RuntimeLane::as_str`] are all generated from this one list, so a new
/// lane cannot be added to one surface while silently missing from another
/// (review findings on the C.6 slice — a hand-maintained `ALL` could drift).
macro_rules! runtime_lanes {
    ($( $(#[$meta:meta])* $variant:ident => $tag:literal ),+ $(,)?) => {
        /// The closed set of runtime execution lanes — the untrusted surfaces a
        /// capability can be dispatched to (§4.2/§5.5). `dispatch()` matches on
        /// this exhaustively; adding a variant is a deliberate, security-reviewed
        /// change that forces every match to confront it.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum RuntimeLane {
            $( $(#[$meta])* $variant, )+
        }

        impl RuntimeLane {
            /// Every lane, exactly once, in declaration order. Generated from the
            /// same list as the enum itself — completeness is structural, not
            /// hand-maintained.
            pub const ALL: [RuntimeLane; [$(runtime_lanes!(@unit $variant)),+].len()] =
                [$(RuntimeLane::$variant),+];

            /// Stable discriminant (matches the serde tag) for logs/routing.
            pub fn as_str(&self) -> &'static str {
                match self {
                    $( RuntimeLane::$variant => $tag, )+
                }
            }
        }
    };
    (@unit $variant:ident) => {
        ()
    };
}

runtime_lanes! {
    /// Host-coupled built-in capability code.
    FirstParty => "first_party",
    /// Sandboxed WASM extension code (the default extension lane; individual
    /// extensions are data behind this lane, not new lanes).
    Wasm => "wasm",
    /// An external MCP server integration.
    Mcp => "mcp",
    /// An OS process under the sandbox — the lane today's script/process adapter
    /// executes on. The host-only lane; §5.9 makes it structural.
    Process => "process",
}

impl RuntimeLane {
    /// Resolve the execution lane for a descriptor's [`crate::RuntimeKind`]
    /// (the loading taxonomy, §5.10). This is the one place the two axes meet:
    /// `authorize()` reads the descriptor's kind and binds the resolved lane into
    /// the `Authorized` witness so `dispatch()` routes without re-deriving it.
    ///
    /// - `Wasm`/`Mcp`/`FirstParty` map to their same-named lanes.
    /// - `Script` runs **on** the [`RuntimeLane::Process`] lane (§4.2: "today's
    ///   script/process adapter executes on the `Process` lane").
    /// - `System` is host-internal (master-key ops, migrations, admin tooling)
    ///   and is **not** an untrusted execution lane, so it maps to `None` — a
    ///   `System` capability is not dispatched to a `RuntimeLane` at all. Callers
    ///   must treat `None` as "host-internal, no untrusted lane", never as a
    ///   default.
    pub fn from_runtime_kind(kind: crate::RuntimeKind) -> Option<RuntimeLane> {
        use crate::RuntimeKind;
        match kind {
            RuntimeKind::Wasm => Some(RuntimeLane::Wasm),
            RuntimeKind::Mcp => Some(RuntimeLane::Mcp),
            RuntimeKind::Script => Some(RuntimeLane::Process),
            RuntimeKind::FirstParty => Some(RuntimeLane::FirstParty),
            RuntimeKind::System => None,
        }
    }
}

impl std::fmt::Display for RuntimeLane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_lane_serde_is_snake_case_and_roundtrips() {
        for lane in RuntimeLane::ALL {
            let json = serde_json::to_value(lane).unwrap();
            assert_eq!(json, serde_json::Value::String(lane.as_str().to_string()));
            let back: RuntimeLane = serde_json::from_value(json).unwrap();
            assert_eq!(back, lane);
        }
    }

    #[test]
    fn all_and_as_str_are_generated_from_one_source() {
        // Completeness is structural now (review findings on the C.6 slice):
        // the enum, ALL, and as_str() are all generated from the single
        // `runtime_lanes!` list, so a new lane cannot exist without appearing
        // in ALL — no hand-maintained inventory to drift. What remains testable
        // is the wire agreement: each macro-supplied tag must match the
        // serde(rename_all) output, and ALL must be duplicate-free.
        let mut seen = std::collections::BTreeSet::new();
        for lane in RuntimeLane::ALL {
            let wire = serde_json::to_value(lane).unwrap();
            assert_eq!(
                wire,
                serde_json::Value::String(lane.as_str().to_string()),
                "macro tag must match the serde rename_all tag for {lane:?}"
            );
            assert!(
                seen.insert(lane.as_str()),
                "duplicate lane in ALL: {lane:?}"
            );
        }
        assert_eq!(seen.len(), RuntimeLane::ALL.len());
    }

    #[test]
    fn tags_are_the_expected_wire_strings() {
        assert_eq!(RuntimeLane::FirstParty.as_str(), "first_party");
        assert_eq!(RuntimeLane::Wasm.as_str(), "wasm");
        assert_eq!(RuntimeLane::Mcp.as_str(), "mcp");
        assert_eq!(RuntimeLane::Process.as_str(), "process");
    }

    #[test]
    fn from_runtime_kind_maps_the_loading_taxonomy_to_lanes() {
        use crate::RuntimeKind;
        assert_eq!(
            RuntimeLane::from_runtime_kind(RuntimeKind::Wasm),
            Some(RuntimeLane::Wasm)
        );
        assert_eq!(
            RuntimeLane::from_runtime_kind(RuntimeKind::Mcp),
            Some(RuntimeLane::Mcp)
        );
        assert_eq!(
            RuntimeLane::from_runtime_kind(RuntimeKind::FirstParty),
            Some(RuntimeLane::FirstParty)
        );
        // Script executes on the Process lane (§4.2).
        assert_eq!(
            RuntimeLane::from_runtime_kind(RuntimeKind::Script),
            Some(RuntimeLane::Process)
        );
        // System is host-internal — no untrusted execution lane.
        assert_eq!(RuntimeLane::from_runtime_kind(RuntimeKind::System), None);
    }
}
