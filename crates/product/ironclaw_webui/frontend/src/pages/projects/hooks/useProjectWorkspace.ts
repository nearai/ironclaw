import { useQuery, useQueryClient } from "@tanstack/react-query";
import React from "react";
import {
  fetchProjectDetail,
  fetchProjectThreads,
  fetchProjectWidgets,
} from "../lib/projects-api";

export function useProjectWorkspace(projectId) {
  const queryClient = useQueryClient();
  const enabled = Boolean(projectId);

  const projectQuery = useQuery({
    queryKey: ["project-detail", projectId],
    queryFn: () => fetchProjectDetail(projectId),
    enabled,
    refetchInterval: enabled ? 7000 : false,
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
    queryClient.invalidateQueries({ queryKey: ["project-threads", projectId] });
    queryClient.invalidateQueries({ queryKey: ["project-widgets", projectId] });
  }, [projectId, queryClient]);

  return {
    // `fetchProjectDetail` returns the page-shaped project object directly
    // (not wrapped in `{ project }`), matching `fetchProjectsOverview` entries.
    project: projectQuery.data || null,
    threads: threadsQuery.data?.threads || [],
    widgets: widgetsQuery.data || [],
    isLoading: enabled && (projectQuery.isLoading || threadsQuery.isLoading),
    isRefreshing: projectQuery.isFetching || threadsQuery.isFetching || widgetsQuery.isFetching,
    error: projectQuery.error || threadsQuery.error || widgetsQuery.error || null,
    invalidate,
  };
}
