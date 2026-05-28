import { html } from "../../../lib/html.js";
import { ExtensionCard } from "./extension-card.js";

export function InstalledTab({ extensions, onActivate, onConfigure, onRemove, isBusy }) {
  if (extensions.length === 0) {
    return html`
      <div className="v2-panel rounded-[18px] p-6 sm:p-8">
        <h3 className="text-lg font-semibold text-white">No extensions installed</h3>
        <p className="mt-2 max-w-md text-sm leading-6 text-iron-300">
          Browse the Registry tab to discover and install WASM tools, channels, and MCP servers.
        </p>
      </div>
    `;
  }

  return html`
    <div className="v2-panel rounded-[18px] p-5 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
        All installed extensions
      </h3>
      ${extensions.map(
        (ext) => html`
          <${ExtensionCard}
            key=${ext.name}
            ext=${ext}
            onActivate=${onActivate}
            onConfigure=${onConfigure}
            onRemove=${onRemove}
            isBusy=${isBusy}
          />
        `
      )}
    </div>
  `;
}
