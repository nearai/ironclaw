---
name: developer-setup
version: 0.2.0
description: One-time onboarding for the developer workflow — installs github-workflow missions, creates the commitments workspace, registers per-repo projects, writes calibration memories. After successful setup this skill is excluded from selection until the marker file is deleted.
activation:
  setup_marker: commitments/.developer-setup-complete
  keywords:
    - developer assistant
    - dev assistant
    - dev workflow
    - developer setup
    - github setup
    - help with github
    - manage my PRs
    - code workflow
    - engineering setup
    - dev setup
    - automate my repos
    - CI keeps failing
    - engineering workflow
  patterns:
    - "(?i)I'm a (developer|engineer|programmer|dev|software engineer)"
    - "(?i)help me (with|manage|set ?up) (github|PRs|repos|CI|code|projects)"
    - "(?i)set ?up.*(dev|coding|engineering|github|developer)"
    - "(?i)(automate|manage) my (repos|PRs|projects|workflow|code)"
  tags:
    - commitments
    - developer
    - github
    - setup
  max_context_tokens: 3000
requires:
  # Capped at MAX_REQUIRED_SKILLS_PER_MANIFEST = 10 in
  # `ironclaw_skills::types`. The trimmed list keeps the 10 highest-impact
  # companions for the developer workflow; the dropped entries
  # (`qa-review`, `review-readiness`, `product-prioritization`) can still
  # be installed manually via `skill_install` when needed.
  skills:
    - commitment-triage
    - commitment-digest
    - decision-capture
    - delegation-tracker
    - idea-parking
    - tech-debt-tracker
    - project-setup
    - github
    - github-workflow
    - security-review
---

# Developer Workflow Setup

You are configuring the full developer workflow — commitment tracking, GitHub automation, tech debt tracking, security/QA reviews, product prioritization, and proactive briefings across multiple repositories.

## Companion skills

These activate during conversation via keyword matching:

| Skill | When | What |
|---|---|---|
| `commitment-triage` | Obligations, deadlines | Signal extraction, commitment creation |
| `commitment-digest` | "show commitments" | Formatted status summary |
| `decision-capture` | Architecture/design decisions | Records decision + rationale |
| `delegation-tracker` | "waiting on @teammate" | Tracks delegation follow-ups |
| `idea-parking` | "park this idea" | Saves for later |
| `tech-debt-tracker` | "this is a hack", "refactor later" | Tracks tech debt, resurfaces weekly |
| `project-setup` | "add repo owner/repo" | Adds a new project with workflow |
| `security-review` | "security review", "check for vulnerabilities" | OWASP audit, auto-fix obvious issues |
| `qa-review` | "QA review", "test coverage", "edge cases" | Test plans, coverage gaps, regression risks |
| `review-readiness` | "ready to merge?", "PR readiness" | Tracks which reviews are complete per branch |
| `product-prioritization` | "what to build next", "prioritize" | Evidence-based feature scoring, demand analysis |
| `github` | GitHub API operations | REST API with credential injection |
| `github-workflow` | Workflow automation reference | Issue-to-merge pipeline templates |
| `review-checklist` | Pre-merge review | 55+ verification items |

If any are missing from `skills/`, tell the user which ones are needed.

## Step 1: Setup questions (4, no timezone)

1. **Repos**: Which GitHub repos do you work on? (1-5, format: `owner/repo`)
2. **Role**: Solo maintainer, team member, or team lead? (Affects delegation vs personal tracking)
3. **Per-repo**: For each repo — who are maintainers/reviewers? Do you use a staging branch?
4. **AI agents**: Do any bots create PRs? (Dependabot, Copilot, internal agents) — these get tracked separately in digests with shorter stale thresholds

Use reasonable defaults if the user says "just set it up."

## Step 2: Create workspace structure

1. Check if `commitments/README.md` exists. If not, create the full commitments workspace (see `commitment-setup` skill for the complete schema including immediacy, resolution paths, trust calibration).
2. Create subdirectory placeholders: `open/`, `resolved/`, `signals/pending/`, `signals/expired/`, `decisions/`, `parked-ideas/`.
3. Create `commitments/tech-debt/README.md` — "Tech debt items. Resurface in weekly retro."

## Step 3: Set up each project

For each repo the user listed, run the `project-setup` procedure:
1. Validate repo via GitHub API
2. Create `projects/<owner>-<repo>/project.md` with metadata
3. Create `projects/<owner>-<repo>/notes.md` for developer notes
4. Install the 6 workflow missions (namespaced by repo slug)
5. Skip `wf-staging-review` if no staging branch

## Step 4: Create developer missions

### commitment-triage (3x weekdays)

```
mission_create(
  name: "commitment-triage",
  goal: "Developer triage. Read commitments/README.md for schema. Read projects/ via memory_tree for all tracked repos. For each repo, check GitHub API: (1) New PR review requests assigned to user → signal with immediacy=batch. (2) CI failures on user's open PRs → signal with immediacy=prompt. (3) @mentions on PRs/issues → signal with immediacy=prompt. (4) New issue assignments → signal with immediacy=batch. (5) Issues/PRs with production/hotfix/critical labels → signal with immediacy=realtime, broadcast immediately. (6) Recently merged PRs — scan review comments for tech-debt patterns ('address in follow-up', 'not blocking but fix later', 'TODO', 'leaving for now') → create tech-debt items in commitments/tech-debt/ with source=pr-review and source_pr reference. Expire signals after 48h. Flag AI agent PRs stuck in CI after 24h. Append summary to commitments/triage-log.md.",
  cadence: "0 9,14,18 * * 1-5"
)
```

### commitment-digest (weekday mornings)

```
mission_create(
  name: "commitment-digest",
  goal: "Developer morning brief. Read commitments/README.md for schema. Read projects/ for tracked repos. For each repo, query GitHub API. Compose digest in this order: (1) OVERNIGHT RESULTS — CI status per repo on user's PRs (green/red/pending), PRs merged overnight. (2) NEEDS YOUR REVIEW — PRs where user is requested reviewer, show age, author, size. Separate human PRs from AI agent PRs. Flag stale reviews (3+ days). (3) YOUR OPEN PRs — each with CI status, review state. Flag READY TO MERGE if approved + CI green. (4) BLOCKED/WAITING — commitments with status=waiting or delegated_to set, agent PRs stuck in CI loops (attempted 3+ fixes). (5) TODAY'S COMMITMENTS — open items sorted by urgency, for agent_can_handle items note what agent would do. (6) QUICK STATS — tech debt count, pending signal count. End with 'Did I miss anything?' Send via message tool. Omit empty sections.",
  cadence: "0 8 * * 1-5"
)
```

### dev-stale-pr-check (weekday afternoons)

```
mission_create(
  name: "dev-stale-pr-check",
  goal: "Check for stale PRs across tracked repos. Read projects/ for repo list. For each repo, query GitHub API for open PRs. Flag PRs with no activity in 3+ days (human) or 1+ day (agent PR stuck in CI). For user's own stale PRs: suggest pinging reviewer or closing if abandoned. For PRs user should review: note how long they've been waiting. Send alert only if stale items found; stay silent otherwise.",
  cadence: "0 16 * * 1-5"
)
```

### dev-weekly-retro (Friday morning)

```
mission_create(
  name: "dev-weekly-retro",
  goal: "Weekly developer retrospective. Gather: (1) All commitments resolved this week from commitments/resolved/. (2) All decisions captured this week from commitments/decisions/. (3) All tech debt items added this week from commitments/tech-debt/ — include items from PR review scans. (4) Per-repo: count of merged PRs this week via GitHub API. (5) Open items carried forward. Compose retro: SHIPPED, DECISIONS MADE (with rationale), SLIPPED/CARRIED FORWARD, TECH DEBT ACCUMULATED (new items + total count + top 3 chronic), PATTERNS (recurring CI failures, slow review cycles). For complex action items, suggest using /plan to create a structured execution plan. Write retro to context/intel/weekly-retro-<date>.md. Send via message tool.",
  cadence: "0 10 * * 5"
)
```

### dev-decision-outcome-check (Wednesday)

```
mission_create(
  name: "dev-decision-outcome-check",
  goal: "Check for decisions needing outcome assessment. Read commitments/decisions/ for entries where outcome is null and decided_at is 7+ days ago. For each, prompt: 'You decided <X> <N> days ago. How did it turn out?' Skip silently if no decisions need review.",
  cadence: "0 10 * * 3"
)
```

### dev-tech-debt-resurface (Monday morning)

```
mission_create(
  name: "dev-tech-debt-resurface",
  goal: "Weekly tech debt review. Read all files in commitments/tech-debt/ via memory_tree and memory_read. Sort by age. Flag items older than 30 days as chronic. For items tagged with a repo, check if related issues exist. If backlog exceeds 10 items, suggest a prioritization session. For high-severity chronic items, suggest using /plan to create a structured breakdown and fix strategy. Send list via message tool. Skip silently if no tech debt.",
  cadence: "0 10 * * 1"
)
```

## Step 5: Write calibration memories

```
memory_write(
  target: "commitments/calibration.md",
  content: "# Developer Calibration\n\n## Decision classification\n- mechanical (auto-act silently): expire stale signals, update CI status, dismiss noise, mark passing checks\n- taste (auto-act, surface in digest): auto-dismiss FYI signals, auto-resolve completed items, update readiness dashboard\n- challenge (always ask): architecture decisions, sending messages to people, merging PRs, deleting branches, any irreversible action\n\n## Effort principle\n- AI makes completeness cheap — when the thorough implementation costs minutes more than the shortcut, always do the thorough thing\n- Always show dual effort estimates when known: human time vs AI-assisted time\n- This reframes prioritization: features that seem expensive may be cheap with AI\n\n## Signal urgency\n- CI failures on user's own PRs = prompt urgency — surface within the hour\n- Production/hotfix/critical labels = realtime — broadcast immediately\n- PR review requests = batch urgency unless from team lead or marked urgent\n- Security P1 findings = realtime\n- AI agent PRs grouped separately in digest with shorter stale threshold (1 day vs 3)\n\n## Tech debt\n- Captured passively from conversation AND from merged PR review comments\n- PR review comments matching 'address in follow-up', 'not blocking but fix', 'TODO later', 'leaving for now' → auto-create tech-debt items\n\n## Reviews\n- Track review readiness per branch in projects/<slug>/readiness/\n- Before merge, check: code review + tests + security + QA. Surface gaps in digest.\n- Security and QA reviews can be run with /security-review and /qa-review\n- Obvious security/QA fixes are auto-applied; ambiguous ones always ask\n\n## Product\n- Feature prioritization uses evidence-based scoring: demand × 3 + impact × 2 + alignment / effort\n- Challenge assumptions — 'I think users want X' requires evidence\n- Use /product-prioritization for structured analysis\n\n## General\n- Architecture/API design decisions = high-confidence capture; debugging 'let's try X' = not a decision\n- Most developer commitments are personal tasks, not delegations — default owner=user\n- Projects tracked in projects/<slug>/project.md\n- For complex tasks, suggest /plan for structured execution\n- Weekly retro writes to context/intel/ as durable intelligence\n- Start conservative: surface everything, earn trust through feedback",
  append: false
)
```

## Step 6: Confirm

Tell the user:

> Your developer workflow is ready:
>
> **Projects:** <list of repos, each with workflow status>
>
> **Missions:**
> - **Triage** 3x weekdays (9am, 2pm, 6pm) — scans GitHub for review requests, CI failures, assignments, mentions, and tech debt from PR reviews
> - **Morning brief** 8am weekdays — overnight CI, PRs needing review, your PR statuses, today's commitments
> - **Stale PR check** 4pm weekdays — flags abandoned PRs and slow reviews
> - **Weekly retro** Friday 10am — what shipped, decisions, tech debt, patterns
> - **Tech debt review** Monday 10am — resurfaces accumulated debt
> - **Decision check** Wednesday 10am — follows up on decisions older than 7 days
>
> Per-repo workflow: issue planning, maintainer gate, PR monitor, CI fix loop, staging review, post-merge learning
>
> **Quick commands:**
> - **"show commitments"** — current status
> - **"show tech debt"** — debt backlog
> - **"add repo owner/repo"** — add another project
> - **"is this PR ready?"** — review readiness dashboard
> - **"what should we build next?"** — evidence-based prioritization
> - **`/security-review`** — run security audit on current changes
> - **`/qa-review`** — generate test plan and coverage analysis
> - **`/plan <description>`** — structured execution plan for complex tasks
> - **`/product-prioritization`** — score and rank features by demand

## Step 7: Mark setup complete

After confirming with the user that everything is in place, write the setup completion marker so this skill stops competing for the activation budget on every subsequent message:

```
memory_write(
  path: "commitments/.developer-setup-complete",
  content: "# Developer Setup Complete\n\nCompleted: <today's UTC date>\n\nRepos: <list of repo slugs>\nMaintainers: <maintainers>\nMissions installed: wf-issue-plan, wf-maintainer-gate, wf-pr-monitor, wf-ci-fix, wf-learning, plus 6 personal productivity missions (commitment-triage, commitment-digest, dev-stale-pr-check, dev-weekly-retro, dev-tech-debt-resurface, dev-decision-outcome-check)"
)
```

This is a one-time marker. The next conversational turn will not load this setup skill (the operational skills like `commitment-triage`, `tech-debt-tracker`, `github`, `github-workflow` keep activating reactively as before). To re-trigger setup (add a new repo with the wizard, re-onboard, switch maintainers), delete `commitments/.developer-setup-complete` first.
