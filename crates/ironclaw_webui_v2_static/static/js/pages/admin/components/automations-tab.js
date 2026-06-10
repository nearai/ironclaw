import { Icon } from "../../../design-system/icons.js";
import { Panel, StatusPill } from "../../../design-system/primitives.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";

const SHARED_SCOPE_ROWS = [
  {
    id: "project",
    labelKey: "admin.automations.scope.project",
    descriptionKey: "admin.automations.scope.projectDescription",
    statusKey: "admin.automations.scope.adminOnly",
    tone: "signal",
  },
  {
    id: "personal",
    labelKey: "admin.automations.scope.personal",
    descriptionKey: "admin.automations.scope.personalDescription",
    statusKey: "admin.automations.scope.personalTab",
    tone: "muted",
  },
];

export function AdminAutomationsTab() {
  const t = useT();

  return html`
    <div className="space-y-5">
      <${Panel} className="p-5 sm:p-6">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
          <div className="min-w-0">
            <h2 className="text-xl font-semibold text-iron-100">
              ${t("admin.automations.title")}
            </h2>
            <p className="mt-2 max-w-3xl text-sm leading-6 text-iron-300">
              ${t("admin.automations.description")}
            </p>
          </div>
          <${StatusPill}
            tone="warning"
            label=${t("admin.automations.status.routePending")}
          />
        </div>
      <//>

      <div className="grid gap-4 lg:grid-cols-2">
        ${SHARED_SCOPE_ROWS.map(
          (row) => html`
            <${Panel} key=${row.id} className="p-5">
              <div className="flex items-start gap-3">
                <span
                  className="grid h-9 w-9 shrink-0 place-items-center rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-signal"
                >
                  <${Icon}
                    name=${row.id === "project" ? "shield" : "calendar"}
                    className="h-4 w-4"
                  />
                </span>
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <h3 className="text-sm font-semibold text-iron-100">
                      ${t(row.labelKey)}
                    </h3>
                    <${StatusPill} tone=${row.tone} label=${t(row.statusKey)} />
                  </div>
                  <p className="mt-2 text-sm leading-6 text-iron-300">
                    ${t(row.descriptionKey)}
                  </p>
                </div>
              </div>
            <//>
          `
        )}
      </div>

      <${Panel} className="p-5 sm:p-6">
        <div className="mb-4 flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
          <div>
            <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
              ${t("admin.automations.sharedDefaults")}
            </h3>
            <p className="mt-2 text-sm leading-6 text-iron-300">
              ${t("admin.automations.sharedDefaultsDescription")}
            </p>
          </div>
          <${StatusPill}
            tone="muted"
            label=${t("admin.automations.status.notConfigured")}
          />
        </div>

        <div className="rounded-[12px] border border-dashed border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-4">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div className="min-w-0">
              <div className="text-sm font-medium text-iron-100">
                ${t("admin.automations.empty.title")}
              </div>
              <div className="mt-1 text-sm leading-6 text-iron-300">
                ${t("admin.automations.empty.description")}
              </div>
            </div>
            <${StatusPill}
              tone="warning"
              label=${t("admin.automations.status.needsProjectApi")}
            />
          </div>
        </div>
      <//>
    </div>
  `;
}
