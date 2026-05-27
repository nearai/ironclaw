Use `google-calendar.create_event` to create a new event on a Google Calendar when the user has provided the event details.

Pass `event` with the Google Calendar event fields to create. Pass `calendar_id` when targeting a non-primary calendar; omit it to use the primary calendar.

This capability performs an external write through the Google Calendar API using host HTTP egress. It requires approval before dispatch and a configured Google credential account with Calendar events scope.
