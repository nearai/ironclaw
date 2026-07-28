// @ts-nocheck
import { useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import React from "react";
import {
  fetchProjectsOverview,
  fetchMissions,
  fireMission as fireMissionRequest,
  pauseMission as pauseMissionRequest,
  resumeMission as resumeMissionRequest,
} from "../lib/missions-api";
import { summarizeMissions } from "../lib/missions-presenters";

function decorateMission(mission, project) {
  return {
    ...mission,
    project: {
      id: project.id,
      name: project.name,
      health: project.health,
    },
  };
}

export function useMissions() {
  const queryClient = useQueryClient();
  const [actionResult, setActionResult] = React.useState(null);

  const projectsQuery = useQuery({
    queryKey: ["projects-overview"],
    queryFn: fetchProjectsOverview,
    refetchInterval: 7000,
  });

  const projects = projectsQuery.data?.projects || [];
  const missionQueries = useQueries({
    queries: projects.map((project) => ({
      queryKey: ["missions", "project", project.id],
      queryFn: () => fetchMissions({ projectId: project.id }),
      refetchInterval: 5000,
      select: (data) => data?.missions || [],
    })),
  });

  const missions = missionQueries.flatMap((query, index) => {
    const project = projects[index];
    return (query.data || []).map((mission) => decorateMission(mission, project));
  });

  const invalidate = React.useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["projects-overview"] });
    queryClient.invalidateQueries({ queryKey: ["missions"] });
    queryClient.invalidateQueries({ queryKey: ["mission-detail"] });
  }, [queryClient]);

  const mutationOptions = (request, successMessage) => ({
    mutationFn: ({ missionId }) => request(missionId),
    onSuccess: () => {
      setActionResult({ type: "success", message: successMessage });
      invalidate();
    },
    onError: (error) => {
      setActionResult({
        type: "error",
        message: error.message || "Unable to update mission",
      });
    },
  });

  const fireMutation = useMutation(mutationOptions(fireMissionRequest, "Mission fired and a run was queued."));
  const pauseMutation = useMutation(mutationOptions(pauseMissionRequest, "Mission paused."));
  const resumeMutation = useMutation(mutationOptions(resumeMissionRequest, "Mission resumed."));

  return {
    projects,
    missions,
    summary: summarizeMissions(missions),
    isLoading: projectsQuery.isLoading || missionQueries.some((query) => query.isLoading),
    isRefreshing: projectsQuery.isFetching || missionQueries.some((query) => query.isFetching),
    error: projectsQuery.error || missionQueries.find((query) => query.error)?.error || null,
    actionResult,
    clearActionResult: () => setActionResult(null),
    fireMission: fireMutation.mutateAsync,
    pauseMission: pauseMutation.mutateAsync,
    resumeMission: resumeMutation.mutateAsync,
    isBusy: fireMutation.isPending || pauseMutation.isPending || resumeMutation.isPending,
    invalidate,
  };
}
