# Documentation PR Review Skill

> Lessons learned from ironclaw PR #294 - 39+ review comments across multiple rounds

## Overview

This skill ensures documentation PRs are comprehensive and pass automated review (Gemini, Copilot) on the first attempt.

---

## Pre-Push Validation Checklist

Run ALL checks before pushing to PR:

### 1. Date Consistency
```bash
# Check all dates are current
grep -rn "2026-02-" . --include="*.md" | grep -v "$(date +%Y-%m-%d)"
# Should return empty (all dates = today)
```

### 2. Port Consistency
```bash
# Ensure all ports match the default
grep -rn "3001\|3002\|3003" . --include="*.md"
# Should be empty or intentional (3000 is default)
```

### 3. No Placeholder Values
```bash
# Check for unresolved placeholders
grep -rn "YOUR_USERNAME\|YOUR_TOKEN\|changeme\|TODO\|FIXME\|<run:" . --include="*.md"
# All should have clear instructions or be intentional examples
```

### 4. No Absolute Paths
```bash
# Check for local absolute paths
grep -rn "/Users/\|/home/\|C:\\\\Users" . --include="*.md"
# Should be empty - use relative paths (src/module/file.rs)
```

### 5. Security Placeholders
```bash
# Auth tokens must have security warnings
grep -B2 "AUTH_TOKEN\|API_KEY\|SECRET" . --include="*.md" | grep -i "warning\|secure\|generate"
# Each placeholder should have a security note
```

### 6. Spelling & Branding
```bash
# Common branding mistakes
grep -rn "Github\|github.com\|npmjs\|postgres\|sqlite" . --include="*.md"
# Should be: GitHub, github.com, npmjs.com, PostgreSQL, SQLite/libSQL
```

### 7. Auto-Generated Content
```bash
# Check for accidental commits of personal/workstation files
grep -rn "claude-mem\|auto-generated\|personal\|workspace-specific" . --include="*.md"
# Remove or replace with appropriate public content
```

### 8. Line Count/Statistics Accuracy
```bash
# Verify any statistics mentioned
# Example: "115,000 lines of Rust" should include measurement context
# Format: "~X lines (measured on vY.Z.0, includes tests, comments)"
```

---

## Common Review Issues & Fixes

### Issue 1: Knowledge Cutoff Dates
**Problem:** Document date vs knowledge cutoff inconsistency
**Fix:** Remove knowledge cutoff references or clarify they apply to model training

### Issue 2: Unsafe Defaults
**Problem:** `GATEWAY_AUTH_TOKEN=changeme` or similar
**Fix:** `GATEWAY_AUTH_TOKEN=*(required)*` with security note

### Issue 3: Supply Chain Warnings
**Problem:** `npm install package@latest` without warning
**Fix:** Add note: "For production, pin to specific version"

### Issue 4: OAuth Credentials in Docs
**Problem:** Hardcoded OAuth client IDs without context
**Fix:** Add security note explaining why this is acceptable for installed apps

### Issue 5: Token Generation Instructions
**Problem:** `<run: openssl rand -hex 32>` won't execute
**Fix:** 
```
TOKEN=REPLACE_WITH_SECURE_TOKEN
# Generate: openssl rand -hex 32
```

### Issue 6: YOUR_USERNAME Placeholders
**Problem:** Multiple instances without clear instruction
**Fix:** Add note: "Replace all instances of YOUR_USERNAME"

---

## Process Improvements

### 1. Batch All Fixes Before Pushing
```
❌ Bad:  push → fix → push → fix → push → fix
✅ Good: collect all issues → fix all → push once
```

### 2. Request Review After Complete
```
❌ Bad:  /gemini review → push → /gemini review → push → /gemini review
✅ Good: finish all changes → /gemini review once
```

### 3. Self-Review Against Source Code
Before pushing, verify documentation matches actual source:
```bash
# Check documented defaults match code
grep -rn "default.*=" src/ --include="*.rs" | grep -i "port\|token\|timeout"

# Check documented features match feature flags
grep -rn "#\[cfg(feature" src/ --include="*.rs"
```

### 4. Sync Both Repos
When working with ironclaw-docs → ironclaw:
```bash
# Always sync in order:
1. Edit ironclaw-docs/
2. Commit to ironclaw-docs
3. cp to ironclaw/docs/
4. Commit to ironclaw PR
5. Push both
```

---

## PR Comment Resolution Template

When resolving review comments, use this format:

```
**Fixed in commit:** abc1234

**Change made:**
- [Description of fix]

**Verification:**
- [How to verify the fix is correct]
```

---

## Quick Reference: Default Values

Unless a document explicitly describes a different value, use repo defaults:

| Setting | Value | Notes |
|---------|-------|-------|
| GATEWAY_PORT | 3000 | Default port |
| GATEWAY_HOST | 127.0.0.1 | Localhost only |
| RUST_VERSION | 1.92 | Matches Cargo.toml `rust-version` (MSRV) |
| DATABASE_BACKEND | postgres | Matches `DatabaseConfig` default backend |
| SANDBOX_ENABLED | true | Secure by default |

---

## Automated Review Bots

| Bot | Triggers | Typical Issues |
|-----|----------|----------------|
| Gemini Code Assist | Every push | Version mismatches, inconsistencies, typos |
| GitHub Copilot | Every push | Security concerns, placeholder values |

**Tip:** Both bots review on every push. Batch changes to minimize review cycles.

---

## File-Specific Checks

### CLAUDE.md / AI Context Files
- Remove auto-generated activity logs
- Add purpose/description header
- Keep only useful public content

### INSTALLATION.md
- Add placeholder replacement notes
- Include token generation instructions BEFORE placeholder
- Note about replacing all YOUR_USERNAME instances

### README.md
- Line counts need measurement context
- Token generation must be actionable (not inline)
- Brand correctly: GitHub (not Github)

### analysis/*.md
- No absolute paths
- Relative paths from project root
- Security warnings for any credentials/tokens

---

*Last updated: 2026-02-22 | Version: 1.0*