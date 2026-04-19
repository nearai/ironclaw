# Gotify Morning Routine Prompt

Use this prompt for an IronClaw scheduled routine that sends a morning status
report through the `gotify` tool and returns the same text as backup output.

## Important Setup Note

`gotify` is installed as a WASM tool, not as an IronClaw channel. Do not set the
routine notification channel to `gotify` unless a separate Gotify channel has
been registered.

The routine should call the `gotify` tool from inside the prompt.

## Routine Prompt

```text
You are running as a scheduled IronClaw routine.

Goal:
Send a morning status report to Gotify, and also return the same report as your final routine output.

Rules:
- Always produce a final plain-text message.
- Do not return empty content.
- Do not use markdown tables.
- Keep the message concise but complete.
- If any tool call fails, still return the composed message as backup.
- If routine/job data cannot be gathered, use the fallback message.

Step 1: Gather status data.
Use available tools to determine:
- Current UTC time.
- Active job count.
- List of routines, including name, enabled/disabled state, next_fire_at, and consecutive_failures.
- Whether any routines have consecutive_failures greater than 0.
- Whether any routines appear overdue or blocked.

Step 2: Compose MESSAGE as plain text.

MESSAGE format:

Good morning, sun! 🐴
Time (UTC): <YYYY-MM-DD HH:MM>
Status: <All systems normal OR ATTENTION REQUIRED>
Active jobs: <count or unknown>

Routines:
<routine name> | <enabled/disabled> | next: <next_fire_at or none> | failures: <n>
<routine name> | <enabled/disabled> | next: <next_fire_at or none> | failures: <n>

Attention:
<short bullet-style plain text lines for failures, overdue routines, blocked routines, or "None">

If any required status data is unavailable, use this fallback MESSAGE exactly:

Good morning, sun! 🐴
Time (UTC): <current UTC time if available, otherwise unknown>
Status: All quiet. No updates.
Active jobs: unknown

Routines:
unavailable

Attention:
None

Step 3: Send MESSAGE through the Gotify tool.
Call the `gotify` tool with MESSAGE as the notification body/message.

Use a clear title if the tool supports one:
IronClaw Morning Status

If the tool supports priority, use:
5 if Status is ATTENTION REQUIRED
3 otherwise

Step 4: Final output.
Return MESSAGE exactly as your final routine output, even if the Gotify tool call succeeds.
```

## Operational Checklist

1. Keep `use_tools` enabled for the routine.
2. Do not set `notify_channel` to `gotify`.
3. Use the prompt above as the routine prompt.
4. Run the routine manually once and verify the final output is non-empty.
5. If Gotify delivery fails but the final output is present, the routine prompt is working and the Gotify tool/auth path needs separate checking.

