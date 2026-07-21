import { Button } from "@ironclaw/design-system";
import { Panel, StatusPill } from "@ironclaw/design-system";
import {
  JOB_DETAIL_TABS,
  canShowCancel,
  canShowRestart,
  formatJobDate,
  jobSecondaryMeta,
  stateLabel,
  statusToneForState,
  truncateJobId,
} from "../lib/jobs-presenters";

export function JobDetailShell({
  job,
  activeTab,
  onTabChange,
  onBack,
  onCancel,
  onRestart,
  isBusy,
  children,
}) {
  return (
    <div className="space-y-5">
      <Panel className="p-5 sm:p-6">
        <div className="flex flex-col gap-5 xl:flex-row xl:items-start xl:justify-between">
          <div className="min-w-0">
            <button onClick={onBack} className="text-sm text-[var(--v2-accent-text)] hover:text-[var(--v2-text-strong)]">Back to all jobs</button>
            <div className="mt-3 flex flex-wrap items-center gap-3">
              <h2 className="text-3xl font-medium tracking-tight text-[var(--v2-text-strong)]">{job.title || "Untitled job"}</h2>
              <StatusPill tone={statusToneForState(job.state)} label={stateLabel(job.state)} />
            </div>
            <div className="mt-3 flex flex-wrap gap-x-4 gap-y-2 font-mono text-[11px] uppercase tracking-[var(--v2-tracking-caps)] text-[var(--v2-text-muted)]">
              <span>{truncateJobId(job.id)}</span>
              <span>created {formatJobDate(job.created_at)}</span>
              {jobSecondaryMeta(job) && (<span>{jobSecondaryMeta(job)}</span>)}
            </div>
          </div>

          <div className="flex flex-wrap gap-2">
            {job.browse_url && (
              <a
                href={job.browse_url}
                target="_blank"
                rel="noreferrer noopener"
                className="v2-button inline-flex h-10 items-center rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 text-sm font-medium text-[var(--v2-text-strong)] hover:border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] hover:bg-[var(--v2-accent-soft)]"
              >
                Browse files
              </a>
            )}
            {canShowCancel(job) && (
              <Button variant="secondary" disabled={isBusy} onClick={() => onCancel(job.id)}>Cancel</Button>
            )}
            {canShowRestart(job) && (
              <Button variant="primary" disabled={isBusy} onClick={() => onRestart(job.id)}>Restart</Button>
            )}
          </div>
        </div>
      </Panel>

      <div className="flex flex-wrap gap-2">
        {JOB_DETAIL_TABS.map((tab) => (
          <button
            key={tab.id}
            onClick={() => onTabChange(tab.id)}
            className={[
              "v2-button rounded-full border px-4 py-2 text-sm",
              activeTab === tab.id
                ? "border-[color-mix(in_srgb,var(--v2-accent)_35%,transparent)] bg-[var(--v2-accent-soft)] text-[var(--v2-text-strong)]"
                : "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_25%,var(--v2-panel-border))] hover:text-[var(--v2-text-strong)]",
            ].join(" ")}
          >
            {tab.label}
          </button>
        ))}
      </div>

      {children}
    </div>
  );
}
