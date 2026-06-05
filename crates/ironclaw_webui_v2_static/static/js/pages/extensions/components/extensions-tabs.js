import { Icon } from "../../../design-system/icons.js";
import { html } from "../../../lib/html.js";
import { EXTENSIONS_TABS } from "../lib/extensions-schema.js";

export function ExtensionsTabs({ activeTab, onTabChange, counts }) {
  return html`
    <div className="flex flex-col gap-1">
      ${EXTENSIONS_TABS.map(
        (tab) => html`
          <button
            key=${tab.id}
            onClick=${() => onTabChange(tab.id)}
            className=${[
              "group flex items-center gap-3 rounded-md px-3 py-2.5 text-left text-sm",
              activeTab === tab.id
                ? "v2-nav-active text-white"
                : "text-iron-300 hover:bg-white/[0.045] hover:text-white",
            ].join(" ")}
          >
            <span
              className=${[
                "grid h-7 w-7 shrink-0 place-items-center rounded-md border",
                activeTab === tab.id
                  ? "border-signal/35 bg-signal/10 text-signal"
                  : "border-white/10 bg-white/[0.035] text-iron-300 group-hover:border-signal/35 group-hover:text-signal",
              ].join(" ")}
            >
              <${Icon} name=${tab.icon} className="h-3.5 w-3.5" />
            </span>
            <span className="min-w-0 truncate">${tab.label}</span>
            ${counts[tab.id] != null &&
            html`
              <span className="ml-auto font-mono text-[11px] text-iron-700"
                >${counts[tab.id]}</span
              >
            `}
          </button>
        `
      )}
    </div>
  `;
}

export function ExtensionsTabsMobile({ activeTab, onTabChange, counts }) {
  return html`
    <div className="flex gap-1.5 overflow-x-auto pb-1">
      ${EXTENSIONS_TABS.map(
        (tab) => html`
          <button
            key=${tab.id}
            onClick=${() => onTabChange(tab.id)}
            className=${[
              "flex shrink-0 items-center gap-2 rounded-md px-3 py-2 text-sm whitespace-nowrap",
              activeTab === tab.id
                ? "border border-signal/35 bg-signal/10 text-white"
                : "border border-transparent text-iron-300 hover:text-white",
            ].join(" ")}
          >
            <${Icon} name=${tab.icon} className="h-3.5 w-3.5" />
            ${tab.label}
            ${counts[tab.id] != null &&
            html`
              <span className="font-mono text-[11px] text-iron-700"
                >${counts[tab.id]}</span
              >
            `}
          </button>
        `
      )}
    </div>
  `;
}
