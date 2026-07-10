// @ts-nocheck
import { useNavigate, useParams } from "react-router";
import { Button } from "../../design-system/button";
import { EmptyPanel } from "../../design-system/primitives";
import React from "react";
import { useT } from "../../lib/i18n";
import { JobActivityTab } from "./components/job-activity-tab";
import { JobDetailShell } from "./components/job-detail-shell";
import { JobFilesTab } from "./components/job-files-tab";
import { JobOverviewTab } from "./components/job-overview-tab";
import { JobsList } from "./components/jobs-list";
import { JobsSummaryStrip } from "./components/jobs-summary-strip";
import { useJobDetail } from "./hooks/useJobDetail";
import { useJobFiles } from "./hooks/useJobFiles";
import { useJobs } from "./hooks/useJobs";

function FeedbackBanner({ result, onDismiss }) {
  const t = useT();
  if (!result) return null;

  const tone = {
    success: "border-mint/30 bg-mint/10 text-mint",
    error: "border-red-400/30 bg-red-500/10 text-red-200",
    info: "border-signal/30 bg-signal/10 text-signal",
  };

  return (
    <div
      className={[
        "flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",
        tone[result.type] || tone.info,
      ].join(" ")}
    >
      <span className="min-w-0 flex-1">{result.message}</span>
      <button
        onClick={onDismiss}
        className="shrink-0 opacity-70 hover:opacity-100"
      >
        {t("jobs.dismiss")}
      </button>
    </div>
  );
}

export function JobsPage() {
  const t = useT();
  const navigate = useNavigate();
  const { jobId = null } = useParams();
  const [search, setSearch] = React.useState("");
  const [stateFilter, setStateFilter] = React.useState("all");
  const [activeTab, setActiveTab] = React.useState(
    jobId ? "activity" : "overview"
  );

  const jobsState = useJobs();
  const detailState = useJobDetail(jobId);
  const filesState = useJobFiles(detailState.job);

  React.useEffect(() => {
    setActiveTab(jobId ? "activity" : "overview");
  }, [jobId]);

  const filteredJobs = React.useMemo(() => {
    const query = search.trim().toLowerCase();
    return jobsState.jobs.filter((job) => {
      const matchesSearch =
        !query ||
        job.title.toLowerCase().includes(query) ||
        job.id.toLowerCase().includes(query);
      const matchesState = stateFilter === "all" || job.state === stateFilter;
      return matchesSearch && matchesState;
    });
  }, [jobsState.jobs, search, stateFilter]);

  const handleOpenJob = React.useCallback(
    (nextJobId) => navigate(`/jobs/${nextJobId}`),
    [navigate]
  );

  const handleCancel = React.useCallback(
    async (targetJobId) => {
      try {
        await jobsState.cancelJob({ jobId: targetJobId });
      } catch {
        // Result state is handled in the mutation hooks.
      }
    },
    [jobsState]
  );

  const handleRestart = React.useCallback(
    async (targetJobId) => {
      try {
        const response = await jobsState.restartJob({ jobId: targetJobId });
        if (response?.new_job_id) {
          navigate(`/jobs/${response.new_job_id}`);
        }
      } catch {
        // Result state is handled in the mutation hooks.
      }
    },
    [jobsState, navigate]
  );

  const headerActions = (
    <>
    {jobId &&
    (<Button variant="ghost" onClick={() => navigate("/jobs")}
      >{t("jobs.allJobs")}</Button>)}
    </>
  );

  let detailContent = null;

  if (jobId) {
    if (detailState.isLoading) {
      detailContent = (
        <div className="space-y-4">
          {[1, 2, 3].map(
            (i) =>
              (<div key={i} className="v2-skeleton h-32 rounded-[18px]" />)
          )}
        </div>
      );
    } else if (detailState.error || !detailState.job) {
      detailContent = (
        <EmptyPanel
          title={t("jobs.unavailable")}
          description={detailState.error?.message || t("jobs.unavailableDesc")}
        >
          <Button variant="secondary" onClick={() => navigate("/jobs")}
            >{t("jobs.returnToJobs")}</Button>
        </EmptyPanel>
      );
    } else {
      const tabs = {
        overview: (<JobOverviewTab job={detailState.job} />),
        activity: (
          <JobActivityTab
            job={detailState.job}
            events={detailState.events}
            onSendPrompt={detailState.sendPrompt}
            isSendingPrompt={detailState.isSendingPrompt}
          />
        ),
        files: (
          <JobFilesTab
            canBrowse={filesState.canBrowse}
            tree={filesState.tree}
            selectedPath={filesState.selectedPath}
            selectedFile={filesState.selectedFile}
            fileError={filesState.fileError}
            isLoadingTree={filesState.isLoadingTree}
            isLoadingFile={filesState.isLoadingFile}
            expandingPath={filesState.expandingPath}
            treeError={filesState.treeError}
            onToggleDirectory={filesState.toggleDirectory}
            onSelectPath={filesState.selectPath}
          />
        ),
      };

      detailContent = (
        <JobDetailShell
          job={detailState.job}
          activeTab={activeTab}
          onTabChange={setActiveTab}
          onBack={() => navigate("/jobs")}
          onCancel={handleCancel}
          onRestart={handleRestart}
          isBusy={jobsState.isBusy}
        >
          {tabs[activeTab] || tabs.overview}
        </JobDetailShell>
      );
    }
  } else {
    detailContent = jobsState.isLoading
      ? (
          <div className="space-y-4">
            {[1, 2, 3].map(
              (i) =>
                (<div
                  key={i}
                  className="v2-skeleton h-28 rounded-[18px]"
                />)
            )}
          </div>
        )
      : (
          <JobsList
            jobs={filteredJobs}
            totalJobs={jobsState.jobs.length}
            selectedJobId={jobId}
            search={search}
            onSearchChange={setSearch}
            stateFilter={stateFilter}
            onStateFilterChange={setStateFilter}
            onSelectJob={handleOpenJob}
            onCancelJob={handleCancel}
            isBusy={jobsState.isBusy}
            isRefreshing={jobsState.isRefreshing}
          />
        );
  }

  return (
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          {jobId &&
          (<div className="flex flex-wrap justify-end gap-2">
            {headerActions}
          </div>)}
          {jobsState.error &&
          (
            <div
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200"
            >
              {jobsState.error.message}
            </div>
          )}
          <FeedbackBanner
            result={jobsState.actionResult}
            onDismiss={jobsState.clearActionResult}
          />
          <FeedbackBanner
            result={detailState.promptResult}
            onDismiss={detailState.clearPromptResult}
          />
          <JobsSummaryStrip summary={jobsState.summary} />
          {detailContent}
        </div>
      </div>
    </div>
  );
}
