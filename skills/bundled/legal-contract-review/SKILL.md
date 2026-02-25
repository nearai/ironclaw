---
name: legal-contract-review
version: 1.0.0
description: Clause-by-clause contract risk review with matter-scoped issue lists.
activation:
  keywords: ["contract review", "msa", "nda", "clause", "redline"]
  tags: ["legal", "contract", "transactional"]
metadata:
  domain: legal
  requires_matter: true
  citation_mode: required
  clawyer:
    requires: {}
---
Perform contract review with explicit risk classification and citations.

Requirements:
- Write under `matters/<matter_id>/contracts/`.
- For each issue, include clause reference and exact language excerpt pointer.
- Use risk levels: low, medium, high, critical.
- Include practical fallback language suggestions.
- If no support exists for a claim, say `insufficient evidence`.

Output artifacts:
- `matters/<matter_id>/contracts/issue-list.md`
- `matters/<matter_id>/contracts/fallback-language.md`
- `matters/<matter_id>/contracts/unresolved-questions.md`

Issue format:
- Clause:
- Issue:
- Risk:
- Why it matters:
- Suggested revision:
- Source citation:
