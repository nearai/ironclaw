import { useT } from "../../../lib/i18n";
import {
  Button,
  EmptyPanel,
  Card,
  SearchInput,
  SectionHeader,
  Select,
  Badge,
  Toolbar,
} from "@ironclaw/ui";
import {
  canShowCancel,
  formatJobDate,
  stateLabel,
  statusToneForState,
  truncateJobId,
} from "../lib/jobs-presenters";

export function JobsList({
  jobs,
  totalJobs,
  selectedJobId,
  search,
  onSearchChange,
  stateFilter,
  onStateFilterChange,
  onSelectJob,
  onCancelJob,
  isBusy,
  isRefreshing,
}) {
  const t = useT();
  const FILTERS = [
    { value: "all", label: t("jobs.list.filter.all") },
    { value: "pending", label: t("jobs.list.filter.pending") },
    { value: "in_progress", label: t("jobs.list.filter.inProgress") },
    { value: "completed", label: t("jobs.list.filter.completed") },
    { value: "failed", label: t("jobs.list.filter.failed") },
    { value: "stuck", label: t("jobs.list.filter.stuck") },
  ];

  if (!jobs.length) {
    const hasFilters = Boolean(search.trim()) || stateFilter !== "all";
    return (
      <EmptyPanel
        title={totalJobs && hasFilters ? t("jobs.list.empty.noMatchTitle") : t("jobs.list.empty.noJobsTitle")}
        description={totalJobs && hasFilters
          ? t("jobs.list.empty.noMatchDesc")
          : t("jobs.list.empty.noJobsDesc")}
      />
    );
  }

  return (
    <div className="space-y-5">
      <Card className="p-4 sm:p-5">
        <SectionHeader
          eyebrow={t("jobs.list.explorer")}
          title={t("jobs.list.queueTitle")}
          description={t("jobs.list.queueDesc")}
          actions={
            <div className="flex items-center gap-2 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]">
              <span>{t("jobs.list.visible", { count: jobs.length })}</span>
              <span>/</span>
              <span>{isRefreshing ? t("jobs.list.state.refreshing") : t("jobs.list.state.live")}</span>
            </div>
          }
        />

        <Toolbar className="mt-5">
          <SearchInput
            label={t("jobs.list.searchPlaceholder")}
            value={search}
            onChange={(event) => onSearchChange(event.currentTarget.value)}
            onClear={() => onSearchChange("")}
            clearLabel={t("jobs.list.clearSearch")}
            placeholder={t("jobs.list.searchPlaceholder")}
            className="md:flex-1"
          />
          <div className="md:w-[220px]">
            <Select
              size="sm"
              value={stateFilter}
              onChange={(event) => onStateFilterChange(event.currentTarget.value)}
              aria-label={t("jobs.list.filterLabel")}
            >
              {FILTERS.map((filter) => (<option key={filter.value} value={filter.value}>{filter.label}</option>))}
            </Select>
          </div>
        </Toolbar>
      </Card>

      <div className="grid gap-3">
        {jobs.map((job) => (
          <article
            key={job.id}
            className={[
              "group flex flex-col gap-4 rounded-[18px] border p-5",
              selectedJobId === job.id
                ? "border-signal/35 bg-signal/10"
                : "border-iron-700 bg-iron-800/60 hover:border-signal/30 hover:bg-iron-800/80",
            ].join(" ")}
          >
            <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
              <button onClick={() => onSelectJob(job.id)} className="min-w-0 rounded-md text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)]">
                <div className="flex flex-wrap items-center gap-2">
                  <h3 className="truncate text-lg font-semibold text-iron-100">{job.title || t("jobs.list.untitled")}</h3>
                  <Badge tone={statusToneForState(job.state)} label={stateLabel(job.state)} />
                </div>
                <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">
                  <span>{truncateJobId(job.id)}</span>
                  <span>{t("jobs.list.created", { value: formatJobDate(job.created_at) })}</span>
                  {job.started_at && (<span>{t("jobs.list.started", { value: formatJobDate(job.started_at) })}</span>)}
                </div>
              </button>

              <div className="flex gap-2">
                {canShowCancel(job) && (
                  <Button
                    variant="secondary"
                    className="h-9 px-3 text-xs"
                    disabled={isBusy}
                    onClick={() => onCancelJob(job.id)}
                  >
                    {t("jobs.action.cancel")}
                  </Button>
                )}
                <Button variant="ghost" className="h-9 px-3 text-xs" onClick={() => onSelectJob(job.id)}>{t("jobs.action.open")}</Button>
              </div>
            </div>
          </article>
        ))}
      </div>
    </div>
  );
}
