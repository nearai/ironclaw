---
name: legal-intake
version: 1.0.0
description: Structured intake workflow for new legal matters.
activation:
  keywords: ["intake", "new matter", "client interview", "issue spotting"]
  tags: ["legal", "intake", "matter"]
metadata:
  domain: legal
  requires_matter: true
  citation_mode: required
  clawyer:
    requires: {}
---
When activated, perform legal intake in a disciplined structure.

Requirements:
- Write outputs under `matters/<matter_id>/` only.
- Separate facts from analysis.
- Add source traceability for each factual statement (`[doc/page/section]` when available).
- If support is missing, state `insufficient evidence`.

Output artifacts:
- `matters/<matter_id>/intake/intake-summary.md`
- `matters/<matter_id>/intake/open-questions.md`
- `matters/<matter_id>/intake/risk-register.md`

Use this template:
1. Parties and roles
2. Timeline summary
3. Claims/defenses at issue
4. Immediate deadlines
5. Unknowns and evidence gaps
6. Risk/uncertainty
