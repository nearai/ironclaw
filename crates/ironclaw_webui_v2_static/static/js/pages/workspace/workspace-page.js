import { useNavigate, useParams } from "react-router";
import { Button } from "../../design-system/button.js";
import { React, html } from "../../lib/html.js";
import { useT } from "../../lib/i18n.js";
import { FeedbackBanner } from "../projects/components/feedback-banner.js";
import { WorkspaceSidebar } from "./components/workspace-sidebar.js";
import { WorkspaceViewer } from "./components/workspace-viewer.js";
import { useWorkspaceBrowser } from "./hooks/useWorkspaceBrowser.js";
import {
  DEFAULT_WORKSPACE_PATH,
  routeForWorkspacePath,
} from "./lib/workspace-presenters.js";

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

  const handleSave = React.useCallback(async () => {
    try {
      await workspace.save();
    } catch {
      // Visible result state is owned by the hook.
    }
  }, [workspace]);

  return html`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="flex h-full min-h-0 flex-col space-y-5">
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
              search=${workspace.search}
              onSearchChange=${workspace.setSearch}
              rootEntries=${workspace.rootEntries}
              selectedPath=${selectedPath}
              expandedPaths=${workspace.expandedPaths}
              searchResults=${workspace.searchResults}
              isLoadingTree=${workspace.isLoadingTree}
              isSearching=${workspace.isSearching}
              onToggleDirectory=${workspace.toggleDirectory}
              onSelectFile=${handleSelectFile}
            />
            <${WorkspaceViewer}
              path=${selectedPath}
              file=${workspace.file}
              draft=${workspace.draft}
              onDraftChange=${workspace.setDraft}
              editing=${workspace.editing}
              onStartEdit=${() => workspace.setEditing(true)}
              onCancelEdit=${() => workspace.setEditing(false)}
              onSave=${handleSave}
              isLoading=${workspace.isLoadingFile}
              isSaving=${workspace.isSaving}
              onNavigate=${navigate}
            />
          </div>
        </div>
      </div>
    </div>
  `;
}
