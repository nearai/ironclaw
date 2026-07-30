import { useT } from "../../../lib/i18n";
import { Callout, Card, Badge } from "@ironclaw/ui";

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
    <Card className="p-4 sm:p-5">
      <Callout tone="warning" title={t("projects.attention.title")}>
        {t("projects.attention.desc")}
      </Callout>
      <div className="mt-4 grid gap-3 xl:grid-cols-2">
        {items.map((item) => (
          <button
            key={`${item.project_id}-${item.thread_id || item.message}`}
            onClick={() => onOpenItem(item)}
            className="group rounded-2xl border border-white/10 bg-iron-950/55 p-4 text-left transition-colors hover:border-signal/30 hover:bg-white/[0.05] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)]"
          >
            <div className="flex items-start justify-between gap-3">
              <div>
                <div className="text-sm font-semibold text-white">{item.project_name}</div>
                <div className="mt-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
                  {item.thread_id
                    ? t("projects.attention.threadLabel", { id: String(item.thread_id).slice(0, 8) })
                    : t("projects.attention.projectLabel")}
                </div>
              </div>
              <Badge tone={attentionTone(item)} label={attentionLabel(item, t)} />
            </div>
            <p className="mt-3 text-sm leading-6 text-iron-200">{item.message}</p>
            <div className="mt-4 text-xs uppercase tracking-[0.16em] text-signal group-hover:text-white">
              {t("projects.attention.openProject")}
            </div>
          </button>
        ))}
      </div>
    </Card>
  );
}
