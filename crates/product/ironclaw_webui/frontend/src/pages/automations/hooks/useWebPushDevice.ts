// @ts-nocheck
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import React from "react";
import { getWebPushStatus } from "../../../lib/api";
import {
  enrollThisBrowser,
  getWebPushBrowserState,
  unenrollThisBrowser,
} from "../../../lib/web-push";

const WEB_PUSH_STATUS_QUERY_KEY = ["web-push", "status"];

/**
 * Per-browser web-push device state for the notification-channels panel's
 * "Web app" row: the account-level enrollment summary (backend status view)
 * plus this browser's own push state, with enroll/unenroll actions.
 *
 * The account-level toggle (whether the web-push target is a notification
 * channel) stays in the ordinary draft/save set — this hook only manages the
 * device dimension underneath it.
 */
export function useWebPushDevice() {
  const queryClient = useQueryClient();
  const statusQuery = useQuery({
    queryKey: WEB_PUSH_STATUS_QUERY_KEY,
    queryFn: getWebPushStatus,
  });

  const [browser, setBrowser] = React.useState({ state: "checking" });
  React.useEffect(() => {
    let cancelled = false;
    getWebPushBrowserState().then(
      (state) => {
        if (!cancelled) setBrowser(state);
      },
      () => {
        if (!cancelled) setBrowser({ state: "unsupported" });
      },
    );
    return () => {
      cancelled = true;
    };
  }, []);

  const refreshAfterAction = async (nextState) => {
    setBrowser(nextState);
    await queryClient.invalidateQueries({ queryKey: WEB_PUSH_STATUS_QUERY_KEY });
    return nextState;
  };

  const enrollMutation = useMutation({
    mutationFn: () => {
      const vapidPublicKey = statusQuery.data?.vapid_public_key;
      return enrollThisBrowser({ vapidPublicKey });
    },
    onSuccess: (nextState) => refreshAfterAction(nextState),
  });
  const unenrollMutation = useMutation({
    mutationFn: () => unenrollThisBrowser(),
    onSuccess: (nextState) => refreshAfterAction(nextState),
  });

  return {
    browser,
    subscriptionCount: statusQuery.data?.subscription_count ?? 0,
    vapidPublicKey: statusQuery.data?.vapid_public_key || "",
    isStatusLoading: statusQuery.isLoading,
    statusError: statusQuery.error || null,
    isBusy: enrollMutation.isPending || unenrollMutation.isPending,
    actionError: enrollMutation.error || unenrollMutation.error || null,
    enroll: () => enrollMutation.mutateAsync(),
    unenroll: () => unenrollMutation.mutateAsync(),
  };
}
