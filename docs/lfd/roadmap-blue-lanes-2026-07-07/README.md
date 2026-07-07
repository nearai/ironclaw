# IronClaw core roadmap LFD handoffs

Source: Notion database `NEAR AI Consolidated Roadmap`, view `36e29a65-26bf-80df-8023-000c60b908c6`, fetched on 2026-07-07.

Interpretation: "blue lanes" means roadmap rows where `Initiative` includes `IronClaw core`, the blue initiative option in the database. Blue status values are not used as the selector.

Use one lane directory per implementation agent. Each lane has a `goal.md` written as an LFD launch document, not as a conventional feature spec.

All lanes inherit [COMMON.md](COMMON.md). A lane `goal.md` may tighten the shared rules but must not weaken them.

**Amendments (2026-07-07 review — read before assigning any lane):**

- [REVIEW-2026-07-07.md](REVIEW-2026-07-07.md) — design review of this
  portfolio; the launch-blocking finding is that lanes as originally
  written had each implementation agent author its own scorer and eval
  answers (a self-graded loss function).
- [INSTRUMENTS.md](INSTRUMENTS.md) — **overrides the paragraph above about
  agents materializing `harness/` and `eval/`**: instruments are
  designer-owned and pre-built in-tree (`lfd/_shared/` +
  `tests/integration/lfd/`); the implementation agent receives them
  read-only along with one writable runner-profile file. No lane launches
  before its INSTRUMENTS launch checklist passes.
- [LANE-ADDENDA.md](LANE-ADDENDA.md) — per-lane path corrections, Stage-0
  test suites, bindings to the designer briefs in `lfd/_briefs/`, and the
  launch waves / surface-conflict table for running lanes concurrently.

## Lane index

| Lane | Roadmap row | Status | Dates | Handoff |
| --- | --- | --- | --- | --- |
| 01 | Reborn | In development | 2026-05-11 to 2026-07-02 | [goal.md](01-reborn-umbrella/goal.md) |
| 02 | NEAR Foundation must-haves | Scoped & Designed | starts 2026-07-03 | [goal.md](02-near-foundation-must-haves/goal.md) |
| 03 | Slack as the main channel | Scoped & Designed | 2026-06-29 to 2026-07-07 | [goal.md](03-slack-as-main-channel/goal.md) |
| 04 | Secrets usage with Skills/Tools | Scoped & Designed | 2026-07-08 to 2026-07-21 | [goal.md](04-secrets-usage-with-skills-tools/goal.md) |
| 05 | Onboarding to channel first approach | In design | 2026-07-08 to 2026-07-17 | [goal.md](05-channel-first-onboarding/goal.md) |
| 06 | Clean up old architecture | Scoped & Designed | 2026-07-03 to 2026-07-14 | [goal.md](06-clean-up-old-architecture/goal.md) |
| 07 | Admin configurable skills/tools | Scoped & Designed | 2026-07-02 to 2026-07-16 | [goal.md](07-admin-configurable-skills-tools/goal.md) |
| 08 | Permission management | Not started | 2026-07-17 to 2026-08-11 | [goal.md](08-permission-management/goal.md) |
| 09 | Custom build tools | Not started | 2026-07-09 to 2026-07-25 | [goal.md](09-custom-build-tools/goal.md) |
| 10 | User-voice model | Not started | 2026-07-11 to 2026-07-22 | [goal.md](10-user-voice-model/goal.md) |
| 11 | Missions | Ideation | 2026-07-03 to 2026-07-12 | [goal.md](11-missions/goal.md) |
| 12 | Multi-tenant cross agent collaboration | Ideation | 2026-07-13 to 2026-07-31 | [goal.md](12-multi-tenant-cross-agent-collaboration/goal.md) |
| 13 | Self-learning loops | Ideation | 2026-07-05 to 2026-07-23 | [goal.md](13-self-learning-loops/goal.md) |
| 14 | Long-term memory | Ideation | 2026-08-12 to 2026-09-02 | [goal.md](14-long-term-memory/goal.md) |
| 15 | Reborn memory platform | In design | 2026-06-22 to 2026-07-31 | [goal.md](15-reborn-memory-platform/goal.md) |
| 16 | Memory placement: move memory to product layer | In design | 2026-06-22 to 2026-07-03 | [goal.md](16-memory-placement-product-layer/goal.md) |
| 17 | Self-learning write pipeline: store memory from turns | In design | 2026-07-03 to 2026-07-12 | [goal.md](17-self-learning-write-pipeline/goal.md) |
| 18 | Long-term memory retrieval pipeline: attach correct context | In design | 2026-07-12 to 2026-07-21 | [goal.md](18-long-term-memory-retrieval-pipeline/goal.md) |
| 19 | Memory benchmarks and evaluation | In design | 2026-07-21 to 2026-07-31 | [goal.md](19-memory-benchmarks-and-evaluation/goal.md) |

