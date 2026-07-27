// @ts-nocheck
import { Navigate, useOutletContext, useParams } from "react-router";
import React from "react";
import { useT } from "../../lib/i18n";
import { RouteLoadBoundary } from "../../app/route-load-boundary";
import { RestartBanner } from "./components/restart-banner";
import { SettingsToolbar } from "./components/settings-toolbar";
import { useSettings } from "./hooks/useSettings";

const AppearanceTab = React.lazy(() =>
  import("./components/appearance-tab").then(({ AppearanceTab }) => ({ default: AppearanceTab }))
);
const InferenceTab = React.lazy(() =>
  import("./components/inference-tab").then(({ InferenceTab }) => ({ default: InferenceTab }))
);
const LanguageTab = React.lazy(() =>
  import("./components/language-tab").then(({ LanguageTab }) => ({ default: LanguageTab }))
);
const SkillsTab = React.lazy(() =>
  import("./components/skills-tab").then(({ SkillsTab }) => ({ default: SkillsTab }))
);
const ToolsTab = React.lazy(() =>
  import("./components/tools-tab").then(({ ToolsTab }) => ({ default: ToolsTab }))
);
const TraceCommonsTab = React.lazy(() =>
  import("./components/trace-commons-tab").then(({ TraceCommonsTab }) => ({
    default: TraceCommonsTab,
  }))
);

export function SettingsPage() {
  const t = useT();
  const { tab: requestedTab } = useParams();
  const {
    gatewayStatus,
    gatewayStatusQuery,
    isAdmin = false,
    theme,
    setTheme,
  } = useOutletContext();
  const defaultTab = isAdmin ? "inference" : "language";
  const tab = requestedTab || defaultTab;
  const {
    settings,
    query,
    save,
    savedKeys,
    needsRestart,
    importSettings,
    isImporting,
    saveError,
  } = useSettings();
  const [searchQuery, setSearchQuery] = React.useState("");

  React.useEffect(() => {
    setSearchQuery("");
  }, [tab]);

  const isLoading = query.isLoading;

  const tabContent = {
    inference: (<InferenceTab
      settings={settings}
      gatewayStatus={gatewayStatus}
      onSave={save}
      savedKeys={savedKeys}
      isLoading={isLoading}
      searchQuery={searchQuery}
    />),
    appearance: (<AppearanceTab
      searchQuery={searchQuery}
      theme={theme}
      onThemeChange={setTheme}
    />),
    tools: (<ToolsTab
      settings={settings}
      onSave={save}
      savedKeys={savedKeys}
      isLoading={isLoading}
      searchQuery={searchQuery}
    />),
    skills: (<SkillsTab searchQuery={searchQuery} />),
    traces: (<TraceCommonsTab searchQuery={searchQuery} />),
    language: (<LanguageTab searchQuery={searchQuery} />),
  };

  const isOperatorTab = (id) => id === "inference";
  const tabContentHas = (id) => Object.prototype.hasOwnProperty.call(tabContent, id);
  const visibleTabIds = Object.keys(tabContent).filter((id) => isAdmin || !isOperatorTab(id));
  const defaultTabIsVisible = tabContentHas(defaultTab) && visibleTabIds.includes(defaultTab);
  const redirectTab = defaultTabIsVisible ? defaultTab : visibleTabIds[0] || "language";

  if (!tabContentHas(tab) || (!isAdmin && isOperatorTab(tab))) {
    return (<Navigate to={`/settings/${redirectTab}`} replace />);
  }

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            {needsRestart &&
            (<div className="sticky top-0 z-20 -mx-4 -mt-4 mb-1 bg-[color-mix(in_srgb,var(--v2-canvas)_92%,transparent)] px-4 pt-4 backdrop-blur sm:-mx-6 sm:px-6">
              <RestartBanner
                visible={true}
                gatewayStatus={gatewayStatus}
                gatewayStatusQuery={gatewayStatusQuery}
              />
            </div>)}

            {saveError &&
            (
              <div
                className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
              >
                {t("error.saveFailed", { message: saveError.message })}
              </div>
            )}

            <SettingsToolbar
              settingsExport={query.data || null}
              onImport={importSettings}
              isImporting={isImporting}
              searchQuery={searchQuery}
              onSearchChange={setSearchQuery}
              onSearchClear={() => setSearchQuery("")}
              canGoBack={false}
            />

            <RouteLoadBoundary>{tabContent[tab]}</RouteLoadBoundary>
          </div>
        </div>
      </div>
    </div>
  );
}
