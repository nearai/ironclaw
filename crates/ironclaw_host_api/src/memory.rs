//! Memory-surface declaration vocabulary (`[memory]` in a v3 manifest).
//!
//! An extension declares `[memory]` to say "I am a backend for the host's
//! memory adapter." The bound provider's manifest is the single source of
//! truth for its memory surface: the manifest's `[[tools]]` array declares the
//! model-visible memory tools the provider serves, and `[memory].lifecycle`
//! declares which host-initiated lifecycle hooks the host may call on it. A
//! hook that is not declared is never called. Compose-time binding selects
//! exactly one memory provider (native by default), so the model's memory
//! interface stays stable while the backend swaps underneath — it is never
//! installed/removed or swapped at runtime. The concrete provider is the
//! manifest's `[runtime].service`; no connection or credential material lives
//! here (that is compose-time configuration).

use serde::{Deserialize, Serialize};

/// Reserved capability-id namespace for the always-on memory adapter tools.
///
/// A `[memory]`-declaring manifest may declare its tools under
/// `ironclaw.memory.*` even when its own extension id differs, so swapping the
/// bound backend never renames the model's memory tools. Trust-safe: `[memory]`
/// requires a first_party runtime, which requires a host-bundled manifest
/// source, so no installable extension can squat the namespace.
pub const MEMORY_TOOL_ID_NAMESPACE: &str = "ironclaw.memory";

/// A host-initiated memory lifecycle hook a provider participates in.
///
/// Declared in `[memory].lifecycle`. The host calls only declared hooks:
///
/// - `read_long_term` / `read_short_term` — the two retrieve-before-run
///   context lanes (general durable memory vs. the active thread's scratch),
///   each queried independently once per run.
/// - `record_interaction` — the after-turn transcript record seam, called
///   after every `Completed` run.
/// - `profile_read` — the loop-start user-profile document read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryLifecycleHook {
    /// Retrieve-before-run: the user's general / durable memory lane.
    ReadLongTerm,
    /// Retrieve-before-run: the active thread's short-term scratch lane.
    ReadShortTerm,
    /// After-turn interaction recording.
    RecordInteraction,
    /// Loop-start user-profile document read.
    ProfileRead,
}

impl MemoryLifecycleHook {
    /// Stable wire token.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReadLongTerm => "read_long_term",
            Self::ReadShortTerm => "read_short_term",
            Self::RecordInteraction => "record_interaction",
            Self::ProfileRead => "profile_read",
        }
    }

    /// Every hook in the vocabulary, in wire order.
    pub const ALL: [MemoryLifecycleHook; 4] = [
        Self::ReadLongTerm,
        Self::ReadShortTerm,
        Self::RecordInteraction,
        Self::ProfileRead,
    ];
}

/// The `[memory]` surface of a v3 manifest: the extension is a memory provider
/// (a backend for the host memory adapter) and declares the host-initiated
/// lifecycle hooks it participates in. `lifecycle` may be empty or absent — a
/// provider that declares `[memory]` with no lifecycle contributes its
/// declared tools only and is never called on any host-initiated hook.
/// Parsing/validation is fail-closed: unknown fields and unknown lifecycle
/// tokens are rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryDescriptor {
    /// Host-initiated lifecycle hooks this provider participates in. Any
    /// subset, including empty. An undeclared hook is NEVER called.
    #[serde(default)]
    pub lifecycle: Vec<MemoryLifecycleHook>,
}

impl MemoryDescriptor {
    /// Whether this provider declares the given lifecycle hook.
    pub fn declares(&self, hook: MemoryLifecycleHook) -> bool {
        self.lifecycle.contains(&hook)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire tokens are the contract: manifests spell hooks exactly this
    /// way, and `as_str` must match the serde snake_case output.
    #[test]
    fn lifecycle_hook_wire_tokens_are_stable() {
        for (hook, token) in [
            (MemoryLifecycleHook::ReadLongTerm, "read_long_term"),
            (MemoryLifecycleHook::ReadShortTerm, "read_short_term"),
            (MemoryLifecycleHook::RecordInteraction, "record_interaction"),
            (MemoryLifecycleHook::ProfileRead, "profile_read"),
        ] {
            assert_eq!(hook.as_str(), token);
            let serialized = serde_json::to_string(&hook).expect("serialize hook");
            assert_eq!(serialized, format!("\"{token}\""));
            let back: MemoryLifecycleHook =
                serde_json::from_str(&serialized).expect("deserialize hook");
            assert_eq!(back, hook);
        }
    }

    #[test]
    fn unknown_lifecycle_token_fails_closed() {
        assert!(serde_json::from_str::<MemoryLifecycleHook>("\"on_boot\"").is_err());
    }

    #[test]
    fn descriptor_defaults_to_an_empty_lifecycle() {
        let descriptor: MemoryDescriptor = serde_json::from_str("{}").expect("empty descriptor");
        assert!(descriptor.lifecycle.is_empty());
        for hook in MemoryLifecycleHook::ALL {
            assert!(!descriptor.declares(hook));
        }
    }

    #[test]
    fn declares_reflects_the_declared_set() {
        let descriptor: MemoryDescriptor =
            serde_json::from_str(r#"{"lifecycle": ["read_long_term", "profile_read"]}"#)
                .expect("descriptor parses");
        assert!(descriptor.declares(MemoryLifecycleHook::ReadLongTerm));
        assert!(descriptor.declares(MemoryLifecycleHook::ProfileRead));
        assert!(!descriptor.declares(MemoryLifecycleHook::ReadShortTerm));
        assert!(!descriptor.declares(MemoryLifecycleHook::RecordInteraction));
    }

    #[test]
    fn descriptor_rejects_unknown_fields() {
        assert!(serde_json::from_str::<MemoryDescriptor>(r#"{"hooks": []}"#).is_err());
    }
}
