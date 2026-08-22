import {
  keepPreviousData,
  type MutationFunction,
  type UseMutationOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import React from "react";
import {
  deleteAutomation,
  listAutomations,
  pauseAutomation,
  renameAutomation,
  resumeAutomation,
  runAutomation,
} from "../../../lib/api";
import { useI18n } from "../../../lib/i18n";
import { dismissToast, toast } from "../../../lib/toast";

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

export type ActionMutationContext = {
  sequence: number;
};

type MutableRef<T> = {
  current: T;
};

export type AutomationMutationLifecycle = {
  onMutate: (_variables: unknown) => ActionMutationContext;
  onError: (
    _error: unknown,
    _variables: unknown,
    context: ActionMutationContext | undefined
  ) => void;
  onSuccess: (
    _data: unknown,
    _variables: unknown,
    context: ActionMutationContext | undefined
  ) => void;
};

type AutomationMutationLifecycleOptions = {
  latestActionSequence: MutableRef<number>;
  actionErrorToastId: MutableRef<string | null>;
  dismissErrorToast: (id: string | null | undefined) => void;
  showErrorToast: () => string;
  invalidateAutomations: () => void;
};

export function createAutomationMutationLifecycle({
  latestActionSequence,
  actionErrorToastId,
  dismissErrorToast,
  showErrorToast,
  invalidateAutomations,
}: AutomationMutationLifecycleOptions): AutomationMutationLifecycle {
  const clearActionError = () => {
    if (actionErrorToastId.current !== null) {
      dismissErrorToast(actionErrorToastId.current);
    }
    actionErrorToastId.current = null;
  };

  return {
    onMutate: () => {
      const sequence = latestActionSequence.current + 1;
      latestActionSequence.current = sequence;
      clearActionError();
      return { sequence };
    },
    onError: (_error, _variables, context) => {
      // A newer action deliberately supersedes older results: a late failure
      // must neither dismiss nor replace the latest action's toast.
      if (context?.sequence !== latestActionSequence.current) return;
      clearActionError();
      actionErrorToastId.current = showErrorToast();
    },
    onSuccess: (_data, _variables, context) => {
      if (context?.sequence === latestActionSequence.current) {
        clearActionError();
      }
      invalidateAutomations();
    },
  };
}

export function createAutomationMutationConfig<TData, TVariables>(
  mutationFn: MutationFunction<TData, TVariables>,
  lifecycle: AutomationMutationLifecycle
): UseMutationOptions<TData, unknown, TVariables, ActionMutationContext> {
  return {
    mutationFn,
    onMutate: lifecycle.onMutate,
    onError: lifecycle.onError,
    onSuccess: lifecycle.onSuccess,
  };
}

/**
 * Keeps the previous query payload visible while the completed-data query key
 * changes. In product terms, `isPlaceholderData` therefore means a filter
 * transition is in progress, not that the Automations page is initially
 * loading.
 */
export function createAutomationsQueryOptions(includeCompleted = false) {
  return {
    queryKey: ["automations", { includeCompleted }] as const,
    queryFn: () =>
      listAutomations({
        limit: AUTOMATIONS_PAGE_LIMIT,
        runLimit: AUTOMATION_RUNS_LIMIT,
        includeCompleted,
      }),
    placeholderData: keepPreviousData,
    refetchInterval: AUTOMATIONS_BASE_REFETCH_MS,
    refetchIntervalInBackground: false,
  };
}

export function useAutomations(includeCompleted = false) {
  const { t, lang } = useI18n();
  const queryClient = useQueryClient();
  const latestActionSequence = React.useRef(0);
  const actionErrorToastId = React.useRef<string | null>(null);
  const query = useQuery(createAutomationsQueryOptions(includeCompleted));
  // Completed one-shots stay out of the default list, but failed completed
  // runs must still contribute to the summary and remain one click away. Load
  // the full projection in parallel without making it the page's initial
  // loading gate. When a historical filter is selected, the primary observer
  // adopts this same cached/in-flight query and keepPreviousData retains the
  // current rows until it settles.
  const completedSummaryQuery = useQuery({
    ...createAutomationsQueryOptions(true),
    enabled: !includeCompleted,
  });

  // Schedule labels are localized in the presenter (`scheduleLabel`), so the
  // memo must re-run when the active language changes, not just the data.
  const automations = React.useMemo(
    () => normalizeAutomations(query.data, t, lang),
    [query.data, t, lang]
  );
  const summaryAutomations = React.useMemo(
    () =>
      normalizeAutomations(
        includeCompleted
          ? query.data
          : completedSummaryQuery.data ?? query.data,
        t,
        lang
      ),
    [completedSummaryQuery.data, includeCompleted, lang, query.data, t]
  );
  const summary = React.useMemo(
    () => automationSummary(summaryAutomations),
    [summaryAutomations]
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
  const refetchAutomations = React.useCallback(() => {
    const refetches = [query.refetch()];
    if (!includeCompleted) {
      refetches.push(completedSummaryQuery.refetch());
    }
    return Promise.all(refetches);
  }, [completedSummaryQuery.refetch, includeCompleted, query.refetch]);
  const showActionErrorToast = React.useCallback(
    () =>
      toast(t("automations.error.actionFailed"), {
        tone: "error",
      }),
    [t]
  );
  const mutationLifecycle = React.useMemo(
    () =>
      createAutomationMutationLifecycle({
        latestActionSequence,
        actionErrorToastId,
        dismissErrorToast: dismissToast,
        showErrorToast: showActionErrorToast,
        invalidateAutomations,
      }),
    [invalidateAutomations, showActionErrorToast]
  );
  const pauseMutation = useMutation(
    createAutomationMutationConfig(
      (automationId: string) => pauseAutomation({ automationId }),
      mutationLifecycle
    )
  );
  const runMutation = useMutation(
    createAutomationMutationConfig(
      (automationId: string) => runAutomation({ automationId }),
      mutationLifecycle
    )
  );
  const resumeMutation = useMutation(
    createAutomationMutationConfig(
      (automationId: string) => resumeAutomation({ automationId }),
      mutationLifecycle
    )
  );
  const renameMutation = useMutation(
    createAutomationMutationConfig(
      ({ automationId, name }: RenameAutomationVariables) =>
        renameAutomation({ automationId, name }),
      mutationLifecycle
    )
  );
  const deleteMutation = useMutation(
    createAutomationMutationConfig(
      (automationId: string) => deleteAutomation({ automationId }),
      mutationLifecycle
    )
  );

  return {
    automations,
    summary,
    schedulerEnabled,
    isLoading: query.isLoading,
    // A manual refresh updates both the visible list and the background
    // completed-inclusive summary. Keep the UI in its refreshing state until
    // neither request can still change what the page presents.
    isRefreshing: query.isFetching || completedSummaryQuery.isFetching,
    isFilterTransition: query.isPlaceholderData,
    isMutating:
      runMutation.isPending ||
      pauseMutation.isPending ||
      resumeMutation.isPending ||
      renameMutation.isPending ||
      deleteMutation.isPending,
    // The completed-inclusive request is deliberately non-blocking. Its
    // failure may leave summary counts stale, but must not turn a successful
    // primary list load into a page-level failure.
    error: query.error || null,
    summaryError:
      includeCompleted ? null : completedSummaryQuery.error || null,
    pauseAutomation: pauseMutation.mutate,
    runAutomation: runMutation.mutate,
    resumeAutomation: resumeMutation.mutate,
    renameAutomation: renameMutation.mutate,
    deleteAutomation: deleteMutation.mutate,
    refetch: refetchAutomations,
  };
}
