import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { ExtensionCard, RegistryCard } from "./extension-card.js";

function packageId(item) {
  return item?.package_ref?.id || "";
}

function catalogItem(entry) {
  return entry.entry || entry.extension || {};
}

export function RegistryTab({
  catalogEntries,
  onInstall,
  onActivate,
  onConfigure,
  onRemove,
  isBusy,
}) {
  const t = useT();
  const [filter, setFilter] = React.useState("");
  const query = filter.trim().toLowerCase();

  const filtered = query
    ? catalogEntries.filter((entry) => {
        const item = catalogItem(entry);
        return (
          (item.display_name || packageId(item)).toLowerCase().includes(query) ||
          (item.description || "").toLowerCase().includes(query) ||
          (item.keywords || []).some((kw) =>
            kw.toLowerCase().includes(query)
          )
        );
      })
    : catalogEntries;

  const installedEntries = filtered.filter((entry) => entry.installed && entry.extension);
  const registryOnlyInstalledEntries = filtered.filter(
    (entry) => entry.installed && !entry.extension && entry.entry
  );
  const installedCount = installedEntries.length + registryOnlyInstalledEntries.length;
  const availableEntries = filtered.filter((entry) => !entry.installed && entry.entry);

  if (catalogEntries.length === 0) {
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
          ${filtered.length} / ${catalogEntries.length}
        </span>
      </div>

      <div className="v2-panel rounded-[18px] p-5 sm:p-6">
        ${filtered.length === 0
          ? html`<p className="py-4 text-sm text-iron-300">
              ${t("ext.registry.noMatch")}
            </p>`
          : html`
              ${installedCount > 0 &&
              html`
                <h3
                  className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal"
                >
                  ${t("extensions.installed")}
                </h3>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
                  ${installedEntries.map(
                    (entry) => html`
                      <${ExtensionCard}
                        key=${entry.id}
                        ext=${entry.extension || entry.entry}
                        onActivate=${onActivate}
                        onConfigure=${onConfigure}
                        onRemove=${onRemove}
                        isBusy=${isBusy}
                      />
                    `
                  )}
                  ${registryOnlyInstalledEntries.map(
                    (entry) => html`
                      <${RegistryCard}
                        key=${entry.id}
                        entry=${entry.entry}
                        statusLabel=${t("extensions.installed")}
                        isBusy=${isBusy}
                      />
                    `
                  )}
                </div>
              `}

              ${availableEntries.length > 0 &&
              html`
                <h3
                  className=${[
                    "mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal",
                    installedCount > 0 ? "mt-6" : "",
                  ].join(" ")}
                >
                  ${t("ext.registry.availableTitle")}
                </h3>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
                  ${availableEntries.map(
                    (entry) => html`
                      <${RegistryCard}
                        key=${entry.id}
                        entry=${entry.entry}
                        onInstall=${onInstall}
                        isBusy=${isBusy}
                      />
                    `
                  )}
                </div>
              `}
            `}
      </div>
    </div>
  `;
}
