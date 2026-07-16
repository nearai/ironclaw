# IronClaw workspace demo (static, mock-backed)

A fully static cut of the gateway webui (`crates/ironclaw_gateway/static`)
that runs the redesigned agent workspace **with no server**: every `/api/*`
and `/auth/*` request and the `/api/chat/events` SSE stream are served
in-browser by `crates/ironclaw_gateway/static/js/core/mock-backend.js`.

This base cut covers the **workspace redesign / design-system application**
(shell, sidebar, chat chrome, surfaces, billing). The chat-first onboarding
experience (consent modal, cascade connect, flow cards, landing handoff,
auto-send) lives in the stacked `achal/chat-first-onboarding` branch.

## Build & run locally

```sh
node demo/build.mjs
python3 -m http.server 8321 -d demo/dist
```

Open with `?token=demo` to skip the auth screen.

## Deploy to Vercel

```sh
node demo/build.mjs
cd demo/dist && vercel deploy --prod
```
