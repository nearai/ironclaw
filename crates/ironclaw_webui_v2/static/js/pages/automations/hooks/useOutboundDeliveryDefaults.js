import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { React } from "../../../lib/html.js";
import {
  getOutboundPreferences,
  listOutboundDeliveryTargets,
  setOutboundPreferences,
} from "../../../lib/api.js";

const PREFERENCES_QUERY_KEY = ["outbound-delivery", "preferences"];
const TARGETS_QUERY_KEY = ["outbound-delivery", "targets"];

export function useOutboundDeliveryDefaults() {
  const queryClient = useQueryClient();
  const preferencesQuery = useQuery({
    queryKey: PREFERENCES_QUERY_KEY,
    queryFn: getOutboundPreferences,
  });
  const targetsQuery = useQuery({
    queryKey: TARGETS_QUERY_KEY,
    queryFn: listOutboundDeliveryTargets,
  });
  const saveMutation = useMutation({
    mutationFn: ({ finalReplyTargetId }) =>
      setOutboundPreferences({ finalReplyTargetId }),
    onSuccess: (preferences) => {
      queryClient.setQueryData(PREFERENCES_QUERY_KEY, preferences);
      queryClient.invalidateQueries({ queryKey: TARGETS_QUERY_KEY });
    },
  });

  const targets = React.useMemo(
    () => targetsQuery.data?.targets ?? [],
    [targetsQuery.data],
  );
  const finalReplyTargets = React.useMemo(
    () => targets.filter((option) => option?.capabilities?.final_replies),
    [targets],
  );

  return {
    preferences: preferencesQuery.data ?? null,
    targets,
    finalReplyTargets,
    currentTarget: preferencesQuery.data?.final_reply_target ?? null,
    currentStatus:
      preferencesQuery.data?.final_reply_target_status ?? "none_configured",
    isLoading: preferencesQuery.isLoading || targetsQuery.isLoading,
    isRefreshing: preferencesQuery.isFetching || targetsQuery.isFetching,
    isSaving: saveMutation.isPending,
    error: preferencesQuery.error || targetsQuery.error || null,
    saveError: saveMutation.error || null,
    saveFinalReplyTarget: (finalReplyTargetId) =>
      saveMutation.mutateAsync({ finalReplyTargetId }),
    refetch: () => {
      preferencesQuery.refetch();
      targetsQuery.refetch();
    },
  };
}
