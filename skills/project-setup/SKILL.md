---
name: project-setup
version: 0.1.0
description: Add a GitHub repository as a tracked project — creates workspace entity, installs workflow automation missions, and includes in dev brief scope.
activation:
  keywords:
    - add repo
    - add repository
    - add project
    - new repo
    - new project
    - track repo
    - setup repo
    - project setup
    - setup workflow
  patterns:
    - "(?i)(add|track|setup|install|enable) (repo|repository|project)\\s"
    - "(?i)add .+/.+ to my workflow"
    - "(?i)set ?up (workflow|automation) for .+/.+"
  tags:
    - developer
    - github
    - setup
  max_context_tokens: 1500
requires:
  skills:
    - github
    - github-workflow
---

# Project Setup

Add a new GitHub repository as a tracked project in the workspace and install workflow automation.

## Step 1: Parse the repo

Extract `owner/repo` from the user's message. If ambiguous, ask. Validate via GitHub API: `http(method="GET", url="https://api.github.com/repos/{owner}/{repo}")`. If 404 or no access, tell the user.

## Step 2: Quick setup (3 questions, no timezone)

1. Who are the maintainers/reviewers? (default: the user's GitHub handle)
2. Do you use a staging branch? What's it called? (default: no staging; use `staging` if yes)
3. Any AI agents creating PRs? (e.g. Dependabot, Copilot, internal bots) (default: none)

Read `main_branch` from the API response's `default_branch` field.

## Step 3: Create project in workspace

Write `projects/<owner>-<repo>/project.md`:

```
---
type: project
repo: <owner/repo>
added_at: <today YYYY-MM-DD>
maintainers: [<handles>]
main_branch: <branch from API>
staging_branch: <branch or null>
ai_agent_authors: [<bot handles>]
workflow_installed: false
---
# <owner/repo>
<description from API response>

## Workflow missions
(populated after install)
```

Write `projects/<owner>-<repo>/notes.md`:
```
# Notes: <owner/repo>
Developer notes for this project. Searchable via memory_search.
```

## Step 4: Install workflow missions

Follow the `github-workflow` skill's install procedure. For each of the 6 mission templates in `github-workflow/references/workflow-routines.md`:

1. Replace `{{repository}}` with `owner/repo`
2. Replace `{{slug}}` with `<owner>-<repo>`
3. Replace `{{maintainers}}` with the maintainer list
4. Replace `{{main_branch}}` and `{{staging_branch}}` with the configured branches
5. Replace `{{batch_interval_hours}}` with `8` (default)
6. Call `mission_create` for each (see `github-workflow/references/workflow-routines.md`)

If `staging_branch` is null, skip `wf-staging-review-<slug>`.

After installing, update the project file:
- Set `workflow_installed: true`
- List the installed mission names under `## Workflow missions`

## Step 5: Confirm

```
Added <owner/repo> as a tracked project:
- Project file: projects/<owner>-<repo>/project.md
- Workflow missions installed: <list>
- Maintainers: <list>
- Main branch: <branch>, Staging: <branch or "none">
- This repo is now included in your morning brief and triage scans

Say "add repo <another/repo>" to add more, or "remove repo <owner/repo>" to uninstall.
```
