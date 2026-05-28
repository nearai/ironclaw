Use `google-calendar.list_events` for read-only retrieval of events on a Google Calendar.

Pass `calendar_id` when the user names a specific calendar; omit it to use the primary calendar. Use `time_min`, `time_max`, `page_token`, and `max_results` only when narrowing a date range or paginating results.

This capability reads from the Google Calendar API through host HTTP egress. It requires a configured Google credential account with Calendar read scope.
