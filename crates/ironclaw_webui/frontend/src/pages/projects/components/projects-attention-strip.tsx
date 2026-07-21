import { useT } from "../../../lib/i18n";
import { Panel, StatusPill } from "@ironclaw/design-system";

function attentionTone(item) {
  return item?.type === "failure" ? "danger" : "warning";
}

function attentionLabel(item, t) {
  return item?.type === "failure" ? t("projects.attention.failure") : t("projects.attention.gate");
}

export function ProjectsAttentionStrip({ items, onOpenItem }) {
  const t = useT();
  if (!items?.length) return null;

  return (
    <Panel className="overflow-hidden border-[color-mix(in_srgb,var(--v2-warning-text)_10%,var(--v2-panel-border))] p-0">
      <div className="border-b border-[color-mix(in_srgb,var(--v2-warning-text)_10%,var(--v2-panel-border))] px-5 py-4 sm:px-6">
        <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-[var(--v2-warning-text)]">{t("projects.attention.title")}</div>
        <p className="mt-2 max-w-[70ch] text-sm leading-6 text-[var(--v2-text)]">
          {t("projects.attention.desc")}
        </p>
      </div>
      <div className="grid gap-3 p-4 sm:p-5 xl:grid-cols-2">
        {items.map((item) => (
          <button
            key={`${item.project_id}-${item.thread_id || item.message}`}
            onClick={() => onOpenItem(item)}
            className="group rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)]/55 p-4 text-left hover:border-[var(--v2-accent)]/30 hover:bg-[var(--v2-surface-muted)]"
          >
            <div className="flex items-start justify-between gap-3">
              <div>
                <div className="text-sm font-medium text-[var(--v2-text-strong)]">{item.project_name}</div>
                <div className="mt-1 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]">
                  {item.thread_id
                    ? t("projects.attention.threadLabel", { id: String(item.thread_id).slice(0, 8) })
                    : t("projects.attention.projectLabel")}
                </div>
              </div>
              <StatusPill tone={attentionTone(item)} label={attentionLabel(item, t)} />
            </div>
            <p className="mt-3 text-sm leading-6 text-[var(--v2-text)]">{item.message}</p>
            <div className="mt-4 text-xs uppercase tracking-[0.16em] text-[var(--v2-accent-text)] group-hover:text-[var(--v2-text-strong)]">
              {t("projects.attention.openProject")}
            </div>
          </button>
        ))}
      </div>
    </Panel>
  );
}
