# Ironclaw Architecture Resources

## Knowledge

- [Crates architecture map](crates/Architecture.md)
  Best starting point for the Reborn mental model, crate ownership, turn data flow, capability flow, subagents, events, and change recipes. Use for: every architecture lesson.
- [Kernel boundary contract](docs/reborn/contracts/kernel-boundary.md)
  Defines the kernel as the security perimeter and separates kernel responsibilities from userland loop behavior. Use for: permissions, trust, authority, and "is this a kernel bug?" questions.
- [Runtime workflow ownership contract](docs/reborn/contracts/runtime-workflows.md)
  Shows common workflows such as interactive chat turns, approval-blocked capabilities, auth-blocked capabilities, jobs, subagents, and transport ingress. Use for: scenario walkthroughs.
- [Turn runner contract](docs/reborn/contracts/turn-runner.md)
  Describes queued runs, runner claims, leases, heartbeats, blocked states, completion, and recovery. Use for: run lifecycle and crash/retry safety.
- [Agent loop protocol contract](docs/reborn/contracts/agent-loop-protocol.md)
  Defines the parent loop envelope as Reply or CapabilityCalls. Use for: model outputs, tool calls, and why side effects cannot be prose.
- [Capability host contract](docs/reborn/contracts/capabilities.md)
  Explains capability invocation, approval/auth resume, obligations, and failure taxonomy. Use for: tool execution and side-effect mediation.
- [Capability access contract](docs/reborn/contracts/capability-access.md)
  Defines grants, leases, default-deny authorization, and the distinction between visibility and authority. Use for: permissions and approval leases.
- [Approval resolution contract](docs/reborn/contracts/approvals.md)
  Explains durable approval records, invocation fingerprints, and bounded leases. Use for: human approval flows.
- [Trigger system contract](docs/reborn/contracts/triggers.md)
  Defines scheduled trigger intake and how due fires become synthetic inbound turns. Use for: routines and automations in the modern Reborn model.
- [Host runtime contract](docs/reborn/contracts/host-runtime.md)
  Describes host-mediated capabilities, obligations, first-party tools, runtime HTTP egress, secrets, network, process, and sandbox handoffs. Use for: runtime effects and security invariants.
- [ironclaw_agent_loop guardrails](crates/ironclaw_agent_loop/CLAUDE.md)
  Crate-local ownership rules for the canonical executor, loop families, strategy state, and loop boundaries. Use for: loop internals.
- [ironclaw_turns guardrails](crates/ironclaw_turns/CLAUDE.md)
  Crate-local rules for turn coordination, active locks, runner APIs, and redacted state. Use for: turn/run lifecycle changes.
- [ironclaw_host_runtime guardrails](crates/ironclaw_host_runtime/CLAUDE.md)
  Crate-local rules for host runtime services, egress, secrets, first-party tools, and production wiring. Use for: runtime-side authority work.
- [ironclaw_triggers agent map](crates/ironclaw_triggers/AGENTS.md)
  Crate-local rules for triggers, schedule validation, deterministic fire identity, and repository contracts. Use for: routine/automation changes.

## Wisdom (Communities)

- Ironclaw pull request review and contract docs
  Use for: testing architectural interpretations against maintainers and concrete diffs.

## Gaps

- Ask Ben which team channels, issue trackers, or design threads contain the highest-signal Ironclaw architecture discussion, then add them here.
