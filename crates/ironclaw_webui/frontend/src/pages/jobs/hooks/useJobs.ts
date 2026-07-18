// @ts-nocheck
import React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  fetchJobs,
  fetchJobsSummary,
  cancelJob as cancelJobRequest,
  restartJob as restartJobRequest,
} from "../lib/jobs-api";
import { truncateJobId } from "../lib/jobs-presenters";

export function useJobs() {
  const queryClient = useQueryClient();
  const [actionResult, setActionResult] = React.useState(null);

  const summaryQuery = useQuery({
    queryKey: ["jobs-summary"],
    queryFn: fetchJobsSummary,
    refetchInterval: 5000,
  });

  const jobsQuery = useQuery({
    queryKey: ["jobs"],
    queryFn: fetchJobs,
    refetchInterval: 5000,
  });

  const invalidate = React.useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["jobs"] });
    queryClient.invalidateQueries({ queryKey: ["jobs-summary"] });
  }, [queryClient]);

  const cancelMutation = useMutation({
    mutationFn: ({ jobId }) => cancelJobRequest(jobId),
    onSuccess: (_data, { jobId }) => {
      setActionResult({
        type: "success",
        message: `Job ${truncateJobId(jobId)} cancelled`,
      });
      invalidate();
    },
    onError: (error) => {
      setActionResult({
        type: "error",
        message: error.message || "Unable to cancel job",
      });
    },
  });

  const restartMutation = useMutation({
    mutationFn: ({ jobId }) => restartJobRequest(jobId),
    onSuccess: (data) => {
      setActionResult({
        type: "success",
        message: `Restart queued as ${truncateJobId(data?.new_job_id)}`,
      });
      invalidate();
    },
    onError: (error) => {
      setActionResult({
        type: "error",
        message: error.message || "Unable to restart job",
      });
    },
  });

  return {
    summary: summaryQuery.data || {
      total: 0,
      pending: 0,
      in_progress: 0,
      completed: 0,
      failed: 0,
      stuck: 0,
    },
    jobs: jobsQuery.data?.jobs || [],
    isLoading: summaryQuery.isLoading || jobsQuery.isLoading,
    isRefreshing: summaryQuery.isFetching || jobsQuery.isFetching,
    error: summaryQuery.error || jobsQuery.error || null,
    actionResult,
    clearActionResult: () => setActionResult(null),
    cancelJob: cancelMutation.mutateAsync,
    restartJob: restartMutation.mutateAsync,
    isBusy: cancelMutation.isPending || restartMutation.isPending,
    invalidate,
  };
}
