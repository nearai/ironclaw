import { useNavigate, useParams } from "react-router";
import { Button } from "../../design-system/button.js";
import { StatusPill } from "../../design-system/primitives.js";
import { React, html } from "../../lib/html.js";
import { useT } from "../../lib/i18n.js";
import { FeedbackBanner } from "../projects/components/feedback-banner.js";
import { WorkspaceDirectory } from "./components/workspace-directory.js";
import { WorkspaceSidebar } from "./components/workspace-sidebar.js";
import { WorkspaceViewer } from "./components/workspace-viewer.js";
import { useWorkspaceBrowser } from "./hooks/useWorkspaceBrowser.js";
import { DEFAULT_WORKSPACE_PATH, routeForWorkspacePath } from "./lib/workspace-presenters.js";

export function WorkspacePage() {
  const t = useT();
  const navigate = useNavigate();
  const params = useParams();
  const selectedPath = params["*"] || DEFAULT_WORKSPACE_PATH;
  const workspace = useWorkspaceBrowser(selectedPath);

  const handleSelectFile = React.useCallback(
    (path) => {
      navigate(routeForWorkspacePath(path));
    },
    [navigate]
  );

  return html`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="flex h-full min-h-0 flex-col space-y-5">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h1 className="text-lg font-semibold text-white">${t("workspace.title")}</h1>
                <${StatusPill} tone="muted" label=${t("workspace.readOnly")} />
              </div>
              <p className="mt-0.5 text-sm text-iron-400">${t("workspace.subtitle")}</p>
            </div>
            <${Button}
              variant="secondary"
              size="sm"
              onClick=${workspace.refresh}
              disabled=${workspace.isFetching}
            >
              ${workspace.isFetching ? t("workspace.refreshing") : t("workspace.refresh")}
            <//>
          </div>

          ${workspace.error &&
          html`
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              ${workspace.error.message}
            </div>
          `}
          <${FeedbackBanner}
            result=${workspace.result}
            onDismiss=${workspace.clearResult}
          />

          <div
            className="grid min-h-0 flex-1 gap-5 xl:grid-cols-[340px_minmax(0,1fr)]"
          >
            <${WorkspaceSidebar}
              rootEntries=${workspace.rootEntries}
              selectedPath=${selectedPath}
              expandedPaths=${workspace.expandedPaths}
              filter=${workspace.filter}
              onFilterChange=${workspace.setFilter}
              isLoadingTree=${workspace.isLoadingTree}
              onToggleDirectory=${workspace.toggleDirectory}
              onSelectFile=${handleSelectFile}
            />
            ${workspace.selectionIsDirectory
              ? html`
                  <${WorkspaceDirectory}
                    path=${selectedPath}
                    entries=${workspace.currentEntries}
                    isLoading=${workspace.isLoadingListing}
                    filter=${workspace.filter}
                    onOpen=${handleSelectFile}
                    onNavigate=${navigate}
                  />
                `
              : html`
                  <${WorkspaceViewer}
                    path=${selectedPath}
                    file=${workspace.file}
                    isLoading=${workspace.isLoadingFile}
                    onNavigate=${navigate}
                  />
                `}
          </div>
        </div>
      </div>
    </div>
  `;
}
