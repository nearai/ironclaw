# IronClaw onboarding demo (static, mock-backed)

A fully static cut of the gateway webui (`crates/ironclaw_gateway/static`)
that runs the complete onboarding → chat → task-creation flow **with no
server**: every `/api/*` and `/auth/*` request and the `/api/chat/events`
SSE stream are served in-browser by the mock backend in
`crates/ironclaw_gateway/static/js/core/mock-backend.js` (the single file
that holds ALL mock data).

## Build & run locally

```sh
node demo/build.mjs
python3 -m http.server 8321 -d demo/dist
```

## Deploy to Vercel

```sh
node demo/build.mjs
cd demo/dist && vercel deploy --prod
```

## Entry URLs (marketing-site handoff convention)

The marketing site deep-links to agent.near.ai with intent query params;
the demo accepts the same ones (see `captureIntentFromUrl` in
`js/core/landing.js`):

| Param | Meaning |
| --- | --- |
| `?usecase=<id>` | use-case picked on the landing page (`daily-briefing`, `keyword-monitor`, `inbox-triage`, ... — ids in `NUX_DATA.useCases`) |
| `?prompt=<text>` | free-text prompt from the hero box (pre-fills the composer) |
| `?integrations=gmail,slack` | integrations "connected" during onboarding (render as Connected) |
| `?billing=<starter\|basic\|proplus\|skipped>` | plan picked (or skipped) during onboarding |
| `?token=demo` | skips the auth screen (prototype-only handoff auth) |

Example — the full onboarding handoff in one URL:

```
/?usecase=daily-briefing&integrations=gmail,google_calendar&billing=skipped&token=demo
```

Landing without `?token=` shows the pre-auth landing (use-case gallery +
carried-intent banner); any token value works in demo mode.
