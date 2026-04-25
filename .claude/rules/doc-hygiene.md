---
paths:
  - "**/*.md"
  - "**/*.py"
  - "docs/**"
---
# Doc Hygiene

## Absolute Paths

Committed `.md` and `.py` files (outside `tests/` and `scripts/`)
must not contain developer-local absolute paths (`/home/<user>/`,
`/Users/<user>/`, `/tmp/`). This is a review convention, not
pre-commit-enforced — grep before merging a docs-touching PR.
Reference: PR #2689.
