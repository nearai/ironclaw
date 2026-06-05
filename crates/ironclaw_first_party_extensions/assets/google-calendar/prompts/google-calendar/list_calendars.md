Use `google-calendar.list_calendars` for read-only discovery of calendars available to the selected Google account.

No input is required. Use this before event operations when the user has not identified a target calendar; later calls may pass the returned calendar id as `calendar_id`.

This capability reads from the Google Calendar API through host HTTP egress. It requires a configured Google credential account with Calendar read scope.
