import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { ExtensionCard, RegistryCard } from "./extension-card.js";

function packageId(item) {
  return item.package_ref?.id || "";
}

export function McpTab({
  mcpServers,
  mcpRegistry,
  onActivate,
  onConfigure,
  onRemove,
  onInstall,
  isBusy,
}) {
  const t = useT();
  if (mcpServers.length === 0 && mcpRegistry.length === 0) {
    return html`
      <div className="v2-panel rounded-[18px] p-6 sm:p-8">
        <h3 className="text-lg font-semibold text-white">${t("extensions.emptyMcpTitle")}</h3>
        <p className="mt-2 max-w-md text-sm leading-6 text-iron-300">
          ${t("extensions.emptyMcpDesc")}
        </p>
      </div>
    `;
  }

  return html`
    <div className="space-y-5">
      ${mcpServers.length > 0 &&
      html`
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            ${t("mcp.installed")}
          </h3>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
            ${mcpServers.map(
              (ext) => html`
                <${ExtensionCard}
                  key=${packageId(ext)}
                  ext=${ext}
                  onActivate=${onActivate}
                  onConfigure=${onConfigure}
                  onRemove=${onRemove}
                  isBusy=${isBusy}
                />
              `
            )}
          </div>
        </div>
      `}
      ${mcpRegistry.length > 0 &&
      html`
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            Available MCP servers
          </h3>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
            ${mcpRegistry.map(
              (entry) => html`
                <${RegistryCard}
                  key=${packageId(entry)}
                  entry=${entry}
                  onInstall=${onInstall}
                  isBusy=${isBusy}
                />
              `
            )}
          </div>
        </div>
      `}
    </div>
  `;
}
