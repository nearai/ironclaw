# IronClaw Mobile

Expo/React Native companion app for IronClaw.

## Run

```bash
cd app
npm install
npm run start
```

Use a development build for native SQLCipher:

```bash
npx expo prebuild
npm run ios
# or
npm run android
```

Development builds connect to `https://agent-stg.near.ai`. Set
`IRONCLAW_APP_ENV=production` when producing a production build for
`https://agent.near.ai`.

Hosted OAuth runs through the NEAR AI account control plane. After sign-in, the
app selects the user's running hosted IronClaw instance and validates its
WebChat v2 API before saving the connection.

The staging account frontend is live, but its currently deployed hosted
IronClaw image must expose `/api/webchat/v2` for the native client to finish
connecting. The app reports an incompatible deployment instead of accepting
the frontend HTML fallback.

Use **Pair a dedicated deployment** with a scoped bearer to connect directly to
any compatible WebChat v2 backend. A short-lived pairing artifact remains the
preferred production contract; manual bearer entry is an advanced bootstrap
path.

## Implemented

- Google, Apple, GitHub, and NEAR hosted account authorization;
- hosted IronClaw instance discovery and dedicated URL connection;
- encrypted native SQLite cache using SQLCipher and a SecureStore-held key;
- offline thread, timeline, automation, and composer-draft reads;
- thread creation, text chat, and foreground response polling;
- automation list, pause, resume, rename, and delete;
- tool settings and global auto-approval control;
- iOS 16.4+ and Android 13+ native configuration.

Run the local verification suite with:

```bash
npm run typecheck
npm test
npm run export:web
npx expo-doctor
```
