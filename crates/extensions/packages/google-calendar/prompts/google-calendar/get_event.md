Use `google-calendar.get_event` for read-only retrieval of one Google Calendar event when the event id is known.

Pass `event_id` exactly. Pass `calendar_id` when the user identified a non-primary calendar; omit it to use the primary calendar.

This capability reads from the Google Calendar API through host HTTP egress. It requires a configured Google credential account with Calendar read scope.
