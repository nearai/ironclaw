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
4. Under **Authorized redirect URIs**, click **+ Add URI** and enter your
   WebUI's Google callback URL:

   ```
   https://<your-ironclaw-host>/api/reborn/product-auth/oauth/google/callback
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

<Step title="Save the Credentials in WebUI Admin">

Open **WebUI Admin → Extension Configuration**, find **Google OAuth client
credentials**, and save the Client ID and Client Secret from Google Cloud.

The configuration is shared by Gmail, Calendar, Drive, Docs, Sheets, and
Slides. New or rotated values apply to the next OAuth operation without an SSH
session or IronClaw restart.

</Step>

</Steps>

You're ready to install any Google extension. Return to the extension page to complete the remaining steps.
