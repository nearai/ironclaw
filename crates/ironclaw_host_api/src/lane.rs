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

/// The closed set of runtime execution lanes — the untrusted surfaces a
/// capability can be dispatched to (§4.2/§5.5). `dispatch()` matches on this
/// exhaustively; adding a variant is a deliberate, security-reviewed change that
/// forces every match to confront it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeLane {
    /// Host-coupled built-in capability code.
    FirstParty,
    /// Sandboxed WASM extension code (the default extension lane; individual
    /// extensions are data behind this lane, not new lanes).
    Wasm,
    /// An external MCP server integration.
    Mcp,
    /// An OS process under the sandbox — the lane today's script/process adapter
    /// executes on. The host-only lane; §5.9 makes it structural.
    Process,
}

impl RuntimeLane {
    /// Stable discriminant (matches the serde tag) for logs/routing.
    pub fn as_str(&self) -> &'static str {
        match self {
            RuntimeLane::FirstParty => "first_party",
            RuntimeLane::Wasm => "wasm",
            RuntimeLane::Mcp => "mcp",
            RuntimeLane::Process => "process",
        }
    }

    /// Every lane, for exhaustiveness-style iteration in tests and registries.
    pub const ALL: [RuntimeLane; 4] = [
        RuntimeLane::FirstParty,
        RuntimeLane::Wasm,
        RuntimeLane::Mcp,
        RuntimeLane::Process,
    ];
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
    fn all_covers_every_variant_exactly_once() {
        // Review findings on the C.6 slice: the previous shape passed vacuously —
        // a new variant could be added to the enum, the match, and as_str() while
        // ALL kept the old four values, silently dropping the lane from every
        // registry that iterates ALL. This version is drift-proof in both
        // directions:
        //
        // - Adding a variant fails to COMPILE here (exhaustive match) until an
        //   ordinal is assigned;
        // - assigning an ordinal fails the `ordinal == index in ALL` assertion
        //   until ALL is extended with the new variant in position;
        // - the bijection check (every ordinal < len, each hit exactly once)
        //   rejects duplicates and omissions.
        fn ordinal(lane: RuntimeLane) -> usize {
            match lane {
                RuntimeLane::FirstParty => 0,
                RuntimeLane::Wasm => 1,
                RuntimeLane::Mcp => 2,
                RuntimeLane::Process => 3,
            }
        }
        let mut seen = [false; RuntimeLane::ALL.len()];
        for (index, lane) in RuntimeLane::ALL.into_iter().enumerate() {
            assert_eq!(
                ordinal(lane),
                index,
                "ALL must list every variant once, in ordinal order: {lane:?}"
            );
            assert!(!seen[index], "duplicate lane in ALL: {lane:?}");
            seen[index] = true;
        }
        assert!(seen.iter().all(|hit| *hit), "ALL must cover every ordinal");
    }

    #[test]
    fn tags_are_the_expected_wire_strings() {
        assert_eq!(RuntimeLane::FirstParty.as_str(), "first_party");
        assert_eq!(RuntimeLane::Wasm.as_str(), "wasm");
        assert_eq!(RuntimeLane::Mcp.as_str(), "mcp");
        assert_eq!(RuntimeLane::Process.as_str(), "process");
    }
}
