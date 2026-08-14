Use `google-calendar.set_reminder` to replace reminder settings on an existing Google Calendar event.

Pass `event_id` exactly and pass the reminder configuration in `reminders`. Pass `calendar_id` when targeting a non-primary calendar; omit it to use the primary calendar.

This capability performs an external write through the Google Calendar API using host HTTP egress. It requires approval before dispatch and a configured Google credential account with Calendar events scope.
