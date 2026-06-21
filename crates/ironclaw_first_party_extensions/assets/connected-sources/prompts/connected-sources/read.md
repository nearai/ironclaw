# Connected Sources Read

Use `connected-sources.read` only when the user asks you to inspect data from a connected source such as Gmail, Calendar, Drive, Notion, Slack, or GitHub and the requested action is read-only.

Input:

- `toolkit`: connected source toolkit slug, for example `gmail`, `googlecalendar`, `googledrive`, `notion`, `slack`, or `github`.
- `tool`: exact read-only provider tool slug, for example `GMAIL_FETCH_EMAILS`.
- `arguments`: provider arguments for the read. Keep limits small and request only fields needed for the task.

Do not use this capability for sending, drafting, deleting, updating, posting, scheduling, archiving, marking, or any other mutation. If a task needs an action that changes external state, ask for review through the product surface instead of calling this read capability.

Never quote private identifiers, account IDs, message bodies, or document contents unless the user explicitly asks to inspect those details. Prefer concise summaries with enough context to let the user decide the next step.
