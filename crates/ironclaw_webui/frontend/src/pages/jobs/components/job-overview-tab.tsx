import {
  DetailList,
  DetailRow,
  EmptyPanel,
  FlowList,
  Card,
  SectionHeader,
  Badge,
} from "@ironclaw/ui";
import { MarkdownRenderer } from "../../chat/components/markdown-renderer";
import {
  formatDuration,
  formatJobDate,
  stateLabel,
  statusToneForState,
} from "../lib/jobs-presenters";

export function JobOverviewTab({ job }) {
  const transitions = (job.transitions || []).map((transition) => ({
    title: `${stateLabel(transition.from)} -> ${stateLabel(transition.to)}`,
    description: [formatJobDate(transition.timestamp), transition.reason].filter(Boolean).join(" / "),
  }));

  return (
    <div className="grid gap-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(320px,0.8fr)]">
      <Card className="p-5 sm:p-6">
        <SectionHeader
          eyebrow="Execution context"
          title="Timing, state, and runtime shape"
          titleAs="h3"
          actions={<Badge tone={statusToneForState(job.state)} label={stateLabel(job.state)} />}
        />

        <DetailList className="mt-3 grid gap-x-6 md:grid-cols-2">
          <DetailRow layout="stacked" term="Created">{formatJobDate(job.created_at) || "Not available"}</DetailRow>
          <DetailRow layout="stacked" term="Started">{formatJobDate(job.started_at) || "Not available"}</DetailRow>
          <DetailRow layout="stacked" term="Completed">{formatJobDate(job.completed_at) || "Not available"}</DetailRow>
          <DetailRow layout="stacked" term="Duration">{formatDuration(job.elapsed_secs) || "Not available"}</DetailRow>
          <DetailRow layout="stacked" term="Kind">{job.job_kind ? `${job.job_kind} job` : "Not available"}</DetailRow>
          <DetailRow layout="stacked" term="Mode">{job.job_mode || "Default worker"}</DetailRow>
        </DetailList>
      </Card>

      <div className="space-y-5">
        <Card className="p-5 sm:p-6">
          <SectionHeader eyebrow="Description" title="Mission brief" titleAs="h3" />
          {job.description
            ? (<MarkdownRenderer content={job.description} className="mt-4 text-sm leading-7 text-iron-200" />)
            : (<p className="mt-4 text-sm leading-6 text-iron-300">This job did not record a long-form description.</p>)}
        </Card>

        {transitions.length
          ? (
              <Card className="p-5 sm:p-6">
                <SectionHeader eyebrow="Transitions" title="State timeline" titleAs="h3" />
                <div className="mt-3">
                  <FlowList items={transitions} />
                </div>
              </Card>
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
