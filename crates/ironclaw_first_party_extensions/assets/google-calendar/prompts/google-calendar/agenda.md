# google-calendar.agenda

Use this for compact calendar context: today's schedule, tomorrow's meetings, this week's agenda, or the next upcoming events.

Prefer this over `google-calendar.list_events` when the user needs an answer rather than raw event payloads. It returns merged, sorted, bounded events with attendee summaries and description previews.

Use `include_all_calendars` only when the user asks across calendars or primary calendar context is insufficient.
