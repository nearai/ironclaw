---
name: legal
version: "0.1.0"
description: Project- and document-aware legal review. Upload PDFs/DOCX into a project, then chat about them with the agent. Companion streams add chat-with-docs and DOCX export.
activation:
  keywords:
    - "legal"
    - "contract"
    - "agreement"
    - "nda"
    - "msa"
    - "redline"
    - "redlining"
    - "review"
    - "deal"
    - "clause"
    - "terms"
    - "due diligence"
    - "diligence"
    - "lease"
    - "loi"
    - "memorandum"
    - "term sheet"
    - "indemnity"
    - "warranty"
    - "exhibit"
    - "schedule"
  exclude_keywords:
    - "memory"
    - "routine"
  patterns:
    - "(?i)\\b(review|redline|redrafting|markup)\\s+(this|the|my|our)?\\s*(contract|agreement|nda|msa|loi|term ?sheet|lease)\\b"
    - "(?i)\\b(upload|attach|add)\\s+(a |the )?(pdf|docx|document|contract|agreement)\\b"
    - "(?i)\\bexplain\\s+(clause|section|paragraph)\\s+\\w+"
  tags:
    - "legal"
    - "documents"
  max_context_tokens: 2000
---

# Legal Harness

The legal skill is a project-aware document workspace. A **project** is a
container for a transaction or matter (NDA, M&A deal, employment file,
landlord/tenant case). Each project holds **documents** (PDF or DOCX),
plus the **chats** the user has had about those documents.

## When to use this skill

Activate when the user wants to:

- Review, redline, or summarise a contract / agreement / NDA / MSA / LOI /
  term sheet / lease.
- Upload one or more documents and ask questions about them.
- Compare clauses, surface unusual terms, or extract obligations.
- Continue a prior chat about a deal already in the workspace.

Don't activate for memory operations, routine scheduling, or generic
research — those are owned by other skills.

## Capabilities (foundation, this PR)

- Create / list / soft-delete a project.
- Upload PDFs and DOCX into a project; the gateway extracts text inline,
  computes a sha256, dedupes within the project, and stores the blob on
  the local filesystem under the ironclaw data dir.
- Fetch a project (with all its non-deleted documents joined) or a single
  document (metadata + extracted text + raw bytes).

## Capabilities (companion streams)

- **Chat-with-docs (Stream B):** create a chat in a project, post a user
  message, and receive a streamed (SSE) assistant reply. The assistant is
  given the project's document text up to a configurable budget.
- **DOCX export (Stream C):** render a chat thread (user + assistant
  turns, with timestamps and document refs) into a downloadable `.docx`.

## HTTP surface

All routes live under the gateway's `/api/skills/legal/` prefix and
require the standard ironclaw gateway token (the `Authorization: Bearer
<token>` header set by `auth_middleware`).

| Verb | Path | Purpose |
|------|------|---------|
| POST | `/api/skills/legal/projects` | Create a project |
| GET  | `/api/skills/legal/projects` | List active projects |
| GET  | `/api/skills/legal/projects/:id` | Project + its documents |
| DELETE | `/api/skills/legal/projects/:id` | Soft-delete a project |
| POST | `/api/skills/legal/projects/:id/documents` | Upload (multipart) |
| GET  | `/api/skills/legal/documents/:id` | Document metadata + text |
| GET  | `/api/skills/legal/documents/:id/blob` | Raw document bytes |

The **Chat** routes (`/api/skills/legal/projects/:id/chats`,
`/api/skills/legal/chats/:id`, etc.) and the **DOCX export** route
(`/api/skills/legal/chats/:id/export.docx`) ship in companion PRs against
the same migration.

## Storage

- DB: ironclaw's libSQL/Turso embedded backend. v1 is libSQL-only; the
  PostgreSQL backend has the matching schema migration but no Rust query
  layer wired in yet (a 501 is returned).
- Blobs: `<ironclaw_base_dir>/legal/blobs/<sha[0..2]>/<sha>` —
  content-addressed; identical bytes share one file across projects.

## Constraints

- 10 MiB upload cap per file (the gateway's body limit applies an outer
  14 MiB cap; the legal handler applies the tighter 10 MiB cap inline).
- Supported formats: `application/pdf` and the OOXML mime
  (`application/vnd.openxmlformats-officedocument.wordprocessingml.document`).
- Project name ≤ 200 chars; metadata JSON ≤ 16 KiB.

## Notes for the agent

- Soft-deleted projects (those with a non-null `deleted_at`) are treated
  as missing by every endpoint. There is no recovery path in v1; a deleted
  project must be recreated.
- Document upload is idempotent within a project: re-uploading the same
  bytes returns the existing row instead of writing a duplicate.
- The migration introduced here is canonical and shared with Streams B
  and C. Do not propose schema changes from this skill — open a follow-up
  issue.
