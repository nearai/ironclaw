# Mobile app to TestFlight

This guide turns the Expo app in `app/` into an iOS build distributed through
TestFlight. The app uses SQLCipher and therefore requires an Expo development
or production build; Expo Go is not sufficient.

## What you need once

- An Apple Developer Program membership with access to App Store Connect.
- An Expo account and the EAS CLI:

  ```bash
  npm install --global eas-cli
  eas login
  ```

- The Apple team ID, App Store Connect app record, and permission to create
  signing certificates/provisioning profiles. EAS can manage the certificates
  interactively, or an administrator can provide them.
- A real device registered in TestFlight. The iOS simulator cannot install a
  TestFlight build.

## Create the App Store Connect record

Create a new iOS app in App Store Connect with bundle ID `ai.near.ironclaw`.
The production configuration in `app/app.config.ts` uses that identifier. For
an internal staging TestFlight app, use a separate App Store Connect record and
bundle ID such as `ai.near.ironclaw.staging`; this avoids mixing staging data
and production installs.

Before submitting, fill in the app name, subtitle, privacy policy URL,
category, age rating, support URL, screenshots, and export-compliance answers.
TestFlight internal testing can start before full App Review, but the app still
needs the required metadata for external testing.

## Configure EAS

From the repository root, initialize the project once:

```bash
cd app
eas init
eas build:configure
```

Commit the generated EAS project ID if the CLI adds one to the app config.
Create or update `app/eas.json` with profiles that match the app config:

```json
{
  "build": {
    "staging": {
      "distribution": "store",
      "channel": "staging",
      "autoIncrement": true,
      "env": { "IRONCLAW_APP_ENV": "staging" }
    },
    "production": {
      "distribution": "store",
      "channel": "production",
      "autoIncrement": true,
      "env": { "IRONCLAW_APP_ENV": "production" }
    }
  },
  "submit": {
    "production": {
      "ios": { "ascAppId": "<APP_STORE_CONNECT_APP_ID>" }
    }
  }
}
```

Use the staging profile for the current hosted backend (`agent-stg.near.ai`).
Use production only after the hosted production service and OAuth redirect
configuration are ready. `app/app.config.ts` derives the app name, URL scheme,
bundle ID, and hosted origin from `IRONCLAW_APP_ENV`.

If the staging app must use the production bundle ID for a temporary test, do
not silently change the profile. Change the App Store Connect record and bundle
ID intentionally, then verify that only one installed app owns that identifier.

## Verify before building

Run the same checks used in development:

```bash
cd app
npm ci
npm run typecheck
npm test
npm run export:web
npx expo-doctor
```

Then verify a native build on the iOS simulator or a development device:

```bash
npx expo prebuild --no-install
npm run ios
```

On a physical device, test at least:

- Google, Apple, GitHub, and NEAR sign-in;
- hosted staging discovery and connection;
- pairing a dedicated deployment;
- thread creation, Markdown responses, collapsed tool activity, retry, and
  ⌘/Ctrl+Enter on an external keyboard;
- automation navigation and settings changes;
- relaunching offline and reading cached threads;
- logout, account switching, and expired-session recovery.

Never put bearer tokens, OAuth client secrets, or local deployment credentials
in `app.config.ts`, `eas.json`, source control, or an Expo public environment
variable. The app receives session credentials through the hosted auth/pairing
flow.

## Build the TestFlight archive

For the current staging beta:

```bash
cd app
IRONCLAW_APP_ENV=staging eas build --platform ios --profile staging
```

The first build prompts EAS to create or select Apple signing credentials.
Review the bundle identifier carefully before approving. The build runs in the
EAS cloud and produces an `.ipa` plus a build page.

For production:

```bash
IRONCLAW_APP_ENV=production eas build --platform ios --profile production
```

The app version is currently `0.1.0`; `autoIncrement: true` increments the
build number for each EAS build. Change the marketing version in
`app/app.config.ts` when shipping a new user-visible release.

## Submit to TestFlight

Submit the completed build directly:

```bash
cd app
eas submit --platform ios --profile production
```

For staging, use the equivalent staging submit profile after adding its
`ascAppId` to `eas.json`:

```bash
eas submit --platform ios --profile staging
```

If the submit profile is not configured, EAS asks for the App Store Connect
credentials or an API key. Prefer an App Store Connect API key owned by the
release team for CI; do not commit the `.p8` file or its private key.

After upload, open App Store Connect → the app → TestFlight. Wait for processing
to finish, answer any export-compliance prompt, then add internal testers. For
external testers, create a test group, provide review notes and a demo account
if needed, and submit the build for Beta App Review.

## Release checklist

- [ ] Correct `IRONCLAW_APP_ENV` and hosted origin verified in the built app.
- [ ] Correct bundle ID and App Store Connect app selected.
- [ ] No secrets or PII in the repository or EAS environment.
- [ ] Typecheck, unit tests, Expo export, and Expo Doctor pass.
- [ ] Physical-device smoke test completed.
- [ ] Offline cache and logout/session recovery tested.
- [ ] OAuth providers and redirect URLs allow the selected iOS scheme.
- [ ] TestFlight build processed and export compliance completed.
- [ ] Internal testers receive the staging or production build intentionally.

## Troubleshooting

- **Missing provisioning profile**: rerun `eas build`, select the correct Apple
  team, and allow EAS to repair credentials.
- **Wrong hosted service**: inspect `IRONCLAW_APP_ENV`; staging resolves to
  `https://agent-stg.near.ai` and production to `https://agent.near.ai`.
- **OAuth returns to the browser**: verify the profile's scheme (`ironclaw`
  or `ironclaw-staging`) is registered with each provider and rebuild after
  changing it.
- **Cannot connect**: confirm the deployment exposes `/api/webchat/v2`, then
  use the app's dedicated-deployment pairing flow.
- **TestFlight install unavailable**: confirm processing finished, the tester
  accepted the invitation, and the device runs iOS 16.4 or newer.
