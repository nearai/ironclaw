// @ts-nocheck
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import React from "react";
import { useT } from "../../../lib/i18n";
import {
  fetchMissionDetail,
  fetchThreadDetail,
  fireMission as fireMissionRequest,
  pauseMission as pauseMissionRequest,
  resumeMission as resumeMissionRequest,
} from "../lib/projects-api";

export function useProjectInspector({ projectId, missionId, threadId }) {
  const t = useT();
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
        message: data?.thread_id
          ? t("projects.action.missionFiredWithRun")
          : t("projects.action.missionFireAccepted"),
      });
      invalidateProject();
    },
    onError: (error) => {
      setActionResult({
        type: "error",
        message: error.message || t("projects.action.fireFailed"),
      });
    },
  });

  const pauseMutation = useMutation({
    mutationFn: ({ targetMissionId }) => pauseMissionRequest(targetMissionId),
    onSuccess: () => {
      setActionResult({
        type: "success",
        message: t("projects.action.missionPaused"),
      });
      invalidateProject();
    },
    onError: (error) => {
      setActionResult({
        type: "error",
        message: error.message || t("projects.action.pauseFailed"),
      });
    },
  });

  const resumeMutation = useMutation({
    mutationFn: ({ targetMissionId }) => resumeMissionRequest(targetMissionId),
    onSuccess: () => {
      setActionResult({
        type: "success",
        message: t("projects.action.missionResumed"),
      });
      invalidateProject();
    },
    onError: (error) => {
      setActionResult({
        type: "error",
        message: error.message || t("projects.action.resumeFailed"),
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
