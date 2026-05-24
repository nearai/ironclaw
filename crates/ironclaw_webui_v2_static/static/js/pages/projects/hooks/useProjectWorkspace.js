import { useQuery, useQueryClient } from "@tanstack/react-query";
import { React } from "../../../lib/html.js";
import {
  fetchProjectDetail,
  fetchProjectMissions,
  fetchProjectThreads,
  fetchProjectWidgets,
} from "../lib/projects-api.js";

export function useProjectWorkspace(projectId) {
  const queryClient = useQueryClient();
  const enabled = Boolean(projectId);

  const projectQuery = useQuery({
    queryKey: ["project-detail", projectId],
    queryFn: () => fetchProjectDetail(projectId),
    enabled,
    refetchInterval: enabled ? 7000 : false,
  });

  const missionsQuery = useQuery({
    queryKey: ["project-missions", projectId],
    queryFn: () => fetchProjectMissions(projectId),
    enabled,
    refetchInterval: enabled ? 5000 : false,
  });

  const threadsQuery = useQuery({
    queryKey: ["project-threads", projectId],
    queryFn: () => fetchProjectThreads(projectId),
    enabled,
    refetchInterval: enabled ? 4000 : false,
  });

  const widgetsQuery = useQuery({
    queryKey: ["project-widgets", projectId],
    queryFn: () => fetchProjectWidgets(projectId),
    enabled,
    refetchInterval: enabled ? 15000 : false,
  });

  const invalidate = React.useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["projects-overview"] });
    queryClient.invalidateQueries({ queryKey: ["project-detail", projectId] });
    queryClient.invalidateQueries({ queryKey: ["project-missions", projectId] });
    queryClient.invalidateQueries({ queryKey: ["project-threads", projectId] });
    queryClient.invalidateQueries({ queryKey: ["project-widgets", projectId] });
  }, [projectId, queryClient]);

  return {
    project: projectQuery.data?.project || null,
    missions: missionsQuery.data?.missions || [],
    threads: threadsQuery.data?.threads || [],
    widgets: widgetsQuery.data || [],
    isLoading: enabled && (projectQuery.isLoading || missionsQuery.isLoading || threadsQuery.isLoading),
    isRefreshing: projectQuery.isFetching || missionsQuery.isFetching || threadsQuery.isFetching || widgetsQuery.isFetching,
    error: projectQuery.error || missionsQuery.error || threadsQuery.error || widgetsQuery.error || null,
    invalidate,
  };
}
