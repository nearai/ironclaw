---
paths:
  - "src/skills/**"
  - "skills/**"
---
# Skills System

SKILL.md files extend the agent's prompt with domain-specific instructions. Each skill is a YAML frontmatter block (metadata, activation criteria, required tools) followed by a markdown body injected into the LLM context.

## Trust Model

| Trust Level | Source | Tool Access |
|-------------|--------|-------------|
| **Trusted** | User-placed in `~/.ironclaw/skills/` or workspace `skills/` | All tools available to the agent |
| **Installed** | Downloaded from ClawHub registry (`~/.ironclaw/installed_skills/`) | Read-only tools only (no shell, file write, HTTP) |

## SKILL.md Format

```yaml
---
name: my-skill
version: 0.1.0
description: Does something useful
activation:
  patterns:
    - "deploy to.*production"
  keywords:
    - "deployment"
  exclude_keywords:
    - "rollback"
  tags:
    - "devops"
  max_context_tokens: 2000
requires:
  bins: [docker, kubectl]
  env: [KUBECONFIG]
---

# Skill instructions here...
```

## Selection Pipeline

1. **Gating** -- Check binary/env/config requirements; skip skills whose prerequisites are missing
2. **Scoring** -- Deterministic scoring: keywords (10/5 pts, cap 30) + patterns (20 pts, cap 40) + tags (3 pts, cap 15). `exclude_keywords` veto (score = 0 if any present)
3. **Budget** -- Select top-scoring skills within `SKILLS_MAX_TOKENS` prompt budget
4. **Attenuation** -- Minimum trust across active skills determines tool ceiling; installed skills lose dangerous tools

## Skill Tools

- `skill_list` -- List all discovered skills with trust level and status
- `skill_search` -- Search ClawHub registry for available skills
- `skill_install` -- Download and install a skill from ClawHub
- `skill_remove` -- Remove an installed skill
