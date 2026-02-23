---
name: web-ui-test
version: 0.1.0
description: Test the IronClaw web UI using the Claude for Chrome browser extension.
activation:
  keywords:
    - test web ui
    - test the ui
    - browser test
    - chrome test
    - test skills tab
    - test chat
    - web gateway test
  patterns:
    - "test.*web.*ui"
    - "test.*browser"
    - "chrome.*extension.*test"
---

# Web UI Testing with Claude for Chrome

Use this skill when manually testing the IronClaw web gateway UI via the Claude for Chrome browser extension.

## Prerequisites

- IronClaw must be running with `GATEWAY_ENABLED=true`
- Note the gateway URL (default: `http://127.0.0.1:3000/`) and auth token
- The Claude for Chrome extension must be installed and connected

## Starting the Server

```bash
CLI_ENABLED=false GATEWAY_AUTH_TOKEN=<your-token> cargo run
```

Wait for "Agent ironclaw ready and listening" in the logs before proceeding.

## Test Checklist

### 1. Connection

- Navigate to `http://127.0.0.1:3000/?token=<token>`
- Verify "Connected" indicator in the top-right corner
- Verify all tabs are visible: Chat, Memory, Jobs, Routines, Extensions, Skills

### 2. Chat Tab

- Send a simple message (e.g., "Hello, what tools do you have?")
- Verify the LLM responds without errors
- If you see "Invalid schema for function" errors, the tool schema fix (PR #301) may not be merged yet

### 3. Skills Tab

- Click the Skills tab
- Verify "No skills installed" or a list of installed skills (no "Skills system not enabled" error)
- Search for "markdown" in the ClawHub search box
- Verify results appear with: name, version, description, relevance score, "updated X ago"
- Verify skill names are clickable links to clawhub.ai
- If search returns empty with a yellow warning banner, the registry may be unreachable

### 4. Skill Install (from search)

- Search for a skill (e.g., "markdown")
- Click "Install" on a result
- Confirm the install dialog
- Verify success toast appears
- Verify the skill appears in "Installed Skills" section

### 5. Skill Install (by URL)

- Scroll to "Install Skill by URL"
- Enter a skill name and a ClawHub download URL:
  - Name: `markdown-viewer`
  - URL: `https://wry-manatee-359.convex.site/api/v1/download?slug=markdown-viewer`
- Click Install
- Verify success toast and skill appears in installed list

### 6. Skill Remove

- Find an installed skill
- Click "Remove"
- Confirm removal
- Verify the skill disappears from the installed list

### 7. Other Tabs (smoke test)

- **Memory**: Should show the memory filesystem (may be empty)
- **Jobs**: Should show job list (may be empty)
- **Routines**: Should show routine list
- **Extensions**: Should show extension list with install options

## Cleanup

After testing, remove any test-installed skills:

```bash
rm -rf ~/.ironclaw/installed_skills/<skill-name>
```

Stop the server with Ctrl+C or by killing the process.

## Known Issues

- ClawHub registry at `clawhub.ai` is behind Vercel which blocks non-browser TLS fingerprints; the backend uses `wry-manatee-359.convex.site` directly
- Skill downloads are ZIP archives containing SKILL.md, not raw text
- The `confirm()` dialog for install may block browser automation; override with `window.confirm = () => true` in the console first
