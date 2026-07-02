import { QueryClientProvider } from "@tanstack/react-query";
import { createRoot } from "react-dom/client";
import { App } from "./app/app.js";
import { html } from "./lib/html.js";
import { queryClient } from "./lib/query-client.js";
import { I18nProvider } from "./lib/i18n.js";
// Only the English fallback is bundled eagerly; every other locale is
// lazy-loaded on demand by I18nProvider (see lib/i18n.js `loaders`).
import "./i18n/en.js";

// The SPA lives under the /v2 namespace: host composition mounts the
// asset router at that prefix (reborn composition's
// `mount_at_prefix("/v2")`) and BrowserRouter below the same basename,
// which renders nothing for out-of-basename locations. The gateway
// never serves this shell outside /v2, but a plain static server with
// SPA fallback can (e.g. local previews hitting /playground) — hop
// into the namespace instead of showing a blank page.
if (window.location.pathname.startsWith("/v2")) {
  createRoot(document.getElementById("v2-root")).render(html`
    <${I18nProvider}>
      <${QueryClientProvider} client=${queryClient}>
        <${App} />
      <//>
    <//>
  `);
} else {
  const { pathname, search, hash } = window.location;
  window.location.replace(`/v2${pathname === "/" ? "" : pathname}${search}${hash}`);
}
