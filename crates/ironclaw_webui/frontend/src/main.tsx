import "./styles/app.css";
import { QueryClientProvider } from "@tanstack/react-query";
import { createRoot } from "react-dom/client";
import { App } from "./app/app";
import { UiTextBridge } from "./app/ui-text-bridge";
import { queryClient } from "./lib/query-client";
import { I18nProvider } from "./lib/i18n";
// Only the English fallback is bundled eagerly; every other locale is
// lazy-loaded on demand by I18nProvider (see lib/i18n.tsx `loaders`).
import "./i18n/en";

createRoot(document.getElementById("v2-root")).render((
  <I18nProvider>
    <UiTextBridge>
      <QueryClientProvider client={queryClient}>
        <App />
      </QueryClientProvider>
    </UiTextBridge>
  </I18nProvider>
));
