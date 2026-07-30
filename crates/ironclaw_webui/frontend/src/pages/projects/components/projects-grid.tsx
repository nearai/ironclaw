import { useT } from "../../../lib/i18n";
import {
  Button,
  EmptyPanel,
  Card,
  SearchInput,
  SectionHeader,
  Badge,
  Toolbar,
  ToolbarGroup,
} from "@ironclaw/ui";
import {
  formatCurrency,
  formatProjectHealth,
  formatProjectRelativeTime,
  healthTone,
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
      className="group cursor-pointer rounded-xl border border-iron-700 bg-iron-800/60 p-5 transition hover:border-signal/30 hover:bg-iron-800/80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/40"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="truncate font-serif text-2xl font-semibold tracking-[-0.03em] text-iron-100">{project.name}</h3>
          <p className="mt-2 line-clamp-3 text-sm leading-6 text-iron-300">
            {project.description || t("projects.noDescription")}
          </p>
        </div>
        <Badge tone={healthTone(project.health)} label={formatProjectHealth(project.health, t)} />
      </div>

      {project.goals?.length
        ? (
            <div className="mt-4 flex flex-wrap gap-2">
              {project.goals.slice(0, 3).map((goal, index) => (
                <span key={index} className="rounded-full border border-iron-700 px-3 py-1 text-xs text-iron-200">
                  {goal}
                </span>
              ))}
            </div>
          )
        : null}

      <div className="mt-5 grid gap-3 sm:grid-cols-2">
        <div className="rounded-2xl border border-iron-700 bg-iron-950/55 p-3">
          <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">{t("projects.card.runtime")}</div>
          <div className="mt-2 text-sm text-iron-100">
            {t("projects.card.threadsToday", { count: project.threads_today || 0 })}
          </div>
        </div>
        <div className="rounded-2xl border border-iron-700 bg-iron-950/55 p-3">
          <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">{t("projects.card.risk")}</div>
          <div className="mt-2 text-sm text-iron-100">{t("projects.card.pendingGates", { count: project.pending_gates || 0 })}</div>
          <div className="mt-1 text-xs text-iron-300">
            {t("projects.card.failures24h", { count: project.failures_24h || 0 })}
          </div>
        </div>
      </div>

      <div className="mt-5 flex items-center justify-between gap-3">
        <div className="text-sm text-iron-300">
          <div>{t("projects.card.spendToday", { value: formatCurrency(project.cost_today_usd || 0) })}</div>
          <div className="mt-1 text-xs uppercase tracking-[0.16em] text-[var(--v2-text-muted)]">{formatProjectRelativeTime(project.last_activity, t)}</div>
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
    <Card
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
      className="cursor-pointer overflow-hidden p-5 transition hover:border-signal/30 sm:p-6"
    >
      <SectionHeader
        eyebrow={t("projects.general.label")}
        title={t("projects.general.title")}
        description={t("projects.general.desc")}
        actions={
          <>
            <div className="rounded-2xl border border-iron-700 bg-iron-950/55 px-4 py-3 text-sm text-iron-200">
              {t("projects.general.threadsToday", { count: project.threads_today || 0 })}
            </div>
            <Button
              data-testid="project-open-workspace"
              variant="secondary"
              onClick={(event) => {
                event.stopPropagation();
                onOpen(project.id);
              }}
            >{t("projects.openGeneralWorkspace")}</Button>
          </>
        }
      />
    </Card>
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

      <Card className="p-4 sm:p-5">
        <SectionHeader
          eyebrow={t("projects.explorer")}
          title={t("projects.scoped.title")}
          description={t("projects.scoped.desc")}
          actions={
            <Toolbar>
              <SearchInput
                data-testid="projects-search-input"
                label={t("projects.searchPlaceholder")}
                value={search}
                onInput={(event) => onSearchChange(event.currentTarget.value)}
                placeholder={t("projects.searchPlaceholder")}
                className="min-w-[220px]"
              />
              <ToolbarGroup>
                <Button onClick={onCreateProject}>{isPreparingChat ? t("projects.preparingChat") : t("projects.newProject")}</Button>
              </ToolbarGroup>
            </Toolbar>
          }
        />
      </Card>

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
