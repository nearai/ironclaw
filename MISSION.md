# Mission: Ironclaw Architecture Fluency

## Why
Ben wants to become confident enough to design, debug, review, and extend Ironclaw without accidentally crossing security, runtime, or ownership boundaries. The goal is practical architectural fluency: being able to predict where a prompt, tool call, routine, approval, or subagent run should flow before touching code.

## Success looks like
- Trace a user prompt through product ingress, turn coordination, loop execution, capability dispatch, events, and replies.
- Explain the kernel boundary, routines/triggers, permissions, approvals, auth gates, run state, and runtime lanes using Ironclaw's own vocabulary.
- Classify any new feature or bug as product, loop, kernel-boundary, substrate, or runtime-lane work.
- Identify the authoritative docs and crates to read before making changes in a subsystem.
- Choose narrow tests that cover behavior through the caller boundary, especially around approvals, permissions, routines, and side effects.

## Constraints
- Teaching should use diagrams, scenario walkthroughs, retrieval practice, and short lessons.
- Lessons should cite high-trust repo docs and source files rather than relying on memory.
- Keep the first lessons focused on Reborn/current architecture, while calling out legacy routine docs when they differ from the modern trigger model.

## Out of scope
- Exhaustive API documentation for every crate.
- Historical architecture archaeology unless it explains a current boundary.
- Production deployment operations beyond what is needed to understand runtime behavior.
