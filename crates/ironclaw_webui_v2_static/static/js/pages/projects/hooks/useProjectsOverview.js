import { useQuery, useQueryClient } from "@tanstack/react-query";
import { React } from "../../../lib/html.js";
import { fetchProjectsOverview } from "../lib/projects-api.js";

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
