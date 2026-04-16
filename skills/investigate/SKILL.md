---
name: investigate
version: "1.0.0"
description: Debug and investigate issues - traces errors, gathers context, and systematically finds root causes
activation:
  keywords:
    - "investigate"
    - "debug"
    - "troubleshoot"
    - "why is"
    - "what's wrong"
    - "broken"
    - "failing"
    - "error"
    - "crash"
    - "bug"
  exclude_keywords:
    - "memory"
    - "RAM"
    - "hardware"
  patterns:
    - "(?i)\\b(investigate|debug|troubleshoot)\\b"
    - "(?i)\\bfix(ed|ing)?\\s+(the\\s+)?(bug|issue|error|problem)"
    - "(?i)\\bsomething\\s+(is\\s+)?(wrong|broken|not\\s+working)"
  tags:
    - "debugging"
    - "investigation"
  max_context_tokens: 2000
requires:
  skills:
    - coding
---

# Investigate Skill

Use this skill when the user asks you to debug, investigate, or find the root cause of an issue.

## Investigation Workflow

### Step 1: Gather Context

Before diving in, understand the problem:

1. **What is the user trying to do?** (their goal)
2. **What did they expect to happen?**
3. **What actually happened?** (error message, incorrect output, crash)
4. **When did it start?** (worked before? recent changes?)

### Step 2: Reproduce the Issue

1. If there's an error message, search for it:
   ```
   grep -r "error message" --include="*.py" --include="*.ts" --include="*.rs" .
   ```

2. If there's a failing command, try to reproduce it:
   ```
   shell(command="<the failing command>", workdir="<project root>")
   ```

3. Check recent changes that might have caused this:
   ```
   shell(command="git log --oneline -10")
   ```

### Step 3: Trace the Problem

Follow the execution path:

1. **Start from the entry point** - where does the code begin executing?
2. **Find the failure point** - where does it break?
3. **Work backwards** - what conditions lead to this failure?

Use `grep` to find relevant code:
```
grep(pattern="function_or_variable_name", include="*.py")
```

### Step 4: Identify Root Cause

Common patterns to look for:

- **Logic errors**: wrong conditions, off-by-one, inverted logic
- **Null/undefined handling**: missing null checks, unwrapping None
- **Async issues**: missing await, race conditions
- **Resource issues**: file not closed, connection pool exhausted
- **Configuration**: wrong env vars, missing secrets

### Step 5: Propose Fix

Present findings clearly:

```
## Investigation Summary

**Problem**: [1 sentence]
**Root Cause**: [1 sentence]
**Location**: [file:line if known]
**Fix**: [concrete recommendation]

## Evidence
- [Evidence 1]
- [Evidence 2]
```

### Step 6: Offer to Fix

Ask the user if they want you to fix the issue, then proceed with `coding` skill guidance.

## Investigation Best Practices

1. **Be systematic** - don't jump to conclusions, follow the evidence
2. **Start simple** - check obvious causes first (typos, missing files, env vars)
3. **Narrow the scope** - find the minimal reproduction case
4. **Document as you go** - save key findings to memory for context
5. **Verify the fix** - confirm the issue is resolved before moving on
