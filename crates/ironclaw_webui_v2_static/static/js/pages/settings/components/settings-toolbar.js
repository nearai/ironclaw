import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { saveBlob } from "../../../lib/download.js";

function downloadJson(filename, data) {
  saveBlob(
    new Blob([JSON.stringify(data, null, 2)], { type: "application/json" }),
    filename,
  );
}

function readJsonFile(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      try {
        resolve(JSON.parse(reader.result));
      } catch (error) {
        reject(error);
      }
    };
    reader.onerror = () =>
      reject(reader.error || new Error("Unable to read file"));
    reader.readAsText(file);
  });
}

export function SettingsToolbar({
  settingsExport,
  onImport,
  isImporting,
  searchQuery,
  onSearchChange,
  onSearchClear,
  onBack,
  canGoBack,
}) {
  const t = useT();
  const fileInputRef = React.useRef(null);
  const messageTimerRef = React.useRef(null);
  const [message, setMessage] = React.useState(null);

  const showMessage = React.useCallback((tone, text) => {
    if (messageTimerRef.current) {
      window.clearTimeout(messageTimerRef.current);
    }
    setMessage({ tone, text });
    messageTimerRef.current = window.setTimeout(() => setMessage(null), 3500);
  }, []);

  React.useEffect(
    () => () => {
      if (messageTimerRef.current) {
        window.clearTimeout(messageTimerRef.current);
      }
    },
    []
  );

  const handleExport = React.useCallback(() => {
    if (!settingsExport) return;
    downloadJson("ironclaw-settings.json", settingsExport);
    showMessage("success", t("settings.exportSuccess"));
  }, [settingsExport, showMessage, t]);

  const handleImportFile = React.useCallback(
    async (event) => {
      const file = event.target.files?.[0];
      event.target.value = "";
      if (!file) return;

      try {
        const payload = await readJsonFile(file);
        if (
          !payload ||
          typeof payload !== "object" ||
          !payload.settings ||
          typeof payload.settings !== "object" ||
          Array.isArray(payload.settings)
        ) {
          throw new Error(t("settings.importInvalid"));
        }
        await onImport(payload);
        showMessage("success", t("settings.importSuccess"));
      } catch (error) {
        showMessage(
          "error",
          t("settings.importFailed", { message: error.message })
        );
      }
    },
    [onImport, showMessage, t]
  );

  return html`
    <div className="rounded-md border border-white/10 bg-white/[0.03] px-3 py-3">
      <div className="flex flex-col gap-3 lg:flex-row lg:items-center">
        <div className="flex min-w-0 flex-1 flex-col gap-3 sm:flex-row sm:items-center">
          ${canGoBack &&
          html`
            <${Button}
              type="button"
              variant="ghost"
              size="sm"
              onClick=${onBack}
              className="w-fit gap-2"
            >
              <${Icon} name="chevron" className="h-3.5 w-3.5 rotate-90" />
              ${t("settings.back")}
            <//>
          `}

          <label className="relative min-w-0 flex-1">
            <span className="sr-only">${t("settings.searchPlaceholder")}</span>
            <${Icon}
              name="search"
              className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[var(--v2-text-faint)]"
            />
            <input
              type="search"
              value=${searchQuery}
              onChange=${(event) => onSearchChange(event.target.value)}
              placeholder=${t("settings.searchPlaceholder")}
              className="h-9 w-full rounded-md border border-white/12 bg-white/[0.04] pl-9 pr-9 text-sm text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/45"
            />
            ${searchQuery &&
            html`
              <button
                type="button"
                onClick=${onSearchClear}
                aria-label=${t("settings.clearSearch")}
                className="absolute right-2 top-1/2 grid h-6 w-6 -translate-y-1/2 place-items-center rounded-md text-[var(--v2-text-faint)] hover:bg-white/[0.07] hover:text-[var(--v2-text-strong)]"
              >
                <${Icon} name="close" className="h-3.5 w-3.5" />
              </button>
            `}
          </label>
        </div>

        <div className="flex shrink-0 flex-wrap gap-2">
          <${Button}
            type="button"
            variant="secondary"
            size="sm"
            onClick=${handleExport}
            disabled=${!settingsExport || isImporting}
            className="gap-2"
          >
            <${Icon} name="download" className="h-3.5 w-3.5" />
            ${t("settings.export")}
          <//>
          <${Button}
            type="button"
            variant="secondary"
            size="sm"
            onClick=${() => fileInputRef.current?.click()}
            disabled=${isImporting}
            className="gap-2"
          >
            <${Icon} name="upload" className="h-3.5 w-3.5" />
            ${isImporting ? t("settings.importing") : t("settings.import")}
          <//>
          <input
            ref=${fileInputRef}
            type="file"
            accept=".json,application/json"
            className="hidden"
            onChange=${handleImportFile}
          />
        </div>
      </div>

      <div className="mt-2 min-w-0">
        <div className="text-xs font-medium text-iron-400">${t("settings.manageJson")}</div>
        ${message &&
        html`
          <div
            role="status"
            className=${[
              "mt-1 text-xs",
              message.tone === "error" ? "text-red-200" : "text-mint",
            ].join(" ")}
          >
            ${message.text}
          </div>
        `}
      </div>
    </div>
  `;
}
