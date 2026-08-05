import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import React from "react";
import {
  deleteRoutine as deleteRoutineRequest,
  fetchRoutines,
  fetchRoutinesSummary,
  toggleRoutine as toggleRoutineRequest,
  triggerRoutine as triggerRoutineRequest,
} from "../lib/routines-api";

export function useRoutines() {
  const queryClient = useQueryClient();
  const [actionResult, setActionResult] = React.useState(null);

  const summaryQuery = useQuery({
    queryKey: ["routines-summary"],
    queryFn: fetchRoutinesSummary,
    refetchInterval: 5000,
  });

  const routinesQuery = useQuery({
    queryKey: ["routines"],
    queryFn: fetchRoutines,
    refetchInterval: 5000,
  });

  const invalidate = React.useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["routines"] });
    queryClient.invalidateQueries({ queryKey: ["routines-summary"] });
    queryClient.invalidateQueries({ queryKey: ["routine-detail"] });
  }, [queryClient]);

  const mutationOptions = (request, message) => ({
    mutationFn: ({ routineId }) => request(routineId),
    onSuccess: () => {
      setActionResult({ type: "success", message });
      invalidate();
    },
    onError: (error) => {
      setActionResult({
        type: "error",
        message: error.message || "Unable to update routine",
      });
    },
  });

  const triggerMutation = useMutation(
    mutationOptions(triggerRoutineRequest, "Routine run queued.")
  );
  const toggleMutation = useMutation(
    mutationOptions(toggleRoutineRequest, "Routine status updated.")
  );
  const deleteMutation = useMutation(
    mutationOptions(deleteRoutineRequest, "Routine deleted.")
  );

  return {
    summary: summaryQuery.data || {
      total: 0,
      enabled: 0,
      disabled: 0,
      unverified: 0,
      failing: 0,
      runs_today: 0,
    },
    routines: routinesQuery.data?.routines || [],
    isLoading: summaryQuery.isLoading || routinesQuery.isLoading,
    isRefreshing: summaryQuery.isFetching || routinesQuery.isFetching,
    error: summaryQuery.error || routinesQuery.error || null,
    actionResult,
    clearActionResult: () => setActionResult(null),
    triggerRoutine: triggerMutation.mutateAsync,
    toggleRoutine: toggleMutation.mutateAsync,
    deleteRoutine: deleteMutation.mutateAsync,
    isBusy:
      triggerMutation.isPending ||
      toggleMutation.isPending ||
      deleteMutation.isPending,
    invalidate,
  };
}
