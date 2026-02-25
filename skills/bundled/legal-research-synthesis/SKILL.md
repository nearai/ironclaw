---
name: legal-research-synthesis
version: 1.0.0
description: Synthesizes legal research into citation-backed memo format.
activation:
  keywords: ["legal research", "memo", "authority", "case law", "statute"]
  tags: ["legal", "research", "memo"]
metadata:
  domain: legal
  requires_matter: true
  citation_mode: required
  clawyer:
    requires: {}
---
Produce legal research synthesis with strict citation hygiene.

Requirements:
- Write under `matters/<matter_id>/research/`.
- Separate question presented, facts, analysis, and conclusion.
- Identify controlling vs persuasive authority when possible.
- Explicitly note jurisdiction assumptions.
- If support is absent or uncertain, state `insufficient evidence`.

Output artifacts:
- `matters/<matter_id>/research/research-memo.md`
- `matters/<matter_id>/research/authority-table.md`
- `matters/<matter_id>/research/open-research-questions.md`

Memo checklist:
- Question Presented
- Short Answer
- Facts (with source references)
- Analysis (authority-by-authority)
- Risks and uncertainty
- Next research steps
