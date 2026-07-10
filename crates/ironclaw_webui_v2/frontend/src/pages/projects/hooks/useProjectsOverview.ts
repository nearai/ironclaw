import { useQuery, useQueryClient } from "@tanstack/react-query";
import React from "react";
import { fetchProjectsOverview } from "../lib/projects-api";

export function useProjectsOverview() {
  const queryClient = useQueryClient();

  const query = useQuery({
    queryKey: ["projects-overview"],
    queryFn: fetchProjectsOverview,
    refetchInterval: 5000,
  });

  const invalidate = React.useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["projects-overview"] });
  }, [queryClient]);

  return {
    overview: query.data || { attention: [], projects: [] },
    isLoading: query.isLoading,
    isRefreshing: query.isFetching,
    error: query.error || null,
    invalidate,
  };
}
