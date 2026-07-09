import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import React from "react";
import {
  deleteAutomation,
  listAutomations,
  pauseAutomation,
  renameAutomation,
  resumeAutomation,
} from "../../../lib/api";
import { useI18n } from "../../../lib/i18n";

import {
  automationSummary,
  normalizeAutomations,
} from "../lib/automations-presenters";
import {
  AUTOMATIONS_BASE_REFETCH_MS,
  nextAutomationsRefetchDelay,
} from "../lib/automations-refresh";

const AUTOMATIONS_PAGE_LIMIT = 50;
const AUTOMATION_RUNS_LIMIT = 25;

type RenameAutomationVariables = {
  automationId: string;
  name: string;
};

export function useAutomations(includeCompleted = false) {
  const { t, lang } = useI18n();
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["automations", { includeCompleted }],
    queryFn: () =>
      listAutomations({
        limit: AUTOMATIONS_PAGE_LIMIT,
        runLimit: AUTOMATION_RUNS_LIMIT,
        includeCompleted,
      }),
    refetchInterval: AUTOMATIONS_BASE_REFETCH_MS,
    refetchIntervalInBackground: false,
  });

  // Schedule labels are localized in the presenter (`scheduleLabel`), so the
  // memo must re-run when the active language changes, not just the data.
  const automations = React.useMemo(
    () => normalizeAutomations(query.data, t, lang),
    [query.data, t, lang]
  );
  const summary = React.useMemo(
    () => automationSummary(automations),
    [automations]
  );
  const nextRefreshDelay = React.useMemo(
    () => nextAutomationsRefetchDelay(automations),
    [automations]
  );

  React.useEffect(() => {
    if (nextRefreshDelay == null) return undefined;
    // The query's base refetchInterval keeps long-horizon schedules polling;
    // this timer only pulls near-due and running automations forward.
    const timer = setTimeout(() => {
      query.refetch();
    }, nextRefreshDelay);
    return () => clearTimeout(timer);
  }, [nextRefreshDelay, query.refetch]);

  // The scheduler (trigger poller) may be turned off, in which case listed
  // automations never fire. Treat an absent flag as enabled so we don't show a
  // false "off" notice against an older payload.
  const schedulerEnabled = query.data?.scheduler_enabled !== false;
  const invalidateAutomations = React.useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["automations"] });
  }, [queryClient]);
  const pauseMutation = useMutation({
    mutationFn: (automationId: string) => pauseAutomation({ automationId }),
    onSuccess: invalidateAutomations,
  });
  const resumeMutation = useMutation({
    mutationFn: (automationId: string) => resumeAutomation({ automationId }),
    onSuccess: invalidateAutomations,
  });
  const renameMutation = useMutation({
    mutationFn: ({ automationId, name }: RenameAutomationVariables) =>
      renameAutomation({ automationId, name }),
    onSuccess: invalidateAutomations,
  });
  const deleteMutation = useMutation({
    mutationFn: (automationId: string) => deleteAutomation({ automationId }),
    onSuccess: invalidateAutomations,
  });

  return {
    automations,
    summary,
    schedulerEnabled,
    isLoading: query.isLoading,
    isRefreshing: query.isFetching,
    isMutating:
      pauseMutation.isPending ||
      resumeMutation.isPending ||
      renameMutation.isPending ||
      deleteMutation.isPending,
    error: query.error || null,
    actionError:
      pauseMutation.error ||
      resumeMutation.error ||
      renameMutation.error ||
      deleteMutation.error ||
      null,
    pauseAutomation: pauseMutation.mutate,
    resumeAutomation: resumeMutation.mutate,
    renameAutomation: renameMutation.mutate,
    deleteAutomation: deleteMutation.mutate,
    refetch: query.refetch,
  };
}
