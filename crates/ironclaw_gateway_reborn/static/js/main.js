import { QueryClientProvider } from "@tanstack/react-query";
import { createRoot } from "react-dom/client";
import { App } from "./app/app.js";
import { html } from "./lib/html.js";
import { queryClient } from "./lib/query-client.js";
import { I18nProvider } from "./lib/i18n.js";
import "./i18n/en.js";
import "./i18n/es.js";
import "./i18n/fr.js";
import "./i18n/de.js";
import "./i18n/pt-BR.js";
import "./i18n/ja.js";
import "./i18n/ar.js";
import "./i18n/hi.js";
import "./i18n/uk.js";
import "./i18n/zh-CN.js";
import "./i18n/ko.js";

createRoot(document.getElementById("v2-root")).render(html`
  <${I18nProvider}>
    <${QueryClientProvider} client=${queryClient}>
      <${App} />
    <//>
  <//>
`);
