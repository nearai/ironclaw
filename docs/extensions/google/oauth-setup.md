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
4. Under **Authorized redirect URIs**, click **+ Add URI**. Which URI you need depends on
   how you complete the OAuth flow:

   **Loopback flow** — the browser returns to a fixed local port. Used when you authorize
   from the machine running IronClaw, or through the SSH tunnel described below:

   ```
   http://127.0.0.1:9876/callback
   ```

   **Server-hosted flow** — the browser returns to your running instance. Use this for a
   deployment that is already reachable over HTTPS:

   ```
   https://your-host/api/reborn/product-auth/oauth/google/callback
   ```

   Adding both is fine, and is the simplest option if you're unsure.

5. Click **Create** and copy the **Client ID** and **Client Secret** shown

<Warning>
Google matches redirect URIs **exactly** — scheme, host, port, and path. A mismatch fails
with `redirect_uri_mismatch` before the consent screen appears. `http` for loopback and
`https` for a hosted instance are both correct; don't "fix" the loopback one to HTTPS.
</Warning>

If you use the server-hosted flow, tell IronClaw the same value so both sides agree:

```bash
export IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI=https://your-host/api/reborn/product-auth/oauth/google/callback
export IRONCLAW_REBORN_GOOGLE_CLIENT_ID=...
export IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET=...
```

These can also live under `[google]` in `config.toml` as `google.redirect_uri` and
`google.client_id`. The client secret belongs in the encrypted secret store, not the file.
See [Configuration](/capabilities/configuration).

</Step>

<Step title="Add Test Users">

Since the app is in **Testing** mode, only explicitly added users can authorize it. Go to [**Google Auth Platform → Audience**](https://console.cloud.google.com/auth/audience), scroll down to **Test users**, and click **+ Add users**.

Add the Google account(s) that will use the extension. The app supports up to 100 test users before requiring verification.

<Info>
Only test users can complete the OAuth flow while the app is in Testing mode. If you get an "access blocked" error, make sure your account is listed here.
</Info>

</Step>

<Step title="Open the SSH Tunnel">
The loopback callback listens on port 9876 *inside* the server, and your browser runs on your own machine. An SSH tunnel bridges the two by forwarding your local port 9876 to the server's loopback address.

Open a new SSH session using port forwarding:

```bash
# ssh -p <SSH-PORT> -L 9876:127.0.0.1:9876 <user>@<ironclaw-server-ip>
ssh -p 15222 -L 9876:127.0.0.1:9876 liquid-zebra@agent4.near.ai
```

Keep this terminal session open while completing the OAuth flow.

<Info>
The port forwarding will remain active as long as the SSH session remains open, and automatically closes when you exit the session.
</Info>

<Warning>
Do **not** open port 9876 in the server's firewall. Local port forwarding carries the
traffic inside the existing SSH connection, so no additional inbound port is needed —
opening it would expose the OAuth callback to the internet for no benefit. The port must
stay loopback-only.
</Warning>


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

Environment variables work too, if you'd rather set them in a service unit or container:

```bash
export IRONCLAW_REBORN_GOOGLE_CLIENT_ID=<your-client-id>
export IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET=<your-client-secret>
export IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI=https://<your-instance-host>/api/reborn/product-auth/oauth/google/callback
```

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
