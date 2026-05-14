import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { RegistryCard } from "./extension-card.js";

export function RegistryTab({
  toolRegistry,
  channelRegistry,
  mcpRegistry,
  onInstall,
  isBusy,
}) {
  const t = useT();
  const allAvailable = [...toolRegistry, ...channelRegistry, ...mcpRegistry];
  const [filter, setFilter] = React.useState("");

  const filtered = filter
    ? allAvailable.filter(
        (e) =>
          (e.display_name || e.name)
            .toLowerCase()
            .includes(filter.toLowerCase()) ||
          (e.description || "").toLowerCase().includes(filter.toLowerCase()) ||
          (e.keywords || []).some((kw) =>
            kw.toLowerCase().includes(filter.toLowerCase())
          )
      )
    : allAvailable;

  if (allAvailable.length === 0) {
    return html`
      <div className="v2-panel rounded-[18px] p-6 sm:p-8">
        <h3 className="text-lg font-semibold text-white">
          ${t("ext.registry.emptyTitle")}
        </h3>
        <p className="mt-2 max-w-md text-sm leading-6 text-iron-300">
          ${t("ext.registry.emptyDesc")}
        </p>
      </div>
    `;
  }

  return html`
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <input
          type="text"
          value=${filter}
          onChange=${(e) => setFilter(e.target.value)}
          placeholder=${t("ext.registry.searchPlaceholder")}
          className="h-9 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <span className="font-mono text-[11px] text-iron-700">
          ${filtered.length} / ${allAvailable.length}
        </span>
      </div>

      <div className="v2-panel rounded-[18px] p-5 sm:p-6">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
        >
          ${t("ext.registry.availableTitle")}
        </h3>
        ${filtered.length === 0
          ? html`<p className="py-4 text-sm text-iron-300">
              ${t("ext.registry.noMatch")}
            </p>`
          : filtered.map(
              (entry) => html`
                <${RegistryCard}
                  key=${entry.name}
                  entry=${entry}
                  onInstall=${onInstall}
                  isBusy=${isBusy}
                />
              `
            )}
      </div>
    </div>
  `;
}
