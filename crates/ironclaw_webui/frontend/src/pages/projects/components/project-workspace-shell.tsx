// @ts-nocheck
import { ProjectActivityColumn } from "./project-activity-column";
import { ProjectFilesystemPanel } from "./project-filesystem-panel";

// Pick a representative project thread for the filesystem panel. Every thread in
// the project shares the same bind-mounted `/workspace` folder, so the most
// recent one is enough to list the project's scoped filesystem.
function representativeThreadId(threads) {
  const sorted = [...(threads || [])].sort(
    (a, b) => new Date(b.updated_at || b.created_at) - new Date(a.updated_at || a.created_at)
  );
  return sorted[0]?.id || null;
}

export function ProjectWorkspaceShell({
  project,
  threads,
  selectedThreadId,
  onSelectThread,
  onNewConversation,
  isStartingConversation,
}) {
  const fsThreadId = representativeThreadId(threads);

  return (
    <div
      data-testid="project-workspace"
      data-project-id={project.id}
      className="grid gap-5 xl:grid-cols-[minmax(0,1.15fr)_minmax(340px,0.85fr)]"
    >
      <div className="space-y-5">
        <div className="min-w-0">
          <h2 data-testid="project-workspace-title" className="text-2xl font-semibold tracking-tight text-white">{project.name}</h2>
          {project.description
            ? (<p className="mt-1 text-sm leading-6 text-iron-300">{project.description}</p>)
            : null}
        </div>

        <ProjectActivityColumn
          threads={threads}
          selectedThreadId={selectedThreadId}
          onSelectThread={onSelectThread}
          onNewConversation={onNewConversation}
          isStartingConversation={isStartingConversation}
        />
      </div>

      <ProjectFilesystemPanel threadId={fsThreadId} />
    </div>
  );
}
