import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { Button } from "../../../design-system/button.js";
import { EmptyPanel, Panel, StatusPill } from "../../../design-system/primitives.js";
import {
  compactCount,
  formatCurrency,
  formatProjectRelativeTime,
  healthTone,
} from "../lib/projects-presenters.js";

function ProjectCard({ project, onOpen, t }) {
  return html`
    <article
      data-testid="project-card"
      data-project-id=${project.id}
      onClick=${() => onOpen(project.id)}
      role="button"
      tabIndex=${0}
      onKeyDown=${(event) => {
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
          <h3 className="truncate font-serif text-2xl font-semibold tracking-[-0.03em] text-iron-100">${project.name}</h3>
          <p className="mt-2 line-clamp-3 text-sm leading-6 text-iron-300">
            ${project.description || t("projects.noDescription")}
          </p>
        </div>
        <${StatusPill} tone=${healthTone(project.health)} label=${project.health || "unknown"} />
      </div>

      ${project.goals?.length
        ? html`
            <div className="mt-4 flex flex-wrap gap-2">
              ${project.goals.slice(0, 3).map((goal, index) => html`
                <span key=${index} className="rounded-full border border-iron-700 px-3 py-1 text-xs text-iron-200">
                  ${goal}
                </span>
              `)}
            </div>
          `
        : null}

      <div className="mt-5 grid gap-3 sm:grid-cols-2">
        <div className="rounded-2xl border border-iron-700 bg-iron-950/55 p-3">
          <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${t("projects.card.runtime")}</div>
          <div className="mt-2 text-sm text-iron-100">
            ${t("projects.card.threadsToday", { count: compactCount(project.threads_today || 0, "thread") })}
          </div>
        </div>
        <div className="rounded-2xl border border-iron-700 bg-iron-950/55 p-3">
          <div className="font-mono text-[10px] uppercase tracking-[0.16em] text-iron-300">${t("projects.card.risk")}</div>
          <div className="mt-2 text-sm text-iron-100">${compactCount(project.pending_gates || 0, "gate")}</div>
          <div className="mt-1 text-xs text-iron-300">
            ${t("projects.card.failures24h", { count: compactCount(project.failures_24h || 0, "failure") })}
          </div>
        </div>
      </div>

      <div className="mt-5 flex items-center justify-between gap-3">
        <div className="text-sm text-iron-300">
          <div>${t("projects.card.spendToday", { value: formatCurrency(project.cost_today_usd || 0) })}</div>
          <div className="mt-1 text-xs uppercase tracking-[0.16em] text-iron-500">${formatProjectRelativeTime(project.last_activity)}</div>
        </div>
        <${Button}
          data-testid="project-open-workspace"
          variant="secondary"
          onClick=${(event) => {
            event.stopPropagation();
            onOpen(project.id);
          }}
        >${t("projects.openWorkspace")}<//>
      </div>
    </article>
  `;
}

function GeneralProjectCard({ project, onOpen, t }) {
  return html`
    <${Panel}
      data-testid="project-card"
      data-project-id=${project.id}
      onClick=${() => onOpen(project.id)}
      role="button"
      tabIndex=${0}
      onKeyDown=${(event) => {
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
      <div className="flex flex-col gap-6 xl:flex-row xl:items-end xl:justify-between">
        <div className="max-w-3xl">
          <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-signal">${t("projects.general.label")}</div>
          <h2 className="mt-3 font-serif text-4xl font-semibold tracking-[-0.04em] text-iron-100">${t("projects.general.title")}</h2>
          <p className="mt-3 text-sm leading-6 text-iron-200">
            ${t("projects.general.desc")}
          </p>
        </div>
        <div className="flex flex-wrap gap-3">
          <div className="rounded-2xl border border-iron-700 bg-iron-950/55 px-4 py-3 text-sm text-iron-200">
            ${compactCount(project.threads_today || 0, "thread")} today
          </div>
          <${Button}
            data-testid="project-open-workspace"
            variant="secondary"
            onClick=${(event) => {
              event.stopPropagation();
              onOpen(project.id);
            }}
          >${t("projects.openGeneralWorkspace")}<//>
        </div>
      </div>
    <//>
  `;
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
    return html`
      <${EmptyPanel}
        title=${t("projects.empty.noneTitle")}
        description=${t("projects.empty.noneDesc")}
      >
        <${Button} onClick=${onCreateProject}>${t("projects.createFromChat")}<//>
      <//>
    `;
  }

  return html`
    <div data-testid="projects-grid" className="space-y-5">
      ${defaultProject && html`<${GeneralProjectCard} project=${defaultProject} onOpen=${onOpenProject} t=${t} />`}

      <${Panel} className="p-4 sm:p-5">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">${t("projects.explorer")}</div>
            <h2 className="mt-2 font-serif text-3xl font-semibold tracking-[-0.04em] text-iron-100">${t("projects.scoped.title")}</h2>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-iron-300">
              ${t("projects.scoped.desc")}
            </p>
          </div>
          <div className="flex gap-2">
            <input
              data-testid="projects-search-input"
              value=${search}
              onInput=${(event) => onSearchChange(event.target.value)}
              placeholder=${t("projects.searchPlaceholder")}
              className="h-11 min-w-[220px] rounded-md border border-iron-700 bg-iron-950/90 px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
            />
            <${Button} onClick=${onCreateProject}>${isPreparingChat ? t("projects.preparingChat") : t("projects.newProject")}<//>
          </div>
        </div>
      <//>

      ${scopedProjects.length
        ? html`<div className="grid gap-4 xl:grid-cols-2 2xl:grid-cols-3">
            ${scopedProjects.map((project) => html`<${ProjectCard} key=${project.id} project=${project} onOpen=${onOpenProject} t=${t} />`)}
          </div>`
        : !projects.length
          ? html`
              <${EmptyPanel}
                title=${t("projects.empty.noMatchTitle")}
                description=${t("projects.empty.noMatchDesc")}
              />
            `
        : html`
            <${EmptyPanel}
              title=${t("projects.scoped.onlyGeneralTitle")}
              description=${t("projects.scoped.onlyGeneralDesc")}
            >
              <${Button} onClick=${onCreateProject}>${isPreparingChat ? t("projects.preparingChat") : t("projects.startProject")}<//>
            <//>
          `}
    </div>
  `;
}
