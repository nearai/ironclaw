---
name: code-review
version: "1.0.0"
description: Review code changes for bugs, style, and security issues
activation:
  keywords:
    - "review"
    - "code review"
    - "review changes"
  patterns:
    - "(?i)review\\s.*(code|changes|diff|PR|pull request|commit)"
    - "(?i)(check|look at|inspect)\\s.*(changes|diff|code)"
  tags:
    - "code-review"
    - "quality"
  max_context_tokens: 1200
---

# Code Review Workflow

When the user asks to review code:

1. **Get the changes**: Run `shell` with `git diff` (unstaged) or `git diff --cached` (staged) or `git diff HEAD~1` (last commit) depending on context.
2. **Focus on what changed**, not surrounding code. Don't review unchanged code unless it's directly relevant to the change.
3. **Check for these categories:**
   - **Bugs**: Logic errors, off-by-one, null/undefined handling, race conditions
   - **Security**: Injection vulnerabilities, credential exposure, path traversal, XSS
   - **Error handling**: Missing error cases, swallowed errors, unclear error messages
   - **Edge cases**: Empty inputs, large inputs, concurrent access, unicode handling
   - **Style**: Inconsistency with surrounding code, unclear naming, missing/excessive comments
4. **Provide actionable feedback** with specific file:line references. Don't just say "this could be improved" - say what to change and why.
5. **Be proportional**: A one-line typo fix doesn't need a full security audit. Match review depth to change scope.
6. If the changes look good, say so clearly. Don't invent problems.
