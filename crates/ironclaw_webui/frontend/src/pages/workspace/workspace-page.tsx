import { useNavigate, useParams } from "react-router";
import { Badge, Button, Callout, SectionHeader } from "@ironclaw/ui";
import React from "react";
import { useT } from "../../lib/i18n";
import { FeedbackBanner } from "../projects/components/feedback-banner";
import { WorkspaceDirectory } from "./components/workspace-directory";
import { WorkspaceSidebar } from "./components/workspace-sidebar";
import { WorkspaceViewer } from "./components/workspace-viewer";
import { useWorkspaceBrowser } from "./hooks/useWorkspaceBrowser";
import { DEFAULT_WORKSPACE_PATH, routeForWorkspacePath } from "./lib/workspace-presenters";

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

  return (
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="flex h-full min-h-0 flex-col space-y-5">
          <SectionHeader
            titleAs="h1"
            title={<span data-testid="workspace-heading">{t("workspace.title")}</span>}
            description={t("workspace.subtitle")}
            actions={
              <>
                <Badge tone="muted" label={t("workspace.readOnly")} />
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={workspace.refresh}
                  disabled={workspace.isFetching}
                >
                  {workspace.isFetching ? t("workspace.refreshing") : t("workspace.refresh")}
                </Button>
              </>
            }
          />

          {workspace.error &&
          (<Callout tone="danger">{workspace.error.message}</Callout>)}
          <FeedbackBanner
            result={workspace.result}
            onDismiss={workspace.clearResult}
          />

          <div
            className="grid min-h-0 flex-1 gap-5 xl:grid-cols-[340px_minmax(0,1fr)]"
          >
            <WorkspaceSidebar
              rootEntries={workspace.rootEntries}
              selectedPath={selectedPath}
              expandedPaths={workspace.expandedPaths}
              filter={workspace.filter}
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
                    onNavigate={navigate}
                  />
                )
              : (
                  <WorkspaceViewer
                    path={selectedPath}
                    file={workspace.file}
                    isLoading={workspace.isLoadingFile}
                    onNavigate={navigate}
                  />
                )}
          </div>
        </div>
      </div>
    </div>
  );
}
