# Design documents

A design document describes a technical vision and the principles guiding a
multi-milestone change, so a team stays aligned without re-deriving the
argument in every review. It is a living document: merge it early, iterate on
it as insight arrives, and treat a merged document as a working draft rather
than an approved plan.

The format follows [GitLab's architecture design documents](https://handbook.gitlab.com/handbook/engineering/architecture/design-documents/):
one directory per document, YAML frontmatter carrying its status and owners,
and the section skeleton Summary → Motivation (Goals / Non-Goals) → Proposal →
Design and implementation details → Alternative Solutions.

## When to write one

Write one when the change coordinates multiple teams or areas, spans several
milestones, alters system stability or security posture, or introduces a new
runtime component. Do not write one for refactors, dependency bumps, or work
whose shape is already well understood — the process overhead has to earn its
guidance and visibility.

## Status vocabulary

| Status | Meaning |
| --- | --- |
| `proposed` | Drafted and awaiting review; not yet accepted |
| `accepted` | Reviewed and approved; work may begin |
| `ongoing` | Implementation is actively underway |
| `implemented` | Complete; the document becomes a knowledge-sharing record |
| `postponed` | Deferred without rejection |
| `rejected` | Reviewed and not adopted in this form |

## Documents

| Document | Status | Owning area |
| --- | --- | --- |
| [Runtime-Swappable Components](runtime-swappable-components/README.md) | `proposed` | `~reborn::extensions` |

Related design records that predate this directory: `docs/reborn/target-architecture/`
(the crate-architecture north star) and `docs/reborn/subagent-spawn/`.
