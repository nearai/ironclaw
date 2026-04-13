---
name: coding
version: "1.0.0"
description: Best practices for code editing, search, and file operations
activation:
  keywords:
    - "code"
    - "edit"
    - "fix"
    - "implement"
    - "refactor"
    - "bug"
    - "function"
    - "class"
    - "file"
    - "module"
    - "test"
    - "compile"
    - "build"
    - "error"
    - "change"
    - "rename"
    - "delete"
    - "add"
    - "update"
  exclude_keywords:
    - "memory"
    - "routine"
    - "schedule"
  patterns:
    - "(?i)(add|remove|update|modify|create|delete|rename|move)\\s.*(file|function|class|method|variable|import)"
    - "(?i)(fix|debug|investigate|trace|find)\\s.*(bug|error|issue|crash|fail)"
  tags:
    - "development"
    - "coding"
  max_context_tokens: 1500
---

# Coding Best Practices

## Tool Usage Discipline

- **Prefer `apply_patch` over `write_file`** for modifying existing files. It sends only the changed portion, preventing accidental full-file rewrites.
- **Always `read_file` before editing.** Understand the context before changing code. Never edit a file you haven't read.
- **Use `glob` for file discovery** instead of `shell` with `find` or `ls`. It's faster, safer, and returns structured results sorted by modification time.
- **Use `grep` for content search** instead of `shell` with `grep` or `rg`. It provides structured output modes (content, file paths, counts) and pagination.
- **Use `list_dir` for directory exploration** instead of `shell` with `ls`.
- **Read before writing.** Never create or overwrite a file without reading it first (unless it's genuinely a new file).

## Code Change Discipline

- **Minimal changes.** Don't add features, refactor, or "improve" beyond what was asked. A bug fix doesn't need surrounding code cleaned up.
- **No unnecessary comments or docstrings.** Only add comments where the logic isn't self-evident. Don't add type annotations or docstrings to code you didn't change.
- **One thing at a time.** Make focused changes, verify with `read_file`, then move to the next change.
- **Fix the pattern, not just the instance.** When you find a bug, use `grep` to search for all occurrences of the same pattern before committing a fix.

## Code Quality

- Don't introduce security vulnerabilities (command injection, XSS, SQL injection, path traversal).
- Preserve existing code style and conventions. Match the indentation, naming, and patterns of surrounding code.
- Test after changes when test infrastructure exists. Use `shell` to run the project's test command.
- Don't add error handling, fallbacks, or validation for scenarios that can't happen. Trust internal code and framework guarantees.
