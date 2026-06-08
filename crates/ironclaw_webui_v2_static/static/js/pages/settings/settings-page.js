import { Navigate, useNavigate, useOutletContext, useParams } from "react-router";
import { React, html } from "../../lib/html.js";
import { useT } from "../../lib/i18n.js";
import { AgentTab } from "./components/agent-tab.js";
import { ChannelsTab } from "./components/channels-tab.js";
import { InferenceTab } from "./components/inference-tab.js";
import { LanguageTab } from "./components/language-tab.js";
import { NetworkingTab } from "./components/networking-tab.js";
import { RestartBanner } from "./components/restart-banner.js";
import { SkillsTab } from "./components/skills-tab.js";
import { SettingsToolbar } from "./components/settings-toolbar.js";
import { ToolsTab } from "./components/tools-tab.js";
import { UsersTab } from "./components/users-tab.js";
import { useSettings } from "./hooks/useSettings.js";

export function SettingsPage() {
  const t = useT();
  const { tab: requestedTab } = useParams();
  const navigate = useNavigate();
  const { gatewayStatus, gatewayStatusQuery, isAdmin = false } = useOutletContext();
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
    inference: html`<${InferenceTab}
      settings=${settings}
      gatewayStatus=${gatewayStatus}
      onSave=${save}
      savedKeys=${savedKeys}
      isLoading=${isLoading}
      searchQuery=${searchQuery}
    />`,
    agent: html`<${AgentTab}
      settings=${settings}
      onSave=${save}
      savedKeys=${savedKeys}
      isLoading=${isLoading}
      searchQuery=${searchQuery}
    />`,
    channels: html`<${ChannelsTab} searchQuery=${searchQuery} />`,
    networking: html`<${NetworkingTab}
      settings=${settings}
      onSave=${save}
      savedKeys=${savedKeys}
      isLoading=${isLoading}
      searchQuery=${searchQuery}
    />`,
    tools: html`<${ToolsTab} searchQuery=${searchQuery} />`,
    skills: html`<${SkillsTab} searchQuery=${searchQuery} />`,
    users: html`<${UsersTab} searchQuery=${searchQuery} />`,
    language: html`<${LanguageTab} searchQuery=${searchQuery} />`,
  };

  const isOperatorTab = (id) => id === "users" || id === "inference";
  const tabContentHas = (id) => Object.prototype.hasOwnProperty.call(tabContent, id);
  const visibleTabIds = Object.keys(tabContent).filter((id) => isAdmin || !isOperatorTab(id));
  const defaultTabIsVisible = tabContentHas(defaultTab) && visibleTabIds.includes(defaultTab);
  const redirectTab = defaultTabIsVisible ? defaultTab : visibleTabIds[0] || "language";

  const handleBack = React.useCallback(() => {
    navigate(`/settings/${redirectTab}`);
  }, [redirectTab, navigate]);

  if (!tabContentHas(tab) || (!isAdmin && isOperatorTab(tab))) {
    return html`<${Navigate} to=${`/settings/${redirectTab}`} replace />`;
  }

  return html`
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            ${needsRestart &&
            html`<div className="sticky top-0 z-20 -mx-4 -mt-4 mb-1 bg-[color-mix(in_srgb,var(--v2-canvas)_92%,transparent)] px-4 pt-4 backdrop-blur sm:-mx-6 sm:px-6">
              <${RestartBanner}
                visible=${true}
                gatewayStatus=${gatewayStatus}
                gatewayStatusQuery=${gatewayStatusQuery}
              />
            </div>`}

            ${''
              // <${SettingsToolbar}
              //   settingsExport=${query.data}
              //   onImport=${importSettings}
              //   isImporting=${isImporting}
              //   searchQuery=${searchQuery}
              //   onSearchChange=${setSearchQuery}
              //   onSearchClear=${() => setSearchQuery("")}
              //   onBack=${handleBack}
              //   canGoBack=${tab !== "inference"}
              // />
            }

            ${saveError &&
            html`
              <div
                className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
              >
                ${t("error.saveFailed", { message: saveError.message })}
              </div>
            `}

            ${tabContent[tab]}
          </div>
        </div>
      </div>
    </div>
  `;
}
