Use `google-calendar.update_event` to modify an existing Google Calendar event when the event id and intended changes are known.

Pass `event_id` exactly and pass `event` with only the fields that should be updated. Pass `calendar_id` when targeting a non-primary calendar; omit it to use the primary calendar.

This capability performs an external write through the Google Calendar API using host HTTP egress. It requires approval before dispatch and a configured Google credential account with Calendar events scope.
