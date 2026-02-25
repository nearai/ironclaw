---
name: legal-litigation-support
version: 1.0.0
description: Litigation workflow support for pleadings, discovery, and witness prep.
activation:
  keywords: ["litigation", "discovery", "pleading", "deposition", "witness"]
  tags: ["legal", "litigation", "discovery"]
metadata:
  domain: legal
  requires_matter: true
  citation_mode: required
  clawyer:
    requires: {}
---
Support litigation tasks with clear deliverables and provenance.

Requirements:
- Write under `matters/<matter_id>/litigation/`.
- Flag deadline-sensitive items first.
- Keep a dedicated section for assumptions and uncertainty.
- Avoid legal conclusions without source support.

Output artifacts:
- `matters/<matter_id>/litigation/discovery-plan.md`
- `matters/<matter_id>/litigation/witness-questions.md`
- `matters/<matter_id>/litigation/issue-theories.md`

Always include:
- Facts relied on
- Authorities/documents cited
- Gaps requiring attorney review
