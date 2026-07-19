import { useQuery } from "@tanstack/react-query";
import { fetchMissionDetail } from "../lib/missions-api";

export function useMissionDetail(missionId) {
  const query = useQuery({
    queryKey: ["mission-detail", missionId],
    queryFn: () => fetchMissionDetail(missionId),
    enabled: Boolean(missionId),
    refetchInterval: missionId ? 5000 : false,
  });

  return {
    mission: query.data?.mission || null,
    isLoading: query.isLoading,
    isRefreshing: query.isFetching,
    error: query.error || null,
  };
}
