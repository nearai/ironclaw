import React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { fetchJobDetail, fetchJobEvents, sendJobPrompt } from "../lib/jobs-api";

type JobPromptVariables = { content: string; done: boolean };
type JobPromptResult = { type: "success" | "error"; message: string };

export function useJobDetail(jobId: string | null) {
  const queryClient = useQueryClient();
  const [promptResult, setPromptResult] = React.useState<JobPromptResult | null>(null);

  const detailQuery = useQuery({
    queryKey: ["job-detail", jobId],
    queryFn: () => fetchJobDetail(jobId),
    enabled: Boolean(jobId),
    refetchInterval: jobId ? 4000 : false,
  });

  const eventsQuery = useQuery({
    queryKey: ["job-events", jobId],
    queryFn: () => fetchJobEvents(jobId),
    enabled: Boolean(jobId),
    refetchInterval: jobId ? 2500 : false,
  });

  const promptMutation = useMutation<unknown, Error, JobPromptVariables>({
    mutationFn: ({ content, done }) => sendJobPrompt(jobId, { content, done }),
    onSuccess: (_data, { done }) => {
      setPromptResult({
        type: "success",
        message: done ? "Done signal sent to the job" : "Follow-up sent to the job",
      });
      queryClient.invalidateQueries({ queryKey: ["job-detail", jobId] });
      queryClient.invalidateQueries({ queryKey: ["job-events", jobId] });
      queryClient.invalidateQueries({ queryKey: ["jobs"] });
      queryClient.invalidateQueries({ queryKey: ["jobs-summary"] });
    },
    onError: (error) => {
      setPromptResult({
        type: "error",
        message: error.message || "Unable to send follow-up",
      });
    },
  });

  React.useEffect(() => {
    setPromptResult(null);
  }, [jobId]);

  return {
    job: detailQuery.data || null,
    events: eventsQuery.data?.events || [],
    isLoading: detailQuery.isLoading,
    isRefreshing: detailQuery.isFetching || eventsQuery.isFetching,
    error: detailQuery.error || eventsQuery.error || null,
    sendPrompt: promptMutation.mutateAsync,
    isSendingPrompt: promptMutation.isPending,
    promptResult,
    clearPromptResult: () => setPromptResult(null),
  };
}
