import { EmptyPanel, FlowList, Panel, StatusPill } from "@ironclaw/design-system";
import { MarkdownRenderer } from "../../chat/components/markdown-renderer";
import {
  formatDuration,
  formatJobDate,
  stateLabel,
  statusToneForState,
} from "../lib/jobs-presenters";

function MetaItem({ label, value }) {
  return (
    <div className="border-t border-[var(--v2-panel-border)] py-4">
      <div className="font-mono text-[11px] uppercase tracking-[var(--v2-tracking-caps)] text-[var(--v2-text-muted)]">{label}</div>
      <div className="mt-2 text-sm leading-6 text-[var(--v2-text-strong)]">{value || "Not available"}</div>
    </div>
  );
}

export function JobOverviewTab({ job }) {
  const transitions = (job.transitions || []).map((transition) => ({
    title: `${stateLabel(transition.from)} -> ${stateLabel(transition.to)}`,
    description: [formatJobDate(transition.timestamp), transition.reason].filter(Boolean).join(" / "),
  }));

  return (
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(320px,0.8fr)]">
      <Panel className="p-5 sm:p-6">
        <div className="flex items-center justify-between gap-4">
          <div>
            <div className="font-mono text-[11px] uppercase tracking-[var(--v2-tracking-caps)] text-[var(--v2-text-muted)]">Execution context</div>
            <h3 className="mt-2 text-xl font-medium text-[var(--v2-text-strong)]">Timing, state, and runtime shape</h3>
          </div>
          <StatusPill tone={statusToneForState(job.state)} label={stateLabel(job.state)} />
        </div>

        <div className="mt-5 grid gap-x-6 md:grid-cols-2">
          <MetaItem label="Created" value={formatJobDate(job.created_at)} />
          <MetaItem label="Started" value={formatJobDate(job.started_at)} />
          <MetaItem label="Completed" value={formatJobDate(job.completed_at)} />
          <MetaItem label="Duration" value={formatDuration(job.elapsed_secs)} />
          <MetaItem label="Kind" value={job.job_kind ? `${job.job_kind} job` : null} />
          <MetaItem label="Mode" value={job.job_mode || "Default worker"} />
        </div>
      </Panel>

      <div className="space-y-5">
        <Panel className="p-5 sm:p-6">
          <div className="font-mono text-[11px] uppercase tracking-[var(--v2-tracking-caps)] text-[var(--v2-text-muted)]">Description</div>
          <h3 className="mt-2 text-xl font-medium text-[var(--v2-text-strong)]">Mission brief</h3>
          {job.description
            ? (<MarkdownRenderer content={job.description} className="mt-4 text-sm leading-7 text-[var(--v2-text)]" />)
            : (<p className="mt-4 text-sm leading-6 text-[var(--v2-text-muted)]">This job did not record a long-form description.</p>)}
        </Panel>

        {transitions.length
          ? (
              <Panel className="p-5 sm:p-6">
                <div className="font-mono text-[11px] uppercase tracking-[var(--v2-tracking-caps)] text-[var(--v2-text-muted)]">Transitions</div>
                <h3 className="mt-2 text-xl font-medium text-[var(--v2-text-strong)]">State timeline</h3>
                <div className="mt-3">
                  <FlowList items={transitions} />
                </div>
              </Panel>
            )
          : (
              <EmptyPanel
                title="No state history yet"
                description="Transitions appear here once the job advances or records a recovery event."
              />
            )}
      </div>
    </div>
  );
}
