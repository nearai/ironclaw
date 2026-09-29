//! System-prompt content owned by the loop tier.
//!
//! These six assets are the *content* half of the default system prompt. They
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
//!
//! Which tool-naming sections one run gets is loop-tier policy, so it lives
//! here too ([`tool_naming_sections`]): a section that names a tool renders
//! only when the run advertises that tool (#7836). The composition root only
//! concatenates what it returns.

use ironclaw_host_api::capability::{
    EXTENSION_SEARCH_CAPABILITY_ID, TOOL_CALL_CAPABILITY_ID, TOOL_DESCRIBE_CAPABILITY_ID,
    TOOL_SEARCH_CAPABILITY_ID,
};
use ironclaw_loop_contracts::AdvertisedTools;

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
///
/// The extension-lifecycle paragraphs that follow it on the ordinary surface
/// are separate assets ([`EXTENSION_LIFECYCLE_PROTOCOL_PROMPT`],
/// [`HOSTED_MCP_REGISTRATION_PROTOCOL_PROMPT`]) so a turn-start selection that
/// leaves those tools out can leave the paragraphs out too. Appended as
/// sections they reproduce the original text byte for byte.
pub const TOOL_DISCLOSURE_PROTOCOL_PROMPT: &str =
    include_str!("../prompts/tool_disclosure_protocol.md");

/// The extension-install paragraph of the tool-discovery section.
pub const EXTENSION_LIFECYCLE_PROTOCOL_PROMPT: &str =
    include_str!("../prompts/extension_lifecycle_protocol.md");

/// The custom-hosted-MCP paragraph of the tool-discovery section.
pub const HOSTED_MCP_REGISTRATION_PROTOCOL_PROMPT: &str =
    include_str!("../prompts/hosted_mcp_registration_protocol.md");

/// Capabilities [`TOOL_DISCLOSURE_PROTOCOL_PROMPT`] names: the three bridges.
const DISCLOSURE_PROTOCOL_TOOLS: [&str; 3] = [
    TOOL_SEARCH_CAPABILITY_ID,
    TOOL_DESCRIBE_CAPABILITY_ID,
    TOOL_CALL_CAPABILITY_ID,
];

/// Capabilities [`EXTENSION_LIFECYCLE_PROTOCOL_PROMPT`] names. The install id
/// is owned by `ironclaw_extension_manager`, which this crate does not
/// depend on; the composition root's tests pin it against that owner.
pub const EXTENSION_LIFECYCLE_PROTOCOL_TOOLS: [&str; 2] =
    [EXTENSION_SEARCH_CAPABILITY_ID, "builtin.extension_install"];

/// Capabilities [`HOSTED_MCP_REGISTRATION_PROTOCOL_PROMPT`] names; pinned
/// against their owner like [`EXTENSION_LIFECYCLE_PROTOCOL_TOOLS`].
pub const HOSTED_MCP_REGISTRATION_PROTOCOL_TOOLS: [&str; 3] = [
    EXTENSION_SEARCH_CAPABILITY_ID,
    "builtin.extension_install",
    "builtin.extension_register_hosted_mcp",
];

/// Capabilities [`TOOL_PREFETCH_PROTOCOL_PROMPT`] names.
const TOOL_PREFETCH_PROTOCOL_TOOLS: [&str; 2] =
    [TOOL_SEARCH_CAPABILITY_ID, TOOL_CALL_CAPABILITY_ID];

/// Prefix of the bound memory provider's capability ids. Provider guidance
/// is opaque text, so the tools it names are found by this prefix.
const MEMORY_CAPABILITY_PREFIX: &str = "ironclaw.memory.";

/// Turn-start tool selection protocol, appended after the tool-discovery
/// section only on a run that advertises a turn-start selection (see
/// [`tool_protocol_sections`]).
///
/// The advertised tools were chosen for the conversation's opening request
/// and stay fixed for the conversation, so the model is told why a tool it
/// needs may be missing, to reach it through `tool_search` → `tool_call`, and
/// to suggest a new conversation (which selects afresh) when the task has
/// changed. Every tool it names is in the always-on selection floor.
pub const TOOL_PREFETCH_PROTOCOL_PROMPT: &str =
    include_str!("../prompts/tool_prefetch_protocol.md");

/// The tool-naming sections one run's system prompt appends after the
/// self-knowledge section, in order: the bound memory provider's `guidance`
/// (see [`memory_guidance_for_run`]), then the tool-protocol sections (see
/// [`tool_protocol_sections`]).
pub fn tool_naming_sections<'a>(
    memory_guidance: Option<&'a str>,
    disclosure: bool,
    tool_prefetch: bool,
    advertised_tools: &AdvertisedTools,
) -> Vec<&'a str> {
    memory_guidance
        .and_then(|guidance| memory_guidance_for_run(guidance, advertised_tools))
        .into_iter()
        .chain(tool_protocol_sections(
            disclosure,
            tool_prefetch,
            advertised_tools,
        ))
        .collect()
}

/// The tool-protocol sections one run's system prompt appends, in order.
///
/// `disclosure` and `tool_prefetch` say whether the runtime has progressive
/// disclosure and turn-start selection on. On an ordinary surface this is
/// what the prompt always carried: with disclosure on, the discovery section
/// and both extension-lifecycle paragraphs. The selection section appears
/// only on a run that advertises a selection, never on one where selection
/// fell back to the ordinary surface. On a selected run every section
/// renders only when the selection advertises the tools it names, and the
/// sections that build on the discovery section (the lifecycle paragraphs
/// and the selection section) need it to render too.
fn tool_protocol_sections(
    disclosure: bool,
    tool_prefetch: bool,
    advertised_tools: &AdvertisedTools,
) -> Vec<&'static str> {
    let mut sections = Vec::new();
    if !disclosure || !advertised_tools.may_name(&DISCLOSURE_PROTOCOL_TOOLS) {
        return sections;
    }
    sections.push(TOOL_DISCLOSURE_PROTOCOL_PROMPT);
    if advertised_tools.may_name(&EXTENSION_LIFECYCLE_PROTOCOL_TOOLS) {
        sections.push(EXTENSION_LIFECYCLE_PROTOCOL_PROMPT);
    }
    if advertised_tools.may_name(&HOSTED_MCP_REGISTRATION_PROTOCOL_TOOLS) {
        sections.push(HOSTED_MCP_REGISTRATION_PROTOCOL_PROMPT);
    }
    if tool_prefetch
        && advertised_tools.is_selected()
        && advertised_tools.may_name(&TOOL_PREFETCH_PROTOCOL_TOOLS)
    {
        sections.push(TOOL_PREFETCH_PROTOCOL_PROMPT);
    }
    sections
}

/// The bound memory provider's guidance for one run: `guidance` itself when
/// the run advertises every `ironclaw.memory.*` capability it names, and
/// `None` otherwise. An ordinary surface always keeps it.
fn memory_guidance_for_run<'a>(
    guidance: &'a str,
    advertised_tools: &AdvertisedTools,
) -> Option<&'a str> {
    let named = memory_capabilities_named_in(guidance);
    let named: Vec<&str> = named.iter().map(String::as_str).collect();
    advertised_tools.may_name(&named).then_some(guidance)
}

/// Every `ironclaw.memory.<name>` capability id `text` mentions.
fn memory_capabilities_named_in(text: &str) -> Vec<String> {
    let mut named: Vec<String> = Vec::new();
    for (start, _) in text.match_indices(MEMORY_CAPABILITY_PREFIX) {
        let rest = &text[start + MEMORY_CAPABILITY_PREFIX.len()..];
        let name_len = rest
            .char_indices()
            .find(|(_, character)| !(character.is_ascii_alphanumeric() || *character == '_'))
            .map_or(rest.len(), |(index, _)| index);
        if name_len == 0 {
            continue;
        }
        let capability_id = format!("{MEMORY_CAPABILITY_PREFIX}{}", &rest[..name_len]);
        if !named.contains(&capability_id) {
            named.push(capability_id);
        }
    }
    named
}

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

/// Appended only when benchmarking mode is active.
///
/// Tells the model there is no human to ask, overriding the "ask the user...a
/// product decision" escape valve in the base prompt's Tool Continuation
/// section — that escape valve is correct for real product usage but causes an
/// agent running unattended dataset evaluation to stall a turn on a clarifying
/// question no one will ever answer.
pub const BENCHMARKING_MODE_PROTOCOL_PROMPT: &str = include_str!("../prompts/benchmarking_mode.md");

/// Appended only to runs with trusted scheduled-trigger origin.
///
/// Scheduled runs have no human available to answer the base prompt's
/// clarifying questions. This protocol tells the model to execute the stored
/// request with bounded assumptions while preserving host approval,
/// authentication, authorization, and policy gates.
pub const SCHEDULED_TRIGGER_MODE_PROTOCOL_PROMPT: &str =
    include_str!("../prompts/scheduled_trigger_mode.md");

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
            ("tool_prefetch_protocol.md", TOOL_PREFETCH_PROTOCOL_PROMPT),
            (
                "extension_lifecycle_protocol.md",
                EXTENSION_LIFECYCLE_PROTOCOL_PROMPT,
            ),
            (
                "hosted_mcp_registration_protocol.md",
                HOSTED_MCP_REGISTRATION_PROTOCOL_PROMPT,
            ),
            ("self_knowledge.md", SELF_KNOWLEDGE_PROTOCOL_PROMPT),
            ("benchmarking_mode.md", BENCHMARKING_MODE_PROTOCOL_PROMPT),
            (
                "scheduled_trigger_mode.md",
                SCHEDULED_TRIGGER_MODE_PROTOCOL_PROMPT,
            ),
        ] {
            assert!(!content.trim().is_empty(), "{name} must not be empty");
        }
    }

    /// The five *appended* protocols are concatenated after the user's file,
    /// separated by a blank line; each must open with a markdown heading so it
    /// reads as its own section rather than running into the previous
    /// paragraph. The base prompt is a whole document and is exempt, and so
    /// are the extension-lifecycle paragraphs, which continue the discovery
    /// section.
    #[test]
    fn appended_protocol_assets_open_their_own_section() {
        for (name, content) in [
            (
                "tool_disclosure_protocol.md",
                TOOL_DISCLOSURE_PROTOCOL_PROMPT,
            ),
            ("tool_prefetch_protocol.md", TOOL_PREFETCH_PROTOCOL_PROMPT),
            ("self_knowledge.md", SELF_KNOWLEDGE_PROTOCOL_PROMPT),
            ("benchmarking_mode.md", BENCHMARKING_MODE_PROTOCOL_PROMPT),
            (
                "scheduled_trigger_mode.md",
                SCHEDULED_TRIGGER_MODE_PROTOCOL_PROMPT,
            ),
        ] {
            assert!(
                content.starts_with('#'),
                "{name} is appended to the system prompt and must open with a \
                 markdown heading; starts with {:?}",
                content.chars().take(16).collect::<String>()
            );
        }
    }

    /// The five appended protocols are distinct sections — a copy/paste that
    /// duplicated one would silently double a section in the resolved prompt.
    #[test]
    fn appended_protocol_assets_are_distinct() {
        let appended = [
            TOOL_DISCLOSURE_PROTOCOL_PROMPT,
            EXTENSION_LIFECYCLE_PROTOCOL_PROMPT,
            HOSTED_MCP_REGISTRATION_PROTOCOL_PROMPT,
            TOOL_PREFETCH_PROTOCOL_PROMPT,
            SELF_KNOWLEDGE_PROTOCOL_PROMPT,
            BENCHMARKING_MODE_PROTOCOL_PROMPT,
            SCHEDULED_TRIGGER_MODE_PROTOCOL_PROMPT,
        ];
        for (i, left) in appended.iter().enumerate() {
            for right in appended.iter().skip(i + 1) {
                assert_ne!(left, right, "appended protocol assets must be distinct");
            }
        }
    }

    #[test]
    fn tool_disclosure_protocol_makes_describe_conditional() {
        assert!(TOOL_DISCLOSURE_PROTOCOL_PROMPT.contains("schema_complete=true"));
        assert!(TOOL_DISCLOSURE_PROTOCOL_PROMPT.contains("schema_complete=false"));
        assert!(TOOL_DISCLOSURE_PROTOCOL_PROMPT.contains("marker is absent"));
        assert!(
            TOOL_DISCLOSURE_PROTOCOL_PROMPT.contains("invoke it directly"),
            "a complete search signature must remove the describe round trip"
        );
    }

    #[test]
    fn scheduled_trigger_protocol_bounds_investigation_without_weakening_explicit_completeness() {
        assert!(
            SCHEDULED_TRIGGER_MODE_PROTOCOL_PROMPT
                .contains("Unless the stored request explicitly requires exhaustive coverage")
        );
        assert!(
            SCHEDULED_TRIGGER_MODE_PROTOCOL_PROMPT
                .contains("do not enumerate an entire large collection")
        );
        assert!(
            SCHEDULED_TRIGGER_MODE_PROTOCOL_PROMPT
                .contains("perform the requested action before further investigation")
        );
        assert!(
            SCHEDULED_TRIGGER_MODE_PROTOCOL_PROMPT.contains("never claim the side effect happened")
        );
    }

    fn selected(ids: &[&str]) -> AdvertisedTools {
        AdvertisedTools::Selected(
            ids.iter()
                .map(|id| {
                    ironclaw_host_api::ids::CapabilityId::new(*id).expect("valid capability id")
                })
                .collect(),
        )
    }

    const BRIDGES: [&str; 3] = [
        "ironclaw.tool_search",
        "ironclaw.tool_describe",
        "ironclaw.tool_call",
    ];

    /// Every tool a section's text names is covered by the gate it renders
    /// under (its own ids, plus the discovery section's for the sections that
    /// only render after it), and every id in its own gate is one it names,
    /// so the gates cannot drift from the text.
    #[test]
    fn each_gated_section_names_only_the_tools_it_is_gated_on() {
        let short = |id: &&str| id.rsplit('.').next().unwrap_or(id).to_string();
        for (section, own_gate, renders_after_discovery) in [
            (
                TOOL_DISCLOSURE_PROTOCOL_PROMPT,
                DISCLOSURE_PROTOCOL_TOOLS.as_slice(),
                false,
            ),
            (
                EXTENSION_LIFECYCLE_PROTOCOL_PROMPT,
                EXTENSION_LIFECYCLE_PROTOCOL_TOOLS.as_slice(),
                true,
            ),
            (
                HOSTED_MCP_REGISTRATION_PROTOCOL_PROMPT,
                HOSTED_MCP_REGISTRATION_PROTOCOL_TOOLS.as_slice(),
                true,
            ),
            (
                TOOL_PREFETCH_PROTOCOL_PROMPT,
                TOOL_PREFETCH_PROTOCOL_TOOLS.as_slice(),
                true,
            ),
        ] {
            let named: Vec<&str> = [
                "tool_search",
                "tool_describe",
                "tool_call",
                "extension_search",
                "extension_install",
                "extension_register_hosted_mcp",
            ]
            .into_iter()
            .filter(|name| {
                let quoted = format!("`{name}");
                section.match_indices(&quoted).any(|(at, _)| {
                    section[at + quoted.len()..]
                        .chars()
                        .next()
                        .is_none_or(|next| !(next.is_ascii_alphanumeric() || next == '_'))
                })
            })
            .collect();
            let own: Vec<String> = own_gate.iter().map(short).collect();
            let mut effective = own.clone();
            if renders_after_discovery {
                effective.extend(DISCLOSURE_PROTOCOL_TOOLS.iter().map(short));
            }
            for name in &named {
                assert!(
                    effective.iter().any(|gated| gated == name),
                    "`{name}` is named but not gated on: {section}"
                );
            }
            for gated in &own {
                assert!(
                    named.contains(&gated.as_str()),
                    "`{gated}` is gated on but not named: {section}"
                );
            }
        }
    }

    #[test]
    fn an_ordinary_surface_keeps_every_section_it_always_had() {
        let ordinary = AdvertisedTools::Ordinary;
        assert!(tool_protocol_sections(false, false, &ordinary).is_empty());
        assert_eq!(
            tool_protocol_sections(true, false, &ordinary),
            [
                TOOL_DISCLOSURE_PROTOCOL_PROMPT,
                EXTENSION_LIFECYCLE_PROTOCOL_PROMPT,
                HOSTED_MCP_REGISTRATION_PROTOCOL_PROMPT,
            ]
        );
        assert_eq!(
            tool_protocol_sections(true, true, &ordinary),
            tool_protocol_sections(true, false, &ordinary),
            "a run the selection fell back on gets no selection section"
        );
        assert_eq!(
            tool_naming_sections(Some("guidance"), true, true, &ordinary),
            [
                "guidance",
                TOOL_DISCLOSURE_PROTOCOL_PROMPT,
                EXTENSION_LIFECYCLE_PROTOCOL_PROMPT,
                HOSTED_MCP_REGISTRATION_PROTOCOL_PROMPT,
            ],
            "the memory guidance comes first, as it always has"
        );
    }

    #[test]
    fn a_selected_surface_keeps_only_sections_whose_tools_it_advertises() {
        let bridges_only = selected(&BRIDGES);
        assert_eq!(
            tool_protocol_sections(true, true, &bridges_only),
            [
                TOOL_DISCLOSURE_PROTOCOL_PROMPT,
                TOOL_PREFETCH_PROTOCOL_PROMPT
            ]
        );

        let mut with_install: Vec<&str> = BRIDGES.to_vec();
        with_install.extend(EXTENSION_LIFECYCLE_PROTOCOL_TOOLS);
        assert_eq!(
            tool_protocol_sections(true, true, &selected(&with_install)),
            [
                TOOL_DISCLOSURE_PROTOCOL_PROMPT,
                EXTENSION_LIFECYCLE_PROTOCOL_PROMPT,
                TOOL_PREFETCH_PROTOCOL_PROMPT,
            ]
        );

        let mut everything: Vec<&str> = BRIDGES.to_vec();
        everything.extend(HOSTED_MCP_REGISTRATION_PROTOCOL_TOOLS);
        assert_eq!(
            tool_protocol_sections(true, true, &selected(&everything)),
            [
                TOOL_DISCLOSURE_PROTOCOL_PROMPT,
                EXTENSION_LIFECYCLE_PROTOCOL_PROMPT,
                HOSTED_MCP_REGISTRATION_PROTOCOL_PROMPT,
                TOOL_PREFETCH_PROTOCOL_PROMPT,
            ]
        );

        assert!(
            tool_protocol_sections(true, true, &selected(&["ironclaw.tool_search"])).is_empty(),
            "without every bridge the discovery section, and all that builds on it, is withheld"
        );
    }

    #[test]
    fn memory_guidance_renders_only_when_the_tools_it_names_are_advertised() {
        let guidance = "Call `ironclaw.memory.search` first; save with `ironclaw.memory.write`.";
        assert_eq!(
            memory_capabilities_named_in(guidance),
            ["ironclaw.memory.search", "ironclaw.memory.write"]
        );
        assert_eq!(
            memory_guidance_for_run(guidance, &AdvertisedTools::Ordinary),
            Some(guidance)
        );
        assert_eq!(
            memory_guidance_for_run(
                guidance,
                &selected(&["ironclaw.memory.search", "ironclaw.memory.write"])
            ),
            Some(guidance)
        );
        assert_eq!(
            memory_guidance_for_run(guidance, &selected(&["ironclaw.memory.search"])),
            None
        );
        assert_eq!(
            memory_guidance_for_run("Remember things.", &selected(&[])),
            Some("Remember things."),
            "guidance naming no memory tool always renders"
        );
    }
}
