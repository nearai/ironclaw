// @ts-nocheck
import { useNavigate, useParams } from "react-router";
import { Button } from "../../design-system/button";
import { InlineNotice } from "../../design-system/inline-notice";
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

function resultTone(result) {
  if (result?.type === "success") return "success";
  if (result?.type === "error") return "danger";
  return "info";
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
            <InlineNotice tone="danger" role="alert">
              {jobsState.error.message}
            </InlineNotice>
          )}
          {jobsState.actionResult && (
            <InlineNotice
              tone={resultTone(jobsState.actionResult)}
              role={jobsState.actionResult.type === "error" ? "alert" : "status"}
              onDismiss={jobsState.clearActionResult}
              dismissLabel={t("jobs.dismiss")}
            >
              {jobsState.actionResult.message}
            </InlineNotice>
          )}
          {detailState.promptResult && (
            <InlineNotice
              tone={resultTone(detailState.promptResult)}
              role={detailState.promptResult.type === "error" ? "alert" : "status"}
              onDismiss={detailState.clearPromptResult}
              dismissLabel={t("jobs.dismiss")}
            >
              {detailState.promptResult.message}
            </InlineNotice>
          )}
          <JobsSummaryStrip summary={jobsState.summary} />
          {detailContent}
        </div>
      </div>
    </div>
  );
}
