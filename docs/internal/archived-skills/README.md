# Archived runtime skills

These bundles were removed from the shipping `skills/` catalog on 2026-08-13. They are preserved
here because their advertised workflows cannot complete on the default Reborn model-tool surface.
Keeping them outside `skills/` prevents automatic activation from teaching an agent unavailable or
incompatible calls.

This is a parity quarantine, not a declaration that every tool name in these files is obsolete.
Bare `memory_*`, `http`, `project_create`, and file-tool calls still resolve through compatibility
aliases. Current scheduled work uses `builtin__trigger_create` and `builtin__trigger_list`.

| Skills | Blocking gap |
| --- | --- |
| `ceo-setup`, `commitment-setup`, `content-creator-setup`, `trader-setup` | Setup requires legacy `mission_*` calls and teaches no current trigger translation; project/marker/widget assumptions also drifted. |
| `code-review` | Its mandatory `github` skill dependency is quarantined, and its PR path requires the retired CodeAct/Monty execution model. |
| `developer-setup`, `github-workflow`, `project-setup` | Core workflows require event cadences and `event_emit`; current model-created triggers support cron/once schedules. |
| `github` | Raw HTTP credential-injection instructions were superseded by the typed GitHub extension and its lifecycle/auth contract. |
| `linear` | No bundled Linear extension currently supplies the authenticated tool surface promised by the skill. |
| `llm-council` | `llm_query` and `llm_query_batched` are not Reborn capabilities. |
| `local-test`, `web-ui-test` | Instructions depend on removed startup/UI conventions, a missing `Dockerfile.test`, and unavailable Claude-in-Chrome tools. |
| `new-project` | Memory writes do not create first-class projects; project creation now uses `builtin__project_create`, and old project-scoped mission arguments are rejected. |
| `parallel-pr-review` | `builtin.spawn_subagent` is disabled on the default production model surface. |
| `plan-mode` | `plan_update`, `mission_fire`, and the legacy mission/checklist protocol are absent. |
| `portfolio` | The portfolio capability and widget loader are absent; `requires.tools` is not a supported skill gate. Its former E2E scenario is preserved under `legacy-tests/`. |

## Restoration bar

Move a bundle back to `skills/` only after all of the following are true:

1. Every required capability is registered, authorized, and model-visible in the default production
   profile, or the skill has been rewritten to the current surface.
2. Tool names and schemas match the disclosed provider definitions. Scheduling uses the current
   trigger schema and makes outbound delivery explicit.
3. A fresh-agent test completes the representative workflow without relying on undocumented name
   translation or operator intervention.
4. Bundling, catalog lint, routing-corpus, and the applicable caller-path integration/E2E tests pass.
