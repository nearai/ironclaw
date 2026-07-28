import "./styles/app.css";
import { QueryClientProvider } from "@tanstack/react-query";
import { createRoot } from "react-dom/client";
import { App } from "./app/app";
import { queryClient } from "./lib/query-client";
import { I18nProvider } from "./lib/i18n";
// Only the English fallback is bundled eagerly; every other locale is
// lazy-loaded on demand by I18nProvider (see lib/i18n.tsx `loaders`).
import "./i18n/en";

// WebUI now mounts at the site root (reborn composition no longer uses
// mount_at_prefix("/v2"); BrowserRouter has no basename). Bookmarks and
// static demos may still hit /v2/... — strip that legacy prefix before
// the router resolves, so /v2/playground lands on /playground instead of
// the catch-all → authenticated app shell.
const { pathname, search, hash } = window.location;
if (pathname === "/v2" || pathname.startsWith("/v2/")) {
  const stripped = pathname === "/v2" ? "/" : pathname.slice("/v2".length) || "/";
  window.history.replaceState(null, "", `${stripped}${search}${hash}`);
}

async function bootstrap() {
  // Hosted workspace demo: a static deploy built with VITE_IRONCLAW_DEMO=1
  // installs an in-browser mock backend before the first render so the
  // authenticated workspace works with no server (see src/demo/). The
  // condition is a build-time constant — production builds tree-shake the
  // demo chunk away entirely.
  if (import.meta.env.VITE_IRONCLAW_DEMO === "1") {
    const { installDemoBackend } = await import("./demo/mock-backend");
    installDemoBackend();
  }

  createRoot(document.getElementById("v2-root")).render((
    <I18nProvider>
      <QueryClientProvider client={queryClient}>
        <App />
      </QueryClientProvider>
    </I18nProvider>
  ));
}

void bootstrap();
