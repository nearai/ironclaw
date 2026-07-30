// @ts-nocheck
import { Button, Icon, Card, SearchInput, Toolbar, ToolbarGroup } from "@ironclaw/ui";
import React from "react";
import { useT } from "../../../lib/i18n";
import { saveBlob } from "../../../lib/download";
import { NoSupportedSettingsImportError } from "../lib/settings-api";

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
      event.currentTarget.value = "";
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
        if (error instanceof NoSupportedSettingsImportError) {
          showMessage("error", t("settings.importNoSupported"));
          return;
        }
        showMessage(
          "error",
          t("settings.importFailed", { message: error.message })
        );
      }
    },
    [onImport, showMessage, t]
  );

  return (
    <Card radius="sm" className="px-3 py-3">
      <Toolbar>
        <div className="flex min-w-0 flex-1 flex-col gap-3 sm:flex-row sm:items-center">
          {canGoBack &&
          (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={onBack}
              className="w-fit gap-2"
            >
              <Icon name="chevron" className="h-3.5 w-3.5 rotate-90" />
              {t("settings.back")}
            </Button>
          )}

          <SearchInput
            className="min-w-0 flex-1"
            label={t("settings.searchPlaceholder")}
            value={searchQuery}
            onChange={(event) => onSearchChange(event.currentTarget.value)}
            onClear={onSearchClear}
            clearLabel={t("settings.clearSearch")}
            placeholder={t("settings.searchPlaceholder")}
          />
        </div>

        <ToolbarGroup>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={handleExport}
            disabled={!settingsExport || isImporting}
            className="gap-2"
          >
            <Icon name="download" className="h-3.5 w-3.5" />
            {t("settings.export")}
          </Button>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={() => fileInputRef.current?.click()}
            disabled={isImporting}
            className="gap-2"
          >
            <Icon name="upload" className="h-3.5 w-3.5" />
            {isImporting ? t("settings.importing") : t("settings.import")}
          </Button>
          <input
            ref={fileInputRef}
            type="file"
            accept=".json,application/json"
            className="hidden"
            onChange={handleImportFile}
          />
        </ToolbarGroup>
      </Toolbar>

      <div className="mt-2 min-w-0">
        <div className="text-xs font-medium text-iron-400">{t("settings.manageJson")}</div>
        {message &&
        (
          <div
            role="status"
            className={[
              "mt-1 text-xs",
              message.tone === "error"
                ? "text-[var(--v2-danger-text)]"
                : "text-[var(--v2-positive-text)]",
            ].join(" ")}
          >
            {message.text}
          </div>
        )}
      </div>
    </Card>
  );
}
