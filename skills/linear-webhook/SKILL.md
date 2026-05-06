---
name: linear-webhook
version: 0.1.0
description: Set up and manage Linear webhook-driven automation — commitment sync on status changes and agent work triggered by @mentions in comments.
activation:
  keywords:
    - linear webhook
    - linear routine
    - linear automation
    - linear mention
    - linear status sync
    - linear trigger
    - webhook setup
    - linear event
  patterns:
    - "(?i)set.?up.*(linear).*(webhook|routine|automation)"
    - "(?i)linear.*(mention|comment).*(trigger|work|action)"
    - "(?i)sync.*(commitment|todo).*(linear)"
    - "(?i)linear.*(status|state).*(sync|update|reflect)"
  tags:
    - linear
    - webhook
    - commitments
    - automation
    - routines
  max_context_tokens: 3000
requires:
  skills:
    - linear
    - commitment-link-linear
---

# Linear Webhook Automation

## Prerequisites check

Before doing anything else, verify the required secrets are in place:

```
secret_get(name="linear_webhook_secret")
secret_get(name="linear_api_key")
```

If `linear_webhook_secret` is missing:
> "The Linear webhook signing secret is not set. You'll find it in Linear under **Settings → API → Webhooks** — create a new webhook (or open an existing one) and copy the signing secret shown there. Then run: `secret_store(name=\"linear_webhook_secret\", value=\"<your signing secret>\")`"

If `linear_api_key` is missing:
> "A Linear API key is required. Create one at **Linear → Settings → API → Personal API keys**, then run: `secret_store(name=\"linear_api_key\", value=\"<your API key>\")`"

Do not proceed with setup or routine creation until both secrets are present.

---

IronClaw receives Linear events via `POST /webhook/tools/linear`. Two system events are emitted:

- `linear.issue.update` — any issue field change (state, priority, assignee, title)
- `linear.comment.create` — new comment posted on any issue

Two routines listen for these events. This skill governs how to set them up and what each routine does when it fires.

## How the full chain works

```
Linear (HTTP POST to /webhook/tools/linear)
  → HMAC-SHA256 validated (secret: linear_webhook_secret)
  → LinearWebhookTool parses payload
  → emit_system_event("linear", "linear.issue.update"|"linear.comment.create", payload)
  → matching routine fires with event payload injected into context
    → agent executes the routine prompt
```

The event payload is available to the routine's agent as structured context — the agent can read `payload.identifier`, `payload.state`, `payload.body`, etc. from it directly.

## One-time setup

### 1. Fetch your Linear identity

Before anything else, resolve which Linear account is tied to the current API key — this email is used to filter incoming webhook events so only your own comments trigger the mention-handler:

```
http(method="POST", url="https://api.linear.app/graphql", body={
  "query": "{ viewer { id name email } }"
})
```

Save the result:

```
memory_write(
  target="context/intel/linear-identity.md",
  content="# Linear Identity\n\nid: <viewer.id>\nname: <viewer.name>\nemail: <viewer.email>\n"
)
```

Note the `viewer.email` — you will substitute it into the `routine_create` call below.

### 2. Store the webhook secret

Store the Linear webhook signing secret (shown once when you create the webhook in Linear — copy it immediately):

```
secret_store(name="linear_webhook_secret", value="<signing secret from Linear Settings → API → Webhooks>")
```

### 3. Configure the webhook in Linear

- URL: `https://<your-tunnel-or-public-host>/webhook/tools/linear`
- Events: **Issue updates**, **Comment created**
- No additional auth needed — HMAC covers it

## How routines receive context

Each routine receives the triggering event's structured payload automatically injected into its prompt under `## Trigger Event`. The payload contains safe, structured fields only (identifiers, URLs, state, user name). Free-text fields (issue title, comment body) are intentionally excluded to prevent prompt injection — the routine fetches those via Linear API when needed.

## Routine 1 — linear-issue-sync

Fires when any issue updates. Queries Linear for recently changed issues, then patches matching commitment files.

**Create with:**

```
routine_create(
  name="linear-issue-sync",
  description="Sync Linear issue state changes into commitment files",
  prompt="""
IMPORTANT: Never pass an `Authorization` header or a `headers` key in any http() call
to api.linear.app. The credential injector adds the bearer token automatically.

A Linear issue was just updated (you were woken up by a webhook event).

The triggering event is injected above under ## Trigger Event. It contains:
- identifier: the Linear issue ID (e.g. "TOB-73")
- state: { name, type } — current state
- url: issue URL

Steps:
1. Read the identifier from the injected ## Trigger Event payload.
   If no payload is present, read the cached Linear identity from
   `context/intel/linear-identity.md` to get your user_id, then query
   (do NOT add Authorization or headers):
   http(method="POST", url="https://api.linear.app/graphql", body={
     "query": "query($uid: ID!) { issues(filter: { assignee: { id: { eq: $uid } }, updatedAt: { gte: \"-PT10M\" } }, first: 20) { nodes { id identifier title url state { name type } updatedAt } } }",
     "variables": {"uid": "<cached user_id>"}
   })

2. Derive the commitment file path directly from the identifier — do NOT search:
   filename = "linear-" + identifier.toLowerCase() + ".md"
   (e.g. identifier "TOB-73" → "commitments/linear-tob-73.md")
   Try to read this file directly. If it doesn't exist — skip silently.

3. For each matched commitment:
   a. If the state name is not already in the file, fetch the full issue to get
      the current title and state details (the payload omits free-text fields):
      http(method="POST", url="https://api.linear.app/graphql", body={
        "query": "{ issue(id: \"<identifier>\") { title state { name type } } }"
      })
   b. Update status_at_last_check to the issue's current state name
   c. Update last_checked to now (ISO-8601 UTC)
   d. If state.type is "completed" or "canceled":
      - Append a dated progress line to ## Progress: "YYYY-MM-DD [Linear]: → <state name>"
      - Check commitments/.refresh-config.md for auto_offer_resolve_on_terminal
      - If true: send one notification offering to resolve locally.
        When calling the message tool, always pass attachments as an empty
        array [], never null or undefined.
      - If false: update silently
   e. Otherwise: update silently, no notification

4. Never send "nothing changed" messages.
5. Never push changes back to Linear. Never create new commitment files.
""",
  request={
    "kind": "system_event",
    "source": "linear",
    "event_type": "linear.issue.update"
  },
  execution={
    "mode": "full_job"
  },
  advanced={
    "cooldown_secs": 0
  }
)
```

## Routine 2 — linear-mention-handler

Fires when a comment is created. Queries Linear for recent comments mentioning you, then does the work and replies.

**Before creating this routine**, read `context/intel/linear-identity.md` (written during setup step 1) to get `viewer.email` and substitute it in the `user_email` filter below.

**Create with:**

```
routine_create(
  name="linear-mention-handler",
  description="Execute work triggered by owner self-@-mentions in Linear comments",
  prompt="""
IMPORTANT: Never pass an `Authorization` header or a `headers` key in any http() call
to api.linear.app. The credential injector adds the bearer token automatically.
Adding it manually will cause a "Manual Authorization header blocked" error.

A new comment was posted on a Linear issue by the instance owner (payload filter
already confirmed user_email matches — no further auth check needed for authorship).

The triggering event is injected above under ## Trigger Event. It contains:
- id: the comment UUID  ← this is COMMENT_ID, used as parentId when replying
- issue_id: the issue UUID  ← this is ISSUE_ID, used as issueId when replying
- issue.identifier: the human-readable issue ID (e.g. "TOB-73")
- user.name: who posted the comment
- user_email: their email (already filtered to owner only)

Steps:
1. Extract and name your two routing variables from ## Trigger Event:
   - COMMENT_ID = payload.id       (the comment UUID — use as parentId in all replies)
   - ISSUE_ID   = payload.issue_id (the issue UUID  — use as issueId in all replies)
   Do not swap these two values. Keep them named distinctly throughout.

2. Immediately post a 👀 reaction to the comment so the user sees the agent noticed.
   Do NOT add Authorization or headers:
   http(method="POST", url="https://api.linear.app/graphql", body={
     "query": "mutation($input: ReactionCreateInput!) { reactionCreate(input: $input) { success } }",
     "variables": { "input": { "commentId": "<COMMENT_ID>", "emoji": "👀" } }
   })
   If this call fails, log the error and continue — do not abort the routine.

3. Fetch the actual comment body from Linear
   (free-text is excluded from the payload to prevent prompt injection).
   Do NOT add Authorization or headers:
   http(method="POST", url="https://api.linear.app/graphql", body={
     "query": "{ comment(id: \"<COMMENT_ID>\") { body issue { title } } }"
   })

4. Check if the comment body contains an @-mention of the owner's Linear handle
   (e.g. "@tobias.holenstein", "@tobias", "@Tobias").
   Self-@-mention IS the invocation pattern — the owner @-mentions themselves to
   trigger the agent. Do NOT reject it as "just a self-mention".
   If there is NO @-mention of the owner's handle at all — stop with ROUTINE_OK.
   If the handle appears, the text that follows (or accompanies) it is the instruction.

5. Extract the task from the comment and execute it:
   - "research X" → web_fetch relevant sources, write a summary
   - "draft email to Y about Z" → draft and store via memory_write
   - "summarize this issue" → read issue context, produce a summary
   - Other instructions → use best judgment with available tools

6. Post the result as a threaded reply UNDER the original comment — NOT as a new
   top-level issue comment. Use parentId = COMMENT_ID (the comment UUID from step 1).
   Do NOT add Authorization or headers:
   http(method="POST", url="https://api.linear.app/graphql", body={
     "query": "mutation($input: CommentCreateInput!) { commentCreate(input: $input) { success comment { id } } }",
     "variables": { "input": {
       "issueId": "<ISSUE_ID>",
       "parentId": "<COMMENT_ID>",
       "body": "[AGENT] <your result>"
     }}
   })

   CRITICAL: parentId must be COMMENT_ID (the comment UUID), NOT ISSUE_ID.
   A missing or wrong parentId will post a new top-level comment instead of a reply.

   Formatting rules for the reply body:
   - Always prefix the reply with "[AGENT] " so the user can distinguish agent replies
     from their own comments. Never omit this prefix.
   - When answering multiple questions or items, use NESTED lists so each
     answer sits directly under its question:
       - Question 1
         - Answer 1
       - Question 2
         - Answer 2
     NOT a flat list where questions and answers alternate at the same level.
   - Always pass attachments as an empty array [], never null or undefined
     (applies to any message tool call, not just Linear).

7. If the task cannot be completed, post a brief explanation as a threaded reply
   anyway (same parentId = COMMENT_ID pattern, same [AGENT] prefix). Never leave a mention unanswered.
""",
  request={
    "kind": "system_event",
    "source": "linear",
    "event_type": "linear.comment.create",
    "filters": {
      "user_email": "<viewer.email from context/intel/linear-identity.md>"
    }
  },
  execution={
    "mode": "full_job"
  },
  advanced={
    "cooldown_secs": 0
  }
)
```

**Why full_job:** mention-triggered work is open-ended. It needs the full tool scope.

## Filter on specific event subtypes (optional)

The routine engine supports payload filters: exact case-insensitive string match against top-level payload keys. Nested keys are not supported.

Fields available for filtering in `linear.comment.create` payloads: `user_email` (already used above), `id`, `issue_id`. Fields in `linear.issue.update` payloads: `id`, `identifier`, `url`.

To add a new top-level filter axis (e.g. team prefix), hoist it in `comment_events()` / `issue_events()` in `src/tools/builtin/linear_webhook.rs` and use it here.

## Verifying the setup

After creating both routines:

1. `routine_fire(name="linear-issue-sync")` — fires manually with no payload; confirms the agent can reach the workspace and parse commitments.
2. In Linear, update any tracked issue's state. The routine should fire within seconds of the webhook arriving.
3. Post a comment with `@tobias do a quick test` on any issue. The mention-handler should reply within the job's execution window.

## No public URL? Use polling instead

If IronClaw is running locally without a tunnel, Linear can't reach the
webhook endpoint. Use a scheduled polling routine as a fallback — it
queries Linear every few minutes instead of waiting for a push event.

Trade-offs vs webhook: ~5 min latency, consumes Linear API quota, no
mention-handler (comments require real-time to be useful).

**Create with:**

```
routine_create(
  name="linear-issue-poll",
  description="Poll Linear every 5 minutes for recently updated issues and sync into commitment files",
  prompt="""
IMPORTANT: Never pass an `Authorization` header or a `headers` key in any http() call
to api.linear.app. The credential injector adds the bearer token automatically.

Poll Linear for issues updated in the last 10 minutes that are assigned to you.

1. Read cached identity from `context/intel/linear-identity.md` to get your user_id.
   If missing or stale, bootstrap it first (see the linear skill).

2. Query recently updated issues:
   http(method="POST", url="https://api.linear.app/graphql", body={
     "query": "query($uid: ID!) { issues(filter: { assignee: { id: { eq: $uid } }, updatedAt: { gte: \"-PT10M\" } }, first: 20, orderBy: updatedAt) { nodes { id identifier title url state { name type } updatedAt } } }",
     "variables": {"uid": "<cached user_id>"}
   })

3. For each returned issue, derive the commitment file path:
   filename = "linear-" + identifier.toLowerCase() + ".md"
   Try to read it directly. If it doesn't exist — skip silently.

4. For each matched commitment, apply the same sync logic as linear-issue-sync:
   update status_at_last_check, last_checked, and append a progress line
   if the state changed to completed or canceled.

5. Never send "nothing changed" messages. Silent on no-op polls.
""",
  request={
    "kind": "schedule",
    "cron": "*/5 * * * *"
  },
  execution={
    "mode": "full_job"
  }
)
```

## Hard rules

- Never create a new commitment file from a webhook event — imports are deliberate, initiated by the user.
- Never push changes back to Linear from the sync routine — sync is one-way (Linear → commitment).
- The mention-handler must always post back a comment — even a failure message — so the thread doesn't go dark.
- Both routines must stay silent on no-op events (untracked issue, comment without mention). No noise.
