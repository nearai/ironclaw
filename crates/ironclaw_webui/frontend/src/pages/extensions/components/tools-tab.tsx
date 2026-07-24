import { useT } from "../../../lib/i18n";
import { ExtensionCard, RegistryCard } from "./extension-card";

function packageId(item) {
  return item.package_ref?.id || "";
}

// The tools view over extensions with tool surfaces (any runtime — wasm and
// MCP-backed extensions sit side by side; runtime shows as a card badge).
export function ToolsTab({
  tools,
  toolRegistry,
  onConfigure,
  onRemove,
  onInstall,
  isBusy,
}) {
  const t = useT();
  if (tools.length === 0 && toolRegistry.length === 0) {
    return (
      <div className="v2-panel rounded-[18px] p-6 sm:p-8">
        <h3 className="text-lg font-semibold text-white">{t("extensions.emptyToolsTitle")}</h3>
        <p className="mt-2 max-w-md text-sm leading-6 text-iron-300">
          {t("extensions.emptyToolsDesc")}
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-5">
      {tools.length > 0 &&
      (
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            {t("tools.installed")}
          </h3>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
            {tools.map(
              (ext) => (
                <ExtensionCard
                  key={packageId(ext)}
                  ext={ext}
                  onConfigure={onConfigure}
                  onRemove={onRemove}
                  isBusy={isBusy}
                />
              )
            )}
          </div>
        </div>
      )}
      {toolRegistry.length > 0 &&
      (
        <div className="v2-panel rounded-[18px] p-5 sm:p-6">
          <h3
            className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
          >
            {t("tools.available")}
          </h3>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
            {toolRegistry.map(
              (entry) => (
                <RegistryCard
                  key={packageId(entry)}
                  entry={entry}
                  onInstall={onInstall}
                  isBusy={isBusy}
                />
              )
            )}
          </div>
        </div>
      )}
    </div>
  );
}
