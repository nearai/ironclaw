// @ts-nocheck
import { Card } from "../../../design-system/card";
import { SelectMenu } from "../../../design-system/select-menu";
import { ApiError } from "../../../lib/api";
import { useT } from "../../../lib/i18n";
import { useUserModelPreference } from "../hooks/useUserModelPreference";

export function UserModelPreferenceSelector() {
  const t = useT();
  const {
    catalog,
    model,
    isLoading,
    isSaving,
    catalogReadFailed,
    preferenceReadFailed,
    saveError,
    setModel,
  } = useUserModelPreference();
  const workspaceDefault = catalog.workspace_default || t("inference.none");
  const availableModels = catalog.models || [];
  const options = [
    {
      value: "",
      label: t("llm.followWorkspaceDefault", { model: workspaceDefault }),
    },
    ...availableModels.map((availableModel) => ({
      value: availableModel,
      label: availableModel,
    })),
  ];
  if (model && !availableModels.includes(model)) {
    options.push({
      value: model,
      label: t("llm.unavailableModel", { model }),
      disabled: true,
      tone: "warning",
    });
  }

  return (
    <Card padding="none" className="p-4 sm:p-5">
      <div className="flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between">
        <div className="min-w-0">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
            {t("llm.modelPreference")}
          </h3>
          <p className="mt-2 max-w-2xl text-sm text-[var(--v2-text-muted)]">
            {t("llm.modelPreferenceDesc")}
          </p>
        </div>
        <div className="w-full min-w-0 xl:ml-auto xl:w-72 xl:max-w-full xl:flex-none">
          <SelectMenu
            data-testid="settings-model-selector"
            value={model || ""}
            options={options}
            onChange={setModel}
            disabled={
              isLoading ||
              isSaving ||
              catalogReadFailed ||
              preferenceReadFailed ||
              (!catalog.selection_enabled && !model)
            }
            ariaLabel={t("llm.modelPreference")}
            align="right"
            className="block w-full min-w-0 max-w-full"
            buttonClassName="w-full min-w-0 overflow-hidden"
            menuClassName="w-full max-w-[calc(100vw-2rem)]"
          />
          <div
            data-testid="settings-model-selector-status"
            className="mt-2 min-h-5 text-xs text-[var(--v2-text-muted)]"
          >
            {catalogReadFailed
              ? t("llm.catalogLoadFailed")
              : preferenceReadFailed
                ? t("llm.preferenceLoadFailed")
              : isSaving
                ? t("llm.preferenceSaving")
                : saveError
                  ? saveError instanceof ApiError
                    ? t("error.saveFailed", { message: saveError.message })
                    : t("llm.preferenceSaveFailed")
                  : !catalog.selection_enabled && !isLoading
                    ? t("llm.selectionUnavailable")
                    : null}
          </div>
        </div>
      </div>
    </Card>
  );
}
