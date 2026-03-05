---
name: local-test
version: 0.1.0
description: Build, run, and test IronClaw locally using Docker containers and Chrome MCP browser automation.
activation:
  keywords:
    - test locally
    - local test
    - docker test
    - test my changes
    - test in docker
    - test web gateway
    - spin up test
    - test container
  patterns:
    - "test.*local"
    - "docker.*test"
    - "spin.*up.*test"
    - "test.*changes.*docker"
  max_context_tokens: 3000
---

# Local Testing with Docker + Chrome MCP

Use this skill to build, run, and test IronClaw web gateway changes locally using `Dockerfile.test` and Chrome MCP browser automation tools.

## Quick Start

```bash
# Build the test image (libsql-only, no PostgreSQL needed)
docker build --platform linux/amd64 -f Dockerfile.test -t ironclaw-test .

# Run on port 3003 (default)
docker run --rm -p 3003:3003 \
  -e ONBOARD_COMPLETED=true \
  -e CLI_ENABLED=false \
  -e NEARAI_API_KEY=<key> \
  ironclaw-test

# Open in browser
# http://localhost:3003/?token=test
```

## Building the Image

The test Dockerfile uses a two-stage build: Rust compilation with `--features libsql` (no PostgreSQL dependency), then a minimal Debian runtime image.

```bash
docker build --platform linux/amd64 -f Dockerfile.test -t ironclaw-test .
```

Build takes ~5-10 minutes on first run (cached subsequent builds are faster). The `--platform linux/amd64` flag avoids QEMU warnings on Apple Silicon but can be omitted if targeting native architecture.

## Running Containers

### Required Environment Variables

| Variable | Purpose | Default in Dockerfile |
|----------|---------|----------------------|
| `ONBOARD_COMPLETED=true` | Skip onboarding wizard (exits immediately otherwise) | not set |
| `CLI_ENABLED=false` | Disable TUI/REPL (causes EOF shutdown otherwise) | not set |

### LLM Backend Configuration

Pick ONE of these configurations:

**NEAR AI (API key mode):**
```bash
docker run --rm -p 3003:3003 \
  -e ONBOARD_COMPLETED=true \
  -e CLI_ENABLED=false \
  -e NEARAI_API_KEY=<your-key> \
  ironclaw-test
```

**NEAR AI (session token mode):**
```bash
docker run --rm -p 3003:3003 \
  -e ONBOARD_COMPLETED=true \
  -e CLI_ENABLED=false \
  -e NEARAI_SESSION_TOKEN=<sess_xxx> \
  -e NEARAI_BASE_URL=https://private.near.ai \
  ironclaw-test
```

**OpenAI:**
```bash
docker run --rm -p 3003:3003 \
  -e ONBOARD_COMPLETED=true \
  -e CLI_ENABLED=false \
  -e LLM_BACKEND=openai \
  -e OPENAI_API_KEY=<your-key> \
  ironclaw-test
```

**Anthropic:**
```bash
docker run --rm -p 3003:3003 \
  -e ONBOARD_COMPLETED=true \
  -e CLI_ENABLED=false \
  -e LLM_BACKEND=anthropic \
  -e ANTHROPIC_API_KEY=<your-key> \
  ironclaw-test
```

**Dummy run (no LLM, just test the UI loads):**
```bash
docker run --rm -p 3003:3003 \
  -e ONBOARD_COMPLETED=true \
  -e CLI_ENABLED=false \
  -e NEARAI_API_KEY=dummy \
  ironclaw-test
```

### Common Overrides

| Variable | Purpose | Example |
|----------|---------|---------|
| `GATEWAY_PORT` | Change the listen port | `3003` (default) |
| `GATEWAY_AUTH_TOKEN` | Auth token for API | `test` (default) |
| `NEARAI_MODEL` | Override LLM model | `claude-3-5-sonnet-20241022` |
| `RUST_LOG` | Logging verbosity | `ironclaw=debug` |
| `ROUTINES_ENABLED` | Enable routines | `true`/`false` |
| `SKILLS_ENABLED` | Enable skills system | `true` (default) |

### Multi-Instance Testing

Run multiple containers on different host ports:

```bash
docker run --rm -d --name ic-test-a -p 3003:3003 -e ONBOARD_COMPLETED=true -e CLI_ENABLED=false -e NEARAI_API_KEY=dummy ironclaw-test
docker run --rm -d --name ic-test-b -p 3004:3003 -e ONBOARD_COMPLETED=true -e CLI_ENABLED=false -e NEARAI_API_KEY=dummy ironclaw-test
```

## Chrome MCP Testing Workflow

Use the Claude for Chrome browser automation tools to test the web UI.

### Step 1: Get Browser Context

```
mcp__claude-in-chrome__tabs_context_mcp
```

Always start here to see current tabs and get fresh tab IDs.

### Step 2: Open the Gateway

```
mcp__claude-in-chrome__tabs_create_mcp  url=http://localhost:3003/?token=test
```

### Step 3: Verify the Page

```
mcp__claude-in-chrome__read_page
```

Check for:
- "Connected" indicator in top-right
- All tabs visible: Chat, Memory, Jobs, Routines, Extensions, Skills

### Step 4: Take Screenshots

```
mcp__claude-in-chrome__computer  action=screenshot
```

### Step 5: Test Mobile Viewport

```
mcp__claude-in-chrome__resize_window  width=375  height=812
mcp__claude-in-chrome__computer  action=screenshot
```

Reset to desktop:
```
mcp__claude-in-chrome__resize_window  width=1280  height=800
```

### Step 6: Run JavaScript Checks

```
mcp__claude-in-chrome__javascript_tool  script="document.querySelector('.connection-status')?.textContent"
```

### Step 7: Test Interactions

Click tabs, send messages, search skills — use `computer` tool with `action=click` and coordinate-based clicks, or use `find` + `form_input` for text entry.

## Cleanup

```bash
# Stop a specific container
docker stop ic-test-a

# Stop all test containers
docker ps --filter ancestor=ironclaw-test -q | xargs -r docker stop

# Remove the test image
docker rmi ironclaw-test
```

## Troubleshooting

### Container exits immediately
- **Missing `ONBOARD_COMPLETED=true`**: The onboarding wizard tries to read stdin, gets EOF, and exits.
- **Missing `CLI_ENABLED=false`**: The REPL channel reads stdin, gets EOF, and shuts down the agent.

### "Model not found" or LLM errors
- Check that your API key/token is valid and the model name is correct.
- For NEAR AI session token mode, you also need `NEARAI_BASE_URL=https://private.near.ai`.

### Platform mismatch warnings on Apple Silicon
- The `--platform linux/amd64` flag causes QEMU emulation warnings — these are harmless.
- Alternatively, omit the flag and build natively if your dependencies support ARM64.

### Port already in use
- The dev server defaults to port 3001; the test Dockerfile defaults to 3003 to avoid conflicts.
- Use a different host port: `-p 3005:3003`.

### Cannot connect from browser
- Verify `GATEWAY_HOST=0.0.0.0` (set by default in Dockerfile).
- Check the container logs: `docker logs <container-id>`.
- Make sure you include the token query param: `?token=test`.
