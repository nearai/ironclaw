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
  const { tab = "inference" } = useParams();
  const navigate = useNavigate();
  const { gatewayStatus, gatewayStatusQuery, isAdmin = true } = useOutletContext();
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

  const handleBack = React.useCallback(() => {
    navigate("/settings/inference");
  }, [navigate]);

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

  if (!tabContent[tab] || (!isAdmin && tab === "users")) {
    return html`<${Navigate} to="/settings/inference" replace />`;
  }

  return html`
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="v2-page-entrance flex-1 p-4 sm:p-6">
          <div className="space-y-5">
            <${RestartBanner}
              visible=${needsRestart}
              gatewayStatus=${gatewayStatus}
              gatewayStatusQuery=${gatewayStatusQuery}
            />

            <${SettingsToolbar}
              settingsExport=${query.data}
              onImport=${importSettings}
              isImporting=${isImporting}
              searchQuery=${searchQuery}
              onSearchChange=${setSearchQuery}
              onSearchClear=${() => setSearchQuery("")}
              onBack=${handleBack}
              canGoBack=${tab !== "inference"}
            />

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
