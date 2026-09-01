import { useT } from "../../../lib/i18n";
import { Button } from "../../../design-system/button";
import { EmptyPanel, Panel, StatusPill } from "../../../design-system/primitives";
import {
  formatProjectDate,
  formatProjectRole,
  formatProjectState,
  projectStateTone,
} from "../lib/projects-presenters";

function ProjectCard({ project, onOpen, t }) {
  return (
    <article
      data-testid="project-card"
      data-project-id={project.id}
      onClick={() => onOpen(project.id)}
      role="button"
      tabIndex={0}
      onKeyDown={(event) => {
        // Only act on key events targeting the card itself. The nested
        // "Open workspace" button is also focusable, and its Enter/Space
        // keydown bubbles up here — without this guard, keyboard activation
        // on that button would fire onOpen twice.
        if (event.currentTarget !== event.target) return;
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onOpen(project.id);
        }
      }}
      className="group cursor-pointer rounded-xl border border-[var(--v2-panel-border)] bg-iron-800/60 p-5 transition hover:border-[color-mix(in_srgb,var(--v2-accent)_44%,var(--v2-panel-border))] hover:bg-iron-800/80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/40"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="truncate font-serif text-title-lg font-semibold tracking-[-0.03em] text-[var(--v2-text-strong)]">{project.name}</h3>
          <p className="mt-2 line-clamp-3 text-ui leading-6 text-[var(--v2-text-muted)]">
            {project.description || t("projects.noDescription")}
          </p>
        </div>
        <StatusPill
          tone={projectStateTone(project.state)}
          label={formatProjectState(project.state, t)}
        />
      </div>

      {project.goals?.length
        ? (
            <div className="mt-4 flex flex-wrap gap-2">
              {project.goals.slice(0, 3).map((goal, index) => (
                <span key={index} className="rounded-full border border-[var(--v2-panel-border)] px-3 py-1 text-ui-sm text-[var(--v2-text-strong)]">
                  {goal}
                </span>
              ))}
            </div>
          )
        : null}

      <div className="mt-5 flex items-center justify-between gap-3">
        <div className="flex min-w-0 flex-wrap items-center gap-2 text-ui text-[var(--v2-text-muted)]">
          <StatusPill tone="muted" label={formatProjectRole(project.role, t)} />
          <time
            data-testid="project-updated-at"
            dateTime={project.updated_at || undefined}
            className="text-ui-sm uppercase tracking-[0.16em] text-[var(--v2-text-muted)]"
          >
            {t("projects.snapshot.updated", { date: formatProjectDate(project.updated_at, t) })}
          </time>
        </div>
        <Button
          data-testid="project-open-workspace"
          variant="secondary"
          onClick={(event) => {
            event.stopPropagation();
            onOpen(project.id);
          }}
        >{t("projects.openWorkspace")}</Button>
      </div>
    </article>
  );
}

function GeneralProjectCard({ project, onOpen, t }) {
  return (
    <Panel
      data-testid="project-card"
      data-project-id={project.id}
      onClick={() => onOpen(project.id)}
      role="button"
      tabIndex={0}
      onKeyDown={(event) => {
        // Only act on key events targeting the card itself. The nested
        // "Open workspace" button is also focusable, and its Enter/Space
        // keydown bubbles up here — without this guard, keyboard activation
        // on that button would fire onOpen twice.
        if (event.currentTarget !== event.target) return;
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onOpen(project.id);
        }
      }}
      className="cursor-pointer overflow-hidden p-5 transition hover:border-[color-mix(in_srgb,var(--v2-accent)_44%,var(--v2-panel-border))] sm:p-6"
    >
      <div className="flex flex-col gap-6 xl:flex-row xl:items-end xl:justify-between">
        <div className="max-w-3xl">
          <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-[var(--v2-accent-text)]">{t("projects.general.label")}</div>
          <h2 className="mt-3 font-serif text-4xl font-semibold tracking-[-0.04em] text-[var(--v2-text-strong)]">{t("projects.general.title")}</h2>
          <p className="mt-3 text-ui leading-6 text-[var(--v2-text-strong)]">
            {t("projects.general.desc")}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <StatusPill
            tone={projectStateTone(project.state)}
            label={formatProjectState(project.state, t)}
          />
          <StatusPill tone="muted" label={formatProjectRole(project.role, t)} />
          <time
            data-testid="project-updated-at"
            dateTime={project.updated_at || undefined}
            className="text-ui-sm uppercase tracking-[0.16em] text-[var(--v2-text-muted)]"
          >
            {t("projects.snapshot.updated", { date: formatProjectDate(project.updated_at, t) })}
          </time>
          <Button
            data-testid="project-open-workspace"
            variant="secondary"
            onClick={(event) => {
              event.stopPropagation();
              onOpen(project.id);
            }}
          >{t("projects.openGeneralWorkspace")}</Button>
        </div>
      </div>
    </Panel>
  );
}

export function ProjectsGrid({
  projects,
  totalProjects,
  search,
  onSearchChange,
  onOpenProject,
  onCreateProject,
  isPreparingChat,
}) {
  const t = useT();
  const defaultProject = projects.find((project) => project.name === "default");
  const scopedProjects = projects.filter((project) => project.name !== "default");

  if (!totalProjects) {
    return (
      <EmptyPanel
        title={t("projects.empty.noneTitle")}
        description={t("projects.empty.noneDesc")}
      >
        <Button onClick={onCreateProject}>{t("projects.createFromChat")}</Button>
      </EmptyPanel>
    );
  }

  return (
    <div data-testid="projects-grid" className="space-y-5">
      {defaultProject && (<GeneralProjectCard project={defaultProject} onOpen={onOpenProject} t={t} />)}

      <Panel className="p-4 sm:p-5">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--v2-text-muted)]">{t("projects.explorer")}</div>
            <h2 className="mt-2 font-serif text-display font-semibold tracking-[-0.04em] text-[var(--v2-text-strong)]">{t("projects.scoped.title")}</h2>
            <p className="mt-2 max-w-2xl text-ui leading-6 text-[var(--v2-text-muted)]">
              {t("projects.scoped.desc")}
            </p>
          </div>
          <div className="flex gap-2">
            <input
              data-testid="projects-search-input"
              value={search}
              onInput={(event) => onSearchChange(event.currentTarget.value)}
              placeholder={t("projects.searchPlaceholder")}
              className="h-11 min-w-[220px] rounded-md border border-[var(--v2-panel-border)] bg-[color-mix(in_srgb,var(--v2-canvas-strong)_88%,transparent)] px-3 text-ui text-[var(--v2-text-strong)] outline-none focus:border-[color-mix(in_srgb,var(--v2-accent)_44%,var(--v2-panel-border))]"
            />
            <Button onClick={onCreateProject}>{isPreparingChat ? t("projects.preparingChat") : t("projects.newProject")}</Button>
          </div>
        </div>
      </Panel>

      {scopedProjects.length
        ? (<div className="grid gap-4 xl:grid-cols-2 2xl:grid-cols-3">
            {scopedProjects.map((project) => (<ProjectCard key={project.id} project={project} onOpen={onOpenProject} t={t} />))}
          </div>)
        : !projects.length
          ? (
              <EmptyPanel
                title={t("projects.empty.noMatchTitle")}
                description={t("projects.empty.noMatchDesc")}
              />
            )
        : (
            <EmptyPanel
              title={t("projects.scoped.onlyGeneralTitle")}
              description={t("projects.scoped.onlyGeneralDesc")}
            >
              <Button onClick={onCreateProject}>{isPreparingChat ? t("projects.preparingChat") : t("projects.startProject")}</Button>
            </EmptyPanel>
          )}
    </div>
  );
}
