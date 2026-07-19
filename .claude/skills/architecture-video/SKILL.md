---
name: architecture-video
description: Use when updating the architecture video under docs/architecture-video to match the current Reborn runtime.
---

# Architecture Video

Treat the existing scenes as historical artwork, not architecture authority.
Regenerate every code path and label from the current Reborn workspace before
editing a scene.

1. Read root `AGENTS.md`, `crates/AGENTS.md`, and the Reborn orientation skill.
2. Run `bash scripts/codebase-graph.sh status`; use the graph when fresh, then
   verify symbols with targeted `rg`.
3. Read `docs/architecture-video/src/IronClawArchitecture.tsx` and the affected
   scenes.
4. Replace references to retired root `src/` or v1 crates with current owners;
   do not present deleted code as a live execution path.
5. Keep scene timing and visual conventions consistent, then run the video's
   package tests/build and inspect the rendered result.

The canonical runtime path is WebUI/product adapters -> product workflow ->
threads/turns -> runner/agent loop -> capability host -> mediated runtime lane.
