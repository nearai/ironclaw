---
title: "Google Calendar"
description: "Let your agent manage your Google Calendar"
---

The Google Calendar extension allows your agent to interact with your Google Calendar — creating events, checking your schedule, updating appointments, and more. It's ideal for automating scheduling tasks, setting reminders, or managing meetings directly from your agent.

---

## Setup

<Steps>

<Step title="Create a Google Cloud Project">

Go to [Google Cloud Console](https://console.cloud.google.com) and create a new project (or select an existing one).

1. Click **Select a project** → **New Project**
2. Give it a name (e.g. `ironclaw-calendar`) and click **Create**

</Step>

<Step title="Enable the Google Calendar API">

With your project selected, navigate to **APIs & Services → Library**, search for **Google Calendar API**, and click **Enable**.

</Step>

<Step title="Create OAuth 2.0 Credentials">

Go to **Google Auth Platform → Clients** and create a new client:

1. Click **Create client**
2. Set **Application type** to **Web application**
3. Give it a name (e.g. `ironclaw-calendar`)
4. Under **Authorized redirect URIs**, click **+ Add URI** and enter:

   ```
   http://127.0.0.1:9876/callback
   ```

5. Click **Create** and copy the **Client ID** and **Client Secret** shown


</Step>

<Step title="Add Test Users">

Since the app is in **Testing** mode, only explicitly added users can authorize it. Go to **APIs & Services → OAuth consent screen**, scroll down to **Test users**, and click **+ Add users**.

Add the Google account(s) that will use the extension (e.g. `yourname@gmail.com`). The app supports up to 100 test users before requiring verification.

<Info>
Only test users can complete the OAuth flow while the app is in Testing mode. If you get an "access blocked" error, make sure your account is listed here.
</Info>

</Step>

<Step title="Connect to the Development Server">

The Google OAuth callback runs on the remote server at port `9876`. Since that port is not exposed publicly, you need to create an **SSH tunnel** that forwards `localhost:9876` on your machine to `127.0.0.1:9876` on the server. This way, when Google redirects to `http://127.0.0.1:9876/callback` after authorization, the request reaches the server correctly.

Open the tunnel by running:

```bash
ssh -p 15222 -L 9876:127.0.0.1:9876 solid-wolf@agent4.near.ai
```

Keep this terminal session open while using the extension.

<Info>
The `-L 9876:127.0.0.1:9876` flag is what creates the tunnel. Without it, the OAuth callback will fail because port 9876 is only accessible from within the server.
</Info>

</Step>

<Step title="Set Environment Variables">

Using the **Client ID** and **Client Secret** obtained in the previous step, export them as environment variables on the server:

```bash
export GOOGLE_OAUTH_CLIENT_ID=<your-client-id>
export GOOGLE_OAUTH_CLIENT_SECRET=<your-client-secret>
```

</Step>

<Step title="Install the Google Calendar Extension">

Install the extension by running:

```bash
ironclaw registry install google-calendar
```

</Step>

<Step title="Configure Your Credentials">

Provide IronClaw with your OAuth credentials:

```bash
ironclaw tool auth google-calendar
```

Follow the prompts to paste the contents of your `credentials.json` file or provide the path to it. IronClaw will open a browser window for you to authorize access to your calendar — once approved, the token is stored securely.

<Info>
The authorization flow only runs once. After that, IronClaw will automatically refresh the access token as needed.
</Info>

</Step>

</Steps>

---

## Available Actions

Here are some of the actions your agent can perform with the Google Calendar extension:

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
