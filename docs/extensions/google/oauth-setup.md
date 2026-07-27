---
title: "OAuth Setup"
description: "One-time setup for any Google extension in IronClaw"
---

All Google extensions share the same OAuth 2.0 setup. Complete these steps once — you can reuse the same Google Cloud project and credentials for every Google extension you install.

---

<Steps>

<Step title="Create a Google Cloud Project">

Go to [Google Cloud Console](https://console.cloud.google.com) and create a new project (or select an existing one).

1. Click **Select a project** → **New Project**
2. Give it a name (e.g. `ironclaw`) and click **Create**

</Step>

<Step title="Create OAuth 2.0 Credentials">

Go to [**Google Auth Platform → Clients**](https://console.cloud.google.com/auth/clients) and create a new client:

1. Click **Create client**
2. Set **Application type** to **Web application**
3. Give it a name (e.g. `ironclaw`)
4. Under **Authorized redirect URIs**, click **+ Add URI** and enter your instance's
   callback, replacing `your-host`:

   ```
   https://your-host/api/reborn/product-auth/oauth/google/callback
   ```

5. Click **Create** and copy the **Client ID** and **Client Secret** shown

<Warning>
Google matches redirect URIs **exactly** — scheme, host, port, and path. A mismatch fails
with `redirect_uri_mismatch` before the consent screen appears.
</Warning>

<Note>
The browser returns to your running IronClaw instance, so it must be reachable at that URL
when you complete the flow. Google will not redirect to a loopback address or a host it
cannot resolve.
</Note>

</Step>

<Step title="Add Test Users">

Since the app is in **Testing** mode, only explicitly added users can authorize it. Go to [**Google Auth Platform → Audience**](https://console.cloud.google.com/auth/audience), scroll down to **Test users**, and click **+ Add users**.

Add the Google account(s) that will use the extension. The app supports up to 100 test users before requiring verification.

<Info>
Only test users can complete the OAuth flow while the app is in Testing mode. If you get an "access blocked" error, make sure your account is listed here.
</Info>

</Step>

<Step title="Give IronClaw the Credentials">

Store the client id, redirect URI, and client secret with `ironclaw config`. Run these on
the machine IronClaw runs on — over SSH if it's a remote or hosted instance.

```bash
ironclaw config set google.client_id <your-client-id>
ironclaw config set google.redirect_uri https://<your-instance-host>/api/reborn/product-auth/oauth/google/callback
ironclaw config set google.client_secret
```

<Note>
`google.client_secret` takes no value on the command line. It always prompts, with input
hidden, so the secret never lands in your shell history or the process list. The client id
and redirect URI are not secrets and are passed normally.
</Note>

Confirm what was stored:

```bash
ironclaw config get google.client_id
ironclaw config get google.redirect_uri
```

The same values can be supplied as environment variables when a service unit or container
injects them from a secret manager:

```
IRONCLAW_REBORN_GOOGLE_CLIENT_ID
IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET
IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI
```

<Warning>
Do not `export` the client secret by hand in an interactive shell. It persists in shell
history and is visible to every child process. Use `ironclaw config set
google.client_secret`, which prompts with input hidden, or inject the variable from your
platform's secret manager.
</Warning>

</Step>

<Step title="Restart So the Change Takes Effect">

`ironclaw config set` never restarts anything — it writes the value and prints:

```
  to apply: ironclaw service restart
```

A running instance keeps serving the old configuration until you restart it. Google OAuth
will keep failing until you do.

<Tabs>
  <Tab title="NEAR AI hosted instance">
    `ironclaw service` commands do **not** work on a NEAR AI hosted instance — there is no
    user service manager for them to talk to, so `service restart` fails rather than
    restarting anything.

    SSH in only to run the `ironclaw config` commands, then restart the agent from the
    [Agent Dashboard](https://agent.near.ai/). That is the only way to restart a hosted
    instance.
  </Tab>

  <Tab title="Self-hosted">
    ```bash
    ironclaw service restart
    ```

    If you're running `ironclaw serve` in the foreground instead, stop it and start it
    again.
  </Tab>
</Tabs>

</Step>

</Steps>

You're ready to install any Google extension. Return to the extension page to complete the remaining steps.
