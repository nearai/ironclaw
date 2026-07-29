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

async function bootstrap() {
  // Staging walkthrough builds only (VITE_DEMO_MODE=1): swap the network
  // layer for in-memory fixtures before anything mounts. The condition is a
  // build-time constant, so production builds eliminate this branch and the
  // demo module entirely.
  if (import.meta.env.VITE_DEMO_MODE === "1") {
    const { installDemoMode } = await import("./demo/install");
    installDemoMode();
  }

  createRoot(document.getElementById("v2-root")).render((
    <I18nProvider>
      <UiTextBridge>
        <QueryClientProvider client={queryClient}>
          <App />
        </QueryClientProvider>
      </UiTextBridge>
    </I18nProvider>
  ));
}

bootstrap();
