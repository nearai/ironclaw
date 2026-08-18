import { useNavigate, useOutletContext, useParams } from "react-router";
import { Button } from "../../design-system/button";
import { InlineNotice } from "../../design-system/inline-notice";
import { StatusPill } from "../../design-system/primitives";
import React from "react";
import { useT } from "../../lib/i18n";
import { WorkspaceDirectory } from "./components/workspace-directory";
import { WorkspaceSidebar } from "./components/workspace-sidebar";
import { WorkspaceViewer } from "./components/workspace-viewer";
import { useWorkspaceBrowser } from "./hooks/useWorkspaceBrowser";
import { DEFAULT_WORKSPACE_PATH, routeForWorkspacePath } from "./lib/workspace-presenters";

export function WorkspacePage() {
  const t = useT();
  const navigate = useNavigate();
  const params = useParams();
  const {
    currentUser = null,
    workspaceRequiresScopedProjection = true,
  } = useOutletContext() as {
    currentUser?: { tenant_id?: string | null; user_id?: string | null } | null;
    workspaceRequiresScopedProjection?: boolean;
  };
  const workspaceThreadId = params.workspaceThreadId || null;
  const selectedPath = params["*"] || DEFAULT_WORKSPACE_PATH;
  const workspace = useWorkspaceBrowser(selectedPath, {
    currentUser,
    requireScopedWorkspace: workspaceRequiresScopedProjection,
    threadId: workspaceThreadId,
  });

  const navigateWithinWorkspace = React.useCallback(
    (path) => navigate(routeForWorkspacePath(path, workspaceThreadId)),
    [navigate, workspaceThreadId],
  );

  const handleSelectFile = React.useCallback(
    (path) => {
      navigateWithinWorkspace(path);
    },
    [navigateWithinWorkspace]
  );

  return (
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="flex h-full min-h-0 flex-col space-y-5">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h1
                  data-testid="workspace-heading"
                  className="text-lg font-semibold text-white"
                >{t("workspace.title")}</h1>
                <StatusPill tone="muted" label={t("workspace.readOnly")} />
              </div>
              <p className="mt-0.5 text-sm text-iron-400">{t("workspace.subtitle")}</p>
            </div>
            <Button
              variant="secondary"
              size="sm"
              onClick={workspace.refresh}
              disabled={workspace.isFetching}
            >
              {workspace.isFetching ? t("workspace.refreshing") : t("workspace.refresh")}
            </Button>
          </div>

          {workspace.error &&
          (
            <InlineNotice tone="danger" role="alert">
              {workspace.error.message}
            </InlineNotice>
          )}
          {workspace.result && (
            <InlineNotice
              tone={workspace.result.type === "success" ? "success" : workspace.result.type === "error" ? "danger" : "info"}
              role={workspace.result.type === "error" ? "alert" : "status"}
              onDismiss={workspace.clearResult}
              dismissLabel={t("projects.feedback.dismiss")}
            >
              {workspace.result.message}
            </InlineNotice>
          )}

          <div
            className="grid min-h-0 flex-1 gap-5 xl:grid-cols-[340px_minmax(0,1fr)]"
          >
            <WorkspaceSidebar
              rootEntries={workspace.rootEntries}
              selectedPath={selectedPath}
              expandedPaths={workspace.expandedPaths}
              filter={workspace.filter}
              scopeKey={workspace.scopeKey}
              listDirectory={workspace.listDirectory}
              onFilterChange={workspace.setFilter}
              isLoadingTree={workspace.isLoadingTree}
              onToggleDirectory={workspace.toggleDirectory}
              onSelectFile={handleSelectFile}
            />
            {workspace.selectionIsDirectory
              ? (
                  <WorkspaceDirectory
                    path={selectedPath}
                    entries={workspace.currentEntries}
                    isLoading={workspace.isLoadingListing}
                    filter={workspace.filter}
                    onOpen={handleSelectFile}
                    onNavigate={navigateWithinWorkspace}
                  />
                )
              : (
                  <WorkspaceViewer
                    path={selectedPath}
                    file={workspace.file}
                    isLoading={workspace.isLoadingFile}
                    onNavigate={navigateWithinWorkspace}
                  />
                )}
          </div>
        </div>
      </div>
    </div>
  );
}
