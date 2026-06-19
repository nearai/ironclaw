import { useNavigate, useOutletContext, useParams } from "react-router";
import { React, html } from "../../lib/html.js";
import { useT } from "../../lib/i18n.js";
import { Button } from "../../design-system/button.js";
import { EmptyPanel } from "../../design-system/primitives.js";
import { useProjectsOverview } from "./hooks/useProjectsOverview.js";
import { useProjectWorkspace } from "./hooks/useProjectWorkspace.js";
import { useProjectInspector } from "./hooks/useProjectInspector.js";
import { FeedbackBanner } from "./components/feedback-banner.js";
import { ProjectsSummaryStrip } from "./components/projects-summary-strip.js";
import { ProjectsAttentionStrip } from "./components/projects-attention-strip.js";
import { ProjectsGrid } from "./components/projects-grid.js";
import { ProjectWorkspaceShell } from "./components/project-workspace-shell.js";

export function ProjectsPage() {
  const t = useT();
  const navigate = useNavigate();
  const { threadsState } = useOutletContext();
  const { projectId = null, threadId = null } = useParams();
  const [search, setSearch] = React.useState("");
  const [chatFlowError, setChatFlowError] = React.useState(null);

  const overviewState = useProjectsOverview();
  const workspaceState = useProjectWorkspace(projectId);
  const inspectorState = useProjectInspector({ projectId, threadId });

  const filteredProjects = React.useMemo(() => {
    const query = search.trim().toLowerCase();
    if (!query) return overviewState.overview.projects;
    return overviewState.overview.projects.filter((project) =>
      [project.name, project.description, ...(project.goals || [])].some((value) => String(value || "").toLowerCase().includes(query))
    );
  }, [overviewState.overview.projects, search]);

  const selectedOverviewProject = React.useMemo(
    () => overviewState.overview.projects.find((project) => project.id === projectId) || null,
    [overviewState.overview.projects, projectId]
  );

  const handleRefresh = React.useCallback(() => {
    overviewState.invalidate();
    workspaceState.invalidate();
  }, [overviewState, workspaceState]);

  const handleOpenProject = React.useCallback((nextProjectId) => {
    navigate(`/projects/${nextProjectId}`);
  }, [navigate]);

  const handleOpenAttention = React.useCallback((item) => {
    if (item.thread_id) {
      navigate(`/projects/${item.project_id}/threads/${item.thread_id}`);
      return;
    }
    navigate(`/projects/${item.project_id}`);
  }, [navigate]);

  const handleCreateProject = React.useCallback(async () => {
    let nextThreadId = null;
    setChatFlowError(null);
    try {
      nextThreadId = await threadsState.createThread();
    } catch (error) {
      setChatFlowError({
        type: "error",
        message: error.message || t("projects.chatAutoFail"),
      });
    }

    navigate("/chat", {
      state: {
        composerDraft: t("projects.creationDraft"),
        threadId: nextThreadId,
      },
    });
  }, [navigate, threadsState]);

  const handleOpenThread = React.useCallback((nextThreadId) => {
    navigate(`/projects/${projectId}/threads/${nextThreadId}`);
  }, [navigate, projectId]);

  // Start a new conversation scoped to this project: the backend authorizes the
  // caller's access to `projectId` before scoping the thread to it, so the new
  // thread shows up under the project. Then open it in chat.
  const handleStartConversation = React.useCallback(async () => {
    setChatFlowError(null);
    try {
      const newThreadId = await threadsState.createThread(projectId);
      navigate("/chat", { state: { threadId: newThreadId } });
      workspaceState.invalidate();
    } catch (error) {
      setChatFlowError({
        type: "error",
        message: error.message || t("projects.chatAutoFail"),
      });
    }
  }, [navigate, threadsState, projectId, workspaceState, t]);

  const handleClearInspector = React.useCallback(() => {
    navigate(`/projects/${projectId}`);
  }, [navigate, projectId]);

  // Project creation lives in the grid (and its empty-state CTA), not a
  // duplicate top-of-page button.
  const headerActions = html`
    ${projectId && html`<${Button} variant="ghost" onClick=${() => navigate("/projects")}>${t("projects.allProjects")}<//>`}
  `;

  let content = null;

  if (projectId) {
    if (workspaceState.isLoading) {
      content = html`
        <div className="space-y-4">
          ${[1, 2, 3].map((index) => html`<div key=${index} className="v2-skeleton h-48 rounded-[20px]" />`)}
        </div>
      `;
    } else if (workspaceState.error || (!workspaceState.project && !selectedOverviewProject)) {
      content = html`
        <${EmptyPanel}
          title=${t("projects.unavailable")}
          description=${workspaceState.error?.message || t("projects.unavailableDesc")}
        >
          <${Button} variant="secondary" onClick=${() => navigate("/projects")}>${t("projects.returnToProjects")}<//>
        <//>
      `;
    } else {
      content = html`
        <${ProjectWorkspaceShell}
          project=${workspaceState.project || selectedOverviewProject}
          threads=${workspaceState.threads}
          selectedThreadId=${threadId}
          onSelectThread=${handleOpenThread}
          onNewConversation=${handleStartConversation}
          isStartingConversation=${threadsState.isCreating}
        />
      `;
    }
  } else {
    content = overviewState.isLoading
      ? html`
          <div className="space-y-4">
            ${[1, 2, 3].map((index) => html`<div key=${index} className="v2-skeleton h-40 rounded-[20px]" />`)}
          </div>
        `
      : html`
          <${ProjectsGrid}
            projects=${filteredProjects}
            totalProjects=${overviewState.overview.projects.length}
            search=${search}
            onSearchChange=${setSearch}
            onOpenProject=${handleOpenProject}
            onCreateProject=${handleCreateProject}
            isPreparingChat=${threadsState.isCreating}
          />
        `;
  }

  return html`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          <div className="flex flex-wrap justify-end gap-2">
            ${headerActions}
          </div>
          ${overviewState.error && html`
            <div className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">
              ${overviewState.error.message}
            </div>
          `}
          <${FeedbackBanner} result=${chatFlowError} onDismiss=${() => setChatFlowError(null)} />
          <${FeedbackBanner} result=${inspectorState.actionResult} onDismiss=${inspectorState.clearActionResult} />
          ${!projectId &&
          html`
            <${ProjectsSummaryStrip} overview=${overviewState.overview} />
            <${ProjectsAttentionStrip} items=${overviewState.overview.attention} onOpenItem=${handleOpenAttention} />
          `}
          ${content}
        </div>
      </div>
    </div>
  `;
}
