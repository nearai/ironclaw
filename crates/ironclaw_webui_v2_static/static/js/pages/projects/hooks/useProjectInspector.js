import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { React } from "../../../lib/html.js";
import {
  fetchMissionDetail,
  fetchThreadDetail,
  fireMission as fireMissionRequest,
  pauseMission as pauseMissionRequest,
  resumeMission as resumeMissionRequest,
} from "../lib/projects-api.js";

export function useProjectInspector({ projectId, missionId, threadId }) {
  const queryClient = useQueryClient();
  const [actionResult, setActionResult] = React.useState(null);

  const missionQuery = useQuery({
    queryKey: ["project-mission-detail", missionId],
    queryFn: () => fetchMissionDetail(missionId),
    enabled: Boolean(missionId),
    refetchInterval: missionId ? 5000 : false,
  });

  const threadQuery = useQuery({
    queryKey: ["project-thread-detail", threadId],
    queryFn: () => fetchThreadDetail(threadId),
    enabled: Boolean(threadId),
    refetchInterval: threadId ? 4000 : false,
  });

  const invalidateProject = React.useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["projects-overview"] });
    queryClient.invalidateQueries({ queryKey: ["project-detail", projectId] });
    queryClient.invalidateQueries({ queryKey: ["project-missions", projectId] });
    queryClient.invalidateQueries({ queryKey: ["project-threads", projectId] });
    if (missionId) {
      queryClient.invalidateQueries({ queryKey: ["project-mission-detail", missionId] });
    }
    if (threadId) {
      queryClient.invalidateQueries({ queryKey: ["project-thread-detail", threadId] });
    }
  }, [missionId, projectId, queryClient, threadId]);

  const fireMutation = useMutation({
    mutationFn: ({ targetMissionId }) => fireMissionRequest(targetMissionId),
    onSuccess: (data) => {
      setActionResult({
        type: "success",
        message: data?.thread_id ? "Mission fired and a new run is live." : "Mission fire request accepted.",
      });
      invalidateProject();
    },
    onError: (error) => {
      setActionResult({
        type: "error",
        message: error.message || "Unable to fire mission",
      });
    },
  });

  const pauseMutation = useMutation({
    mutationFn: ({ targetMissionId }) => pauseMissionRequest(targetMissionId),
    onSuccess: () => {
      setActionResult({
        type: "success",
        message: "Mission paused.",
      });
      invalidateProject();
    },
    onError: (error) => {
      setActionResult({
        type: "error",
        message: error.message || "Unable to pause mission",
      });
    },
  });

  const resumeMutation = useMutation({
    mutationFn: ({ targetMissionId }) => resumeMissionRequest(targetMissionId),
    onSuccess: () => {
      setActionResult({
        type: "success",
        message: "Mission resumed.",
      });
      invalidateProject();
    },
    onError: (error) => {
      setActionResult({
        type: "error",
        message: error.message || "Unable to resume mission",
      });
    },
  });

  return {
    mission: missionQuery.data?.mission || null,
    thread: threadQuery.data?.thread || null,
    inspectorType: threadId ? "thread" : missionId ? "mission" : null,
    isLoading: missionQuery.isLoading || threadQuery.isLoading,
    isRefreshing: missionQuery.isFetching || threadQuery.isFetching,
    error: missionQuery.error || threadQuery.error || null,
    actionResult,
    clearActionResult: () => setActionResult(null),
    fireMission: fireMutation.mutateAsync,
    pauseMission: pauseMutation.mutateAsync,
    resumeMission: resumeMutation.mutateAsync,
    isBusy: fireMutation.isPending || pauseMutation.isPending || resumeMutation.isPending,
  };
}
