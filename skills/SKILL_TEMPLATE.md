---
# SKILL.md Template for IronClaw
# Copy this file to skills/<your-skill>/SKILL.md and fill in the template
#
# For detailed documentation on skills, see docs/capabilities/skills.mdx
---

name: my-skill
version: "1.0.0"
# Short description shown in skill list and search results
description: One-line description of what this skill does
activation:
  # Words that trigger this skill (checked against message content)
  keywords:
    - "keyword1"
    - "keyword2"
  # Regex patterns for more specific triggers
  # Use sparingly - keywords are usually sufficient
  patterns:
    - "(?i)\\btrigger\\s*phrase\\b"
  # Words that prevent this skill from activating
  exclude_keywords:
    - "exclude this phrase"
  # Labels for categorization
  tags:
    - "category"
  # Maximum tokens this skill consumes per activation
  # Adjust based on skill complexity (500-3000 is typical)
  max_context_tokens: 1500
# Optional: skill dependencies
requires:
  # Required binaries (checked at activation time)
  bins: []
  # Required environment variables
  env: []
  # Companion skills (advisory - downloaded but not auto-invoked)
  skills: []
---

# My Skill

## Purpose
Describe what this skill does and when to use it.

## When to Activate
Explain the use cases this skill addresses.

## Workflow

### Step 1: [Action]
Describe what to do.

### Step 2: [Action]
Describe what to do.

## Output Format
Describe expected output or results.

## Examples
Show concrete examples of how to use this skill.
