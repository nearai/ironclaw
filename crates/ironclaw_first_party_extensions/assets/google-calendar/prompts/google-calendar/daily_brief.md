# google-calendar.daily_brief

Use this for morning briefs, standup prep, and "what needs my attention today" requests.

It returns a compact agenda plus compact Gmail attention summaries. Email previews only truncate Gmail's already-short API snippets; they do not fetch full message bodies. Prefer this capability over separate calendar and Gmail calls when the user wants a curated daily overview, and use `gmail.get_message` when full email content is required.

Use `email_query` to narrow the inbox signal when the user names a sender, project, or label. Follow up with full event, email, Drive, Docs, or Sheets reads only for selected items.
