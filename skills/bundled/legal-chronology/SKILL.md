---
name: legal-chronology
version: 1.0.0
description: Builds evidence-backed event chronology from matter documents.
activation:
  keywords: ["chronology", "timeline", "sequence of events", "event log"]
  tags: ["legal", "timeline", "litigation"]
metadata:
  domain: legal
  requires_matter: true
  citation_mode: required
  clawyer:
    requires: {}
---
Build a chronology with source-backed entries only.

Requirements:
- Write under `matters/<matter_id>/chronology/`.
- Each event must include date/time confidence and provenance.
- Distinguish verified facts from inferred sequencing.
- If conflicting sources exist, list both and mark conflict.

Output artifacts:
- `matters/<matter_id>/chronology/master-chronology.md`
- `matters/<matter_id>/chronology/source-index.md`

Event row format:
- Date:
- Event:
- Source:
- Confidence:
- Notes:
