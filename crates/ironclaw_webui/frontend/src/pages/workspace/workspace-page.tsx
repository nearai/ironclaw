import { useNavigate, useParams } from "react-router";
import { Button } from "@ironclaw/design-system";
import { StatusPill } from "@ironclaw/design-system";
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
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h1
                  data-testid="workspace-heading"
                  className="text-lg font-medium text-[var(--v2-text-strong)]"
                >{t("workspace.title")}</h1>
                <StatusPill tone="muted" label={t("workspace.readOnly")} />
              </div>
              <p className="mt-0.5 text-sm text-[var(--v2-text-faint)]">{t("workspace.subtitle")}</p>
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
            <div
              role="alert"
              className="rounded-xl border border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] px-4 py-3 text-sm text-[var(--v2-danger-text)]"
            >
              {workspace.error.message}
            </div>
          )}
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
