# gmail.fetch_message_summaries

Use this for inbox triage, email digests, and "top emails" requests when message bodies are not required.

Prefer this over `gmail.list_messages` followed by many `gmail.get_message` calls because it returns ids, thread ids, sender, recipient, subject, date, snippet, labels, unread state, and a bounded body preview in one compact result.

Use `query` to narrow the mailbox before fetching summaries. Keep `max_results` small unless the user explicitly asks for a larger digest.
