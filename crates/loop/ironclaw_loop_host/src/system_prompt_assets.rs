//! System-prompt content owned by the loop tier.
//!
//! These five assets are the *content* half of the default system prompt. They
//! live here — beside the other `prompts/*.md` assets this crate ships and
//! beside [`identity_context`](crate::identity_context), whose
//! `HostIdentityContextSource` is what puts them in front of a model — rather
//! than in the composition root, which owns *assembly*, not prompt text
//! (PROPOSAL §6.10.1 "system-prompt content → prompt assets in the loop/product
//! owner"; house rule "prompt templates live in files, not Rust code, inside
//! the crate that owns the behavior").
//!
//! Placement only. The seeding/validation of the user-editable `SYSTEM.md`
//! under the standalone storage root stays in the composition root: it is
//! boot-time `std::fs` work on a real host path, and this crate performs no
//! filesystem I/O of that kind.

/// The seed contents of the user-editable `SYSTEM.md` identity file.
///
/// Written once, on first boot, into the standalone storage root; from then on
/// the file is the user's. Everything below is appended *in memory* at resolve
/// time instead, so existing installs get it too.
pub const DEFAULT_SYSTEM_PROMPT: &str = include_str!("../prompts/default_system.md");

/// Progressive tool-disclosure protocol, appended to the system prompt only
/// when disclosure is active (bridged mode).
///
/// A weak model will not adopt the search/describe/call protocol from the
/// `tool_search` tool description alone — it needs an explicit, imperative
/// system-prompt instruction telling it that its visible tools are a subset and
/// to search before concluding a capability is missing. This text references
/// the bridge tools, so it must NOT appear when disclosure is off (no bridges
/// exist on that surface).
pub const TOOL_DISCLOSURE_PROTOCOL_PROMPT: &str =
    include_str!("../prompts/tool_disclosure_protocol.md");

/// Docs-grounding self-knowledge protocol, appended to the system prompt
/// unconditionally.
///
/// This is ground knowledge about the running system, not a user preference:
/// without it the model answers questions about IronClaw's own capabilities
/// from training data instead of the published docs. Seeding it into the
/// user-editable file would only reach fresh installs, so it is appended in
/// memory on every resolve — the same mechanism the tool-disclosure protocol
/// uses.
pub const SELF_KNOWLEDGE_PROTOCOL_PROMPT: &str = include_str!("../prompts/self_knowledge.md");

/// Persistent-memory protocol, appended to the system prompt unconditionally.
///
/// Nothing otherwise tells the model *when* a durable user fact is worth
/// saving, that surfaced memories came from earlier conversations, or how to
/// phrase one so it does not read as a standing instruction later (#7185).
/// That is ground knowledge about the running system rather than a user
/// preference, so — like the self-knowledge section — seeding it into the
/// user-editable file would only reach fresh installs; it is appended in memory
/// on every resolve instead.
pub const MEMORY_PROTOCOL_PROMPT: &str = include_str!("../prompts/memory_protocol.md");

/// Appended only when benchmarking mode is active.
///
/// Tells the model there is no human to ask, overriding the "ask the user...a
/// product decision" escape valve in the base prompt's Tool Continuation
/// section — that escape valve is correct for real product usage but causes an
/// agent running unattended dataset evaluation to stall a turn on a clarifying
/// question no one will ever answer.
pub const BENCHMARKING_MODE_PROTOCOL_PROMPT: &str = include_str!("../prompts/benchmarking_mode.md");

#[cfg(test)]
mod tests {
    use super::*;

    /// No asset may be empty — an empty `include_str!` target is how a
    /// mis-pointed path shows up, and it would silently drop a whole section
    /// of the system prompt rather than fail.
    #[test]
    fn every_system_prompt_asset_is_non_empty() {
        for (name, content) in [
            ("default_system.md", DEFAULT_SYSTEM_PROMPT),
            (
                "tool_disclosure_protocol.md",
                TOOL_DISCLOSURE_PROTOCOL_PROMPT,
            ),
            ("self_knowledge.md", SELF_KNOWLEDGE_PROTOCOL_PROMPT),
            ("memory_protocol.md", MEMORY_PROTOCOL_PROMPT),
            ("benchmarking_mode.md", BENCHMARKING_MODE_PROTOCOL_PROMPT),
        ] {
            assert!(!content.trim().is_empty(), "{name} must not be empty");
        }
    }

    /// The four *appended* protocols are concatenated after the user's file,
    /// separated by a blank line; each must open with a markdown heading so it
    /// reads as its own section rather than running into the previous
    /// paragraph. The base prompt is a whole document and is exempt.
    #[test]
    fn appended_protocol_assets_open_their_own_section() {
        for (name, content) in [
            (
                "tool_disclosure_protocol.md",
                TOOL_DISCLOSURE_PROTOCOL_PROMPT,
            ),
            ("self_knowledge.md", SELF_KNOWLEDGE_PROTOCOL_PROMPT),
            ("memory_protocol.md", MEMORY_PROTOCOL_PROMPT),
            ("benchmarking_mode.md", BENCHMARKING_MODE_PROTOCOL_PROMPT),
        ] {
            assert!(
                content.starts_with('#'),
                "{name} is appended to the system prompt and must open with a \
                 markdown heading; starts with {:?}",
                content.chars().take(16).collect::<String>()
            );
        }
    }

    /// The memory protocol is the only thing that tells the model *when* to
    /// save a durable user fact and *what* the automatically surfaced memory
    /// block is (#7185). Pin the load-bearing parts: the exact tool ids it
    /// names (a renamed tool would leave the instruction pointing at nothing),
    /// the curated `memory` target plus append mode the always-on prompt lane
    /// reads back, and the never-save carve-out for secrets.
    #[test]
    fn memory_protocol_names_the_save_path_and_its_limits() {
        for expected in [
            "ironclaw.memory.write",
            "ironclaw.memory.search",
            "`memory`",
            "append: true",
            "across conversations",
            "Never save secrets",
        ] {
            assert!(
                MEMORY_PROTOCOL_PROMPT.contains(expected),
                "memory protocol must mention {expected:?}"
            );
        }
    }

    /// Field-proven doctrine the protocol has to carry, each pinned because
    /// dropping it degrades recall quality in a way no compiler catches:
    ///
    /// - **Declarative form.** A memory saved as an imperative ("Always respond
    ///   concisely") is re-read as a standing directive on every later turn —
    ///   and the always-on lane re-injects it every turn — so it can override
    ///   what the user is asking for now. The worked example pair is the part
    ///   models actually copy, so pin both halves.
    /// - **Staleness skip-list.** Task progress, session outcomes, and
    ///   short-lived artifacts (PR numbers, commit SHAs) crowd out durable
    ///   facts and go wrong within days.
    /// - **Priority framing.** Tells the model which memory is worth the write
    ///   when it has to choose.
    #[test]
    fn memory_protocol_carries_the_write_quality_doctrine() {
        for (doctrine, expected) in [
            ("declarative-form rule", "declarative fact"),
            (
                "declarative example (good)",
                "User prefers concise responses",
            ),
            ("declarative example (bad)", "Always respond concisely"),
            ("staleness skip-list", "task progress"),
            ("staleness skip-list", "commit SHAs"),
            ("staleness horizon", "stale within a week"),
            ("priority framing", "repeat or correct themselves"),
        ] {
            assert!(
                MEMORY_PROTOCOL_PROMPT.contains(expected),
                "memory protocol lost its {doctrine}: expected {expected:?}"
            );
        }
    }

    /// The protocol must stay short enough to be worth appending to every
    /// prompt on every turn. Guidance that grows unchecked is how a system
    /// prompt quietly becomes the dominant cost of a cheap turn.
    #[test]
    fn memory_protocol_stays_within_its_prompt_budget() {
        let lines = MEMORY_PROTOCOL_PROMPT.lines().count();
        assert!(
            lines <= 18,
            "memory protocol is appended to every turn's prompt and must stay compact; \
             {lines} lines"
        );
    }

    /// The four appended protocols are distinct sections — a copy/paste that
    /// duplicated one would silently double a section in the resolved prompt.
    #[test]
    fn appended_protocol_assets_are_distinct() {
        let appended = [
            TOOL_DISCLOSURE_PROTOCOL_PROMPT,
            SELF_KNOWLEDGE_PROTOCOL_PROMPT,
            MEMORY_PROTOCOL_PROMPT,
            BENCHMARKING_MODE_PROTOCOL_PROMPT,
        ];
        for (i, left) in appended.iter().enumerate() {
            for right in appended.iter().skip(i + 1) {
                assert_ne!(left, right, "appended protocol assets must be distinct");
            }
        }
    }
}
