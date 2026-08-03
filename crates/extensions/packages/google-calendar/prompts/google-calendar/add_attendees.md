Use `google-calendar.add_attendees` to add one or more attendees to an existing Google Calendar event.

Pass `event_id` exactly and pass attendee email addresses in `attendees`. Pass `calendar_id` when targeting a non-primary calendar; omit it to use the primary calendar.

This capability performs an external write through the Google Calendar API using host HTTP egress. It requires approval before dispatch and a configured Google credential account with Calendar events scope.
