import { StatusPill, Text } from "@ironclaw/design-system";
import { formatRoutineDate } from "../lib/routines-presenters";

function runTone(status) {
  if (status === "ok") return "success";
  if (status === "running") return "warning";
  return "danger";
}

export function RoutineRecentRuns({ runs }) {
  if (!runs?.length) {
    return (
      <div className="rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)]/40 p-4 text-sm text-[var(--v2-text-muted)]">
        No runs recorded yet.
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {runs.map(
        (run) => (
          <div key={run.id} className="rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)]/40 p-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <StatusPill tone={runTone(run.status)} label={run.status} />
              <Text variant="eyebrow" tone="faint">
                {formatRoutineDate(run.started_at)}
              </Text>
            </div>
            {run.result_summary &&
            (<p className="mt-3 text-sm leading-6 text-[var(--v2-text-muted)]">{run.result_summary}</p>)}
          </div>
        )
      )}
    </div>
  );
}
