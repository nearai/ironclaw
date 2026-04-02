---
title: "Google Calendar"
description: "Let your agent manage your Google Calendar"
---

The Google Calendar extension allows your agent to interact with your Google Calendar — creating events, checking your schedule, updating appointments, and more. It's ideal for automating scheduling tasks, setting reminders, or managing meetings directly from your agent.

---

## Setup

If you haven't set up Google OAuth yet, complete the [Google OAuth Setup](/extensions/google/oauth-setup) first.

<Steps>

<Step title="Enable the Google Calendar API">

In your Google Cloud project, navigate to **APIs & Services → Library**, search for [**Google Calendar API**](https://console.cloud.google.com/marketplace/product/google/calendar-json.googleapis.com?q=search&referrer=search), and click **Enable**.

</Step>

<Step title="Install the Extension">

```bash
ironclaw registry install google-calendar
```

</Step>

<Step title="Authorize Access">

```bash
ironclaw tool auth google-calendar
```

IronClaw will provide a URL for you to authenticate - remember to follow the [auth setup](./oauth-setup) to enable your agent to capture the callback. If possible, it will open a browser window. Once approved, the token is stored securely and refreshed automatically.

<Tip>
If you already authenticated one Google service, you still need to authenticate each additional Google extension separately.
</Tip>

</Step>

</Steps>

---

## Available Actions

- `list_calendars`: List all calendars in your Google account
- `list_events`: List upcoming events in a calendar
- `get_event`: Get details of a specific event
- `create_event`: Create a new calendar event
- `update_event`: Update an existing event (title, time, description, attendees)
- `delete_event`: Delete a calendar event
- `find_free_slots`: Find available time slots across one or more calendars
- `add_attendees`: Add attendees to an existing event
- `set_reminder`: Set a reminder for an event

---

## Example Usage

Once configured, you can ask your agent things like:

- _"Schedule a team sync for next Tuesday at 3pm for 1 hour"_
- _"What's on my calendar this week?"_
- _"Move my Friday meeting to Monday morning"_
- _"Find a free 30-minute slot for me and john@example.com this week"_
- _"Cancel all my meetings on Thursday afternoon"_

---

## Working with Multiple Calendars

If your Google account has multiple calendars (personal, work, shared), you can tell your agent which one to use:

<Tip>
Say something like: _"Add this to my Work calendar, not my personal one."_ The agent will use `list_calendars` to find the right calendar by name before creating the event.
</Tip>
