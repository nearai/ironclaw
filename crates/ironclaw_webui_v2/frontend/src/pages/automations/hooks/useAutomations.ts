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

type ActionMutationContext = {
  sequence: number;
};

function isLatestAutomationAction(
  context: ActionMutationContext | undefined,
  latestSequence: number
) {
  return context?.sequence === latestSequence;
}

export function useAutomations(includeCompleted = false) {
  const { t, lang } = useI18n();
  const queryClient = useQueryClient();
  const [hasActionError, setHasActionError] = React.useState(false);
  const latestActionSequence = React.useRef(0);
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
  const clearActionError = React.useCallback(() => {
    setHasActionError(false);
  }, []);
  const beginAction = React.useCallback((): ActionMutationContext => {
    const sequence = latestActionSequence.current + 1;
    latestActionSequence.current = sequence;
    clearActionError();
    return { sequence };
  }, [clearActionError]);
  const showActionError = React.useCallback((
    _error: unknown,
    _variables: unknown,
    context: ActionMutationContext | undefined
  ) => {
    // A newer action deliberately supersedes older results: starting any
    // action clears the banner, and a late completion must not resurrect it.
    if (isLatestAutomationAction(context, latestActionSequence.current)) {
      setHasActionError(true);
    }
  }, []);
  const handleActionSuccess = React.useCallback((
    _data: unknown,
    _variables: unknown,
    context: ActionMutationContext | undefined
  ) => {
    if (isLatestAutomationAction(context, latestActionSequence.current)) {
      clearActionError();
    }
    invalidateAutomations();
  }, [clearActionError, invalidateAutomations]);
  const pauseMutation = useMutation({
    mutationFn: (automationId: string) => pauseAutomation({ automationId }),
    onMutate: beginAction,
    onError: showActionError,
    onSuccess: handleActionSuccess,
  });
  const resumeMutation = useMutation({
    mutationFn: (automationId: string) => resumeAutomation({ automationId }),
    onMutate: beginAction,
    onError: showActionError,
    onSuccess: handleActionSuccess,
  });
  const renameMutation = useMutation({
    mutationFn: ({ automationId, name }: RenameAutomationVariables) =>
      renameAutomation({ automationId, name }),
    onMutate: beginAction,
    onError: showActionError,
    onSuccess: handleActionSuccess,
  });
  const deleteMutation = useMutation({
    mutationFn: (automationId: string) => deleteAutomation({ automationId }),
    onMutate: beginAction,
    onError: showActionError,
    onSuccess: handleActionSuccess,
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
    actionError: hasActionError,
    dismissActionError: clearActionError,
    pauseAutomation: pauseMutation.mutate,
    resumeAutomation: resumeMutation.mutate,
    renameAutomation: renameMutation.mutate,
    deleteAutomation: deleteMutation.mutate,
    refetch: query.refetch,
  };
}
