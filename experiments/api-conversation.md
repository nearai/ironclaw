# API Conversation Experiment

End-to-end walkthrough: start the stack, create a user, open a thread, have a continuous conversation over the REST API.

---

## 1. Start the local stack

```bash
make up       # build images if needed and start all containers (detached)
make logs     # follow logs — wait until you see the gateway listening on :3000
```

The gateway binds to `http://localhost:3000`. Other useful commands:

```bash
make rebuild  # after code changes: rebuild images then restart
make restart  # restart only the t3claw service (config change)
make down     # stop containers, keep DB volumes
make wipe     # full reset — deletes all volumes and data
```

---

## 2. Get the admin token

The admin token is set in `.env` as `GATEWAY_AUTH_TOKEN`. Check it:

```bash
grep GATEWAY_AUTH_TOKEN .env
```

If it was not set, the server printed a random one to stdout at startup — check `make logs`.

Set a shell variable for convenience:

```bash
ADMIN_TOKEN="<your-token-here>"
BASE="http://localhost:3000"
```

---

## 3. Create a user

```bash
curl -s -X POST "$BASE/api/admin/users" \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"display_name": "Test User", "email": "test@example.com", "role": "member"}' \
  | tee /tmp/new_user.json
```

The response includes a **one-time plaintext token** — this is the only time you'll see it:

```json
{
  "id": "550e8400-...",
  "display_name": "Test User",
  "email": "test@example.com",
  "role": "member",
  "status": "active",
  "token": "a1b2c3d4...(64 hex chars)...",
  "created_at": "2026-04-20T..."
}
```

Store the token:

```bash
USER_TOKEN=$(cat /tmp/new_user.json | python3 -m json.tool | grep '"token"' | awk -F'"' '{print $4}')
echo "User token: $USER_TOKEN"
```

All subsequent requests use `USER_TOKEN` instead of `ADMIN_TOKEN`.

---

## 4. Create a conversation thread

A thread is how you group messages into a continuous conversation. Create one:

```bash
curl -s -X POST "$BASE/api/chat/thread/new" \
  -H "Authorization: Bearer $USER_TOKEN" \
  | tee /tmp/thread.json
```

Response:

```json
{
  "id": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
  "state": "Idle",
  "turn_count": 0,
  "created_at": "2026-04-20T..."
}
```

Store the thread ID:

```bash
THREAD_ID=$(cat /tmp/thread.json | python3 -m json.tool | grep '"id"' | awk -F'"' '{print $4}')
echo "Thread: $THREAD_ID"
```

---

## 5. Open the SSE stream (in a separate terminal)

Before sending messages, open a terminal to watch the agent's responses stream in real time:

```bash
curl -N "$BASE/api/chat/events?token=$USER_TOKEN"
```

Leave this running. Each agent response, tool call, and status update will appear here as JSON lines prefixed with `data:`.

---

## 6. Send a message

```bash
curl -s -X POST "$BASE/api/chat/send" \
  -H "Authorization: Bearer $USER_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{\"content\": \"Hello! What can you help me with today?\", \"thread_id\": \"$THREAD_ID\"}"
```

This returns `202 Accepted` immediately — the message is queued to the agent loop:

```json
{ "message_id": "...", "status": "accepted" }
```

The agent processes it asynchronously. Watch your SSE terminal for `response` events.

---

## 7. Continue the conversation

Every subsequent message includes the same `thread_id`. The agent reads the full thread history each turn, so it has context of everything said before:

```bash
curl -s -X POST "$BASE/api/chat/send" \
  -H "Authorization: Bearer $USER_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{\"content\": \"Can you summarise what we just discussed?\", \"thread_id\": \"$THREAD_ID\"}"
```

Omitting `thread_id` sends the message to a default assistant thread — it still goes through the agent loop but without a specific thread context.

---

## 8. Fetch conversation history

Poll the thread history to get the full exchange so far (useful if you're not using SSE):

```bash
curl -s "$BASE/api/chat/history?thread_id=$THREAD_ID" \
  -H "Authorization: Bearer $USER_TOKEN" \
  | python3 -m json.tool
```

---

## 9. List all threads

```bash
curl -s "$BASE/api/chat/threads" \
  -H "Authorization: Bearer $USER_TOKEN" \
  | python3 -m json.tool
```

---

## How threading works

- **Thread = conversation context.** The agent loop reads the persisted message history for the thread and includes it in the LLM prompt. Each new turn is appended.
- **Create a new thread** for a fresh conversation with no prior context.
- **Reuse the same thread_id** to continue an existing one — the agent remembers everything in it.
- **No thread_id** → message goes to the default "assistant" thread (the same one the web UI main chat uses).
- Threads are **per-user** — `USER_TOKEN` only sees threads belonging to that user.
- Everything goes through the full agent harness: tool calls, safety checks, skills, hooks, etc. The API is not a raw LLM proxy (for that, use `/v1/chat/completions`).

---

## Generating a new token for an existing user (admin)

If you lose a user's token, an admin can issue a new one:

```bash
curl -s -X POST "$BASE/api/admin/users/<user-id>/token" \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

Or the user can mint their own from the web UI under Profile → API Tokens.

# DELETE LATER

hey, please run the t3n mcp to check it is working, auth yourself and check my names in my profile. my did is did:t3n:2ad9d1f0f54cd125a2ce33289e35a59fe1b0bf9f. you should have that mcp available, let me know if it isn't for some reason
