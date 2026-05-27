Use `google-calendar.find_free_slots` to inspect Google Calendar busy windows and derive open time ranges for scheduling.

Pass `time_min` and `time_max` for the search window. Pass `calendar_ids` when the user names specific calendars; otherwise the primary calendar is used.

This capability reads from the Google Calendar API through host HTTP egress. It requires a configured Google credential account with Calendar read scope.
