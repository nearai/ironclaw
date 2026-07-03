import { QueryClientProvider } from "@tanstack/react-query";
import { createRoot } from "react-dom/client";
import { App } from "./app/app.js";
import { html } from "./lib/html.js";
import { queryClient } from "./lib/query-client.js";
import { I18nProvider } from "./lib/i18n.js";
// Only the English fallback is bundled eagerly; every other locale is
// lazy-loaded on demand by I18nProvider (see lib/i18n.js `loaders`).
import "./i18n/en.js";

createRoot(document.getElementById("v2-root")).render(html`
  <${I18nProvider}>
    <${QueryClientProvider} client=${queryClient}>
      <${App} />
    <//>
  <//>
`);
