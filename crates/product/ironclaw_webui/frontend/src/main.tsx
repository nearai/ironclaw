import "./styles/app.css";
import { QueryClientProvider } from "@tanstack/react-query";
import { createRoot } from "react-dom/client";
import { App } from "./app/app";
import { queryClient } from "./lib/query-client";
import { I18nProvider } from "./lib/i18n";
import { registerServiceWorker } from "./lib/register-sw";
// Only the English fallback is bundled eagerly; every other locale is
// lazy-loaded on demand by I18nProvider (see lib/i18n.tsx `loaders`).
import "./i18n/en";

// Boot-time side effect: register the notification service worker (safe
// no-op in browsers without support; never blocks or fails rendering).
registerServiceWorker();

createRoot(document.getElementById("v2-root")).render((
  <I18nProvider>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </I18nProvider>
));
