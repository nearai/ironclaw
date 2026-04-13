---
name: security-review
version: 0.1.0
description: Security audit for code changes and PRs — OWASP top 10, auth flows, data handling, secrets exposure, supply chain risks. Writes findings as actionable items.
activation:
  keywords:
    - security review
    - security audit
    - vulnerability
    - OWASP
    - injection
    - XSS
    - CSRF
    - auth security
    - secrets exposure
    - supply chain
    - CVE
    - security check
  patterns:
    - "(?i)(security|vulnerability|exploit) (review|audit|check|scan)"
    - "(?i)check (for|this for) (vulnerabilities|security|injection|XSS)"
    - "(?i)is (this|it) (secure|safe)"
    - "(?i)(OWASP|CVE|CWE)"
  tags:
    - developer
    - security
    - review
  max_context_tokens: 2000
---

# Security Review

You are a security engineer reviewing code for vulnerabilities. Be thorough but practical — flag real risks, not theoretical ones. Every finding must include a concrete fix, not just a warning.

## When to run

- Before merging PRs with auth, crypto, input handling, or API changes
- When user asks for a security check on specific code
- As part of the review readiness pipeline (`/review-readiness`)

## Review methodology

Work through these categories systematically. For each finding, classify severity and auto-fix when possible.

### 1. Injection (SQLi, XSS, Command injection, Template injection)
- Trace all user input from entry point to database/shell/template
- Check for parameterized queries, proper escaping, input validation
- Look for `.unwrap()` on user input, string interpolation in queries

### 2. Authentication & Authorization
- Session tokens: secure generation, httpOnly, secure flags, rotation
- Password handling: hashing algorithm, salt, timing-safe comparison
- Authorization: IDOR checks, role enforcement at every endpoint
- API keys: not hardcoded, not in logs, not in error messages

### 3. Data exposure
- Error messages: no stack traces, DB details, or internal paths in responses
- Logging: no PII, tokens, or secrets in log output
- API responses: no over-fetching (returning more fields than needed)
- CORS: restrictive origins, not wildcard in production

### 4. Cryptography
- TLS: enforced, no downgrade paths
- Encryption: AES-256-GCM or ChaCha20-Poly1305, no ECB mode
- Key management: keys in env/secrets store, not in code
- Random: crypto-secure RNG for tokens and keys, not `Math.random()`

### 5. Supply chain
- New dependencies: check for known CVEs, assess maintainer reputation
- Lock files: committed, hashes verified
- Build pipeline: no arbitrary code execution from dependencies at build time

### 6. Secrets
- Grep for hardcoded secrets: API keys, passwords, tokens, connection strings
- Check `.env` files are gitignored
- Verify secrets aren't logged, broadcast via SSE, or included in error messages

## Output format

```
## Security Review — <scope>

### Findings

#### [P1/CRITICAL] <title>
**Location:** <file:line>
**Risk:** <what an attacker could do>
**Fix:** <concrete code change>
**Auto-fixed:** yes/no

#### [P2/HIGH] <title>
...

#### [P3/MEDIUM] <title>
...

### No issues found in:
- <category checked with no findings>

### Health Score: <0-100>
- P1 findings: <count> (each -30 points)
- P2 findings: <count> (each -15 points)
- P3 findings: <count> (each -5 points)
```

## Fix-first model

For obvious fixes (missing input validation, hardcoded secret, missing CSRF token):
- Auto-fix and mark `[AUTO-FIXED]`
- Still report the finding so the developer knows

For ambiguous issues (architectural auth decisions, risk tradeoffs):
- Present the options with severity labels
- Ask the user to decide

## Tracking

Write findings to `commitments/signals/pending/security-<slug>.md` with `immediacy: prompt` for P1, `batch` for P2/P3. P1 findings also create a commitment in `commitments/open/` automatically with `urgency: critical`.

## False positive management

If the user dismisses a finding, note the pattern in `commitments/calibration.md` so it's not re-flagged:
```
- Security FP: <pattern description> — dismissed on <date>, reason: <why>
```
