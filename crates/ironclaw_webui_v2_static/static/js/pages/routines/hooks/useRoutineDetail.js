import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { React } from "../../../lib/html.js";
import {
  fetchRoutineDetail,
  toggleRoutine as toggleRoutineRequest,
  triggerRoutine as triggerRoutineRequest,
} from "../lib/routines-api.js";

export function useRoutineDetail(routineId) {
  const queryClient = useQueryClient();
  const [actionResult, setActionResult] = React.useState(null);

  const detailQuery = useQuery({
    queryKey: ["routine-detail", routineId],
    queryFn: () => fetchRoutineDetail(routineId),
    enabled: Boolean(routineId),
    refetchInterval: routineId ? 5000 : false,
  });

  const invalidate = React.useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["routine-detail", routineId] });
    queryClient.invalidateQueries({ queryKey: ["routines"] });
    queryClient.invalidateQueries({ queryKey: ["routines-summary"] });
  }, [queryClient, routineId]);

  const mutationOptions = (request, message) => ({
    mutationFn: () => request(routineId),
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

  return {
    routine: detailQuery.data || null,
    isLoading: detailQuery.isLoading,
    error: detailQuery.error || null,
    actionResult,
    clearActionResult: () => setActionResult(null),
    triggerRoutine: triggerMutation.mutateAsync,
    toggleRoutine: toggleMutation.mutateAsync,
    isBusy: triggerMutation.isPending || toggleMutation.isPending,
  };
}
