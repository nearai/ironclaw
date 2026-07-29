import { Text } from "@ironclaw/design-system";
import { useT } from "../../../lib/i18n";
import { ExtensionCard, RegistryCard } from "./extension-card";
import type {
  ConfigureFocusHandler,
  InstallFocusHandler,
} from "../lib/focus-target";

function packageId(item) {
  return item.package_ref?.id || "";
}

// The tools view over extensions with tool surfaces (any runtime — wasm and
// MCP-backed extensions sit side by side; runtime shows as a card badge).
/**
 * @param {{
 *   tools: any[];
 *   toolRegistry: any[];
 *   onConfigure: ConfigureFocusHandler<any>;
 *   onRemove: (extension: any) => void;
 *   onInstall: InstallFocusHandler;
 *   isBusy: boolean;
 * }} props
 */
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
      <div className="v2-panel rounded-[var(--v2-radius-bubble)] p-6 sm:p-8">
        <h3 className="text-lg font-medium text-[var(--v2-text-strong)]">{t("extensions.emptyToolsTitle")}</h3>
        <Text variant="body" tone="muted" className="mt-2 max-w-md">
          {t("extensions.emptyToolsDesc")}
        </Text>
      </div>
    );
  }

  return (
    <div className="space-y-5">
      {tools.length > 0 &&
      (
        <div className="v2-panel rounded-[var(--v2-radius-bubble)] p-5 sm:p-6">
          <Text as="h3" variant="eyebrow" tone="accent" className="mb-4">
            {t("tools.installed")}
          </Text>
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
        <div className="v2-panel rounded-[var(--v2-radius-bubble)] p-5 sm:p-6">
          <Text as="h3" variant="eyebrow" tone="accent" className="mb-4">
            {t("tools.available")}
          </Text>
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
