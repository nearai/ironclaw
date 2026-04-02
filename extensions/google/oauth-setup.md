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
4. Under **Authorized redirect URIs**, click **+ Add URI** and enter:

   ```
   http://127.0.0.1:9876/callback
   ```

5. Click **Create** and copy the **Client ID** and **Client Secret** shown

</Step>

<Step title="Add Test Users">

Since the app is in **Testing** mode, only explicitly added users can authorize it. Go to [**Google Auth Platform → Audience**](https://console.cloud.google.com/auth/audience), scroll down to **Test users**, and click **+ Add users**.

Add the Google account(s) that will use the extension. The app supports up to 100 test users before requiring verification.

<Info>
Only test users can complete the OAuth flow while the app is in Testing mode. If you get an "access blocked" error, make sure your account is listed here.
</Info>

</Step>

<Step title="Open the SSH Tunnel">
To complete the OAuth flow, we need to allow Google to reach the IronClaw server. Since port 9876 is only accessible from within the server, you need to open an SSH tunnel that forwards your local port 9876 to the server.

Open a new SSH session using port forwarding:

```bash
# ssh -p <SSH-PORT> -L 9876:127.0.0.1:9876 <user>@<ironclaw-server-ip>
ssh -p 15222 -L 9876:127.0.0.1:9876 liquid-zebra@agent4.near.ai
```

Keep this terminal session open while completing the OAuth flow.

<Info>
The port forwarding will remain active as long as the SSH session remains open, and automatically closes when you exit the session.
</Info>

<Tip>
Remember to whitelist the port 9876 in your server's firewall settings to allow the tunnel to work properly
</Tip>


</Step>

<Step title="Set Environment Variables">

Once connected via SSH, export your OAuth credentials as environment variables:

```bash
export GOOGLE_OAUTH_CLIENT_ID=<your-client-id>
export GOOGLE_OAUTH_CLIENT_SECRET=<your-client-secret>
```

</Step>

</Steps>

You're ready to install any Google extension. Return to the extension page to complete the remaining steps.
