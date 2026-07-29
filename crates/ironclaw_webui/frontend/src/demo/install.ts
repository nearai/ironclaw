// DEMO-mode bootstrap. Loaded (dynamically) by `src/main.tsx` ONLY when the
// build carries `VITE_DEMO_MODE=1`; production builds dead-code-eliminate
// the import so none of this ships to real deployments.
//
// What it does:
//   1. Seeds a demo bearer token so the auth layer skips the login screen.
//   2. Replaces `window.fetch` with the fixture router for same-origin
//      /api/* and /auth/* paths (everything else passes through).
//   3. Replaces `EventSource`/`WebSocket` with inert, silently-open fakes
//      (fixture mutations can still push synthetic frames).

import { handleDemoRequest, registerDemoRoutes } from "./router";
import { DemoEventSource, DemoWebSocket } from "./streams";
import { coreRoutes } from "./routes/core";
import { systemRoutes } from "./routes/system";
import { workRoutes } from "./routes/work";

const DEMO_TOKEN = "demo-mode-static-token";

export function installDemoMode() {
  registerDemoRoutes([...coreRoutes, ...systemRoutes, ...workRoutes]);

  // The auth layer reads `sessionStorage.ironclaw_token`; with a token
  // present and /session mocked, the SPA boots straight into the shell.
  if (!sessionStorage.getItem("ironclaw_token")) {
    sessionStorage.setItem("ironclaw_token", DEMO_TOKEN);
  }

  const realFetch = window.fetch.bind(window);
  window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
    const demoResponse = await handleDemoRequest(input, init);
    return demoResponse ?? realFetch(input, init);
  };

  window.EventSource = DemoEventSource as unknown as typeof EventSource;
  window.WebSocket = DemoWebSocket as unknown as typeof WebSocket;

  console.info(
    "%cIronClaw DEMO MODE%c fixture data only — no backend attached",
    "background:#1d4ed8;color:#fff;padding:2px 6px;border-radius:4px;font-weight:600",
    "color:inherit"
  );
}
