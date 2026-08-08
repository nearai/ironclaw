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
 * Derive the panel-facing device state from the raw browser probe plus the
 * account correlation:
 *   "checking" | "unsupported" | "permission-denied" | "not-enrolled"
 *   "enrolled"                — subscription verified to belong to THIS account
 *   "enrolled-other-account"  — a subscription exists but this account holds
 *                               no record for it (another account enrolled in
 *                               this browser profile)
 *   "enrolled-unverified"     — a subscription exists but correlation was
 *                               unavailable (e.g. the status query failed);
 *                               no disable action is offered from this state.
 */
function deriveDeviceState(probe) {
  if (!probe || probe.state !== "enrolled") return probe?.state || "checking";
  if (probe.accountMatch === true) return "enrolled";
  if (probe.accountMatch === false) return "enrolled-other-account";
  return "enrolled-unverified";
}

/**
 * Per-browser web-push device state for the notification-channels panel's
 * "Web app" row: the account-level enrollment summary (backend status view)
 * plus this browser's own push state, with enroll/unenroll actions.
 *
 * The account-level toggle (whether the web-push target is a notification
 * channel) stays in the ordinary draft/save set — this hook only manages the
 * device dimension underneath it. Browser state is correlated with the
 * signed-in account through the status view's `endpoint_digest` values, so a
 * subscription enrolled by a DIFFERENT account in the same browser profile
 * is never presented as this account's enrollment (and never offered a
 * local unsubscribe that would sever the other account).
 */
export function useWebPushDevice() {
  const queryClient = useQueryClient();
  const statusQuery = useQuery({
    queryKey: WEB_PUSH_STATUS_QUERY_KEY,
    queryFn: getWebPushStatus,
  });

  const [probe, setProbe] = React.useState(null);
  // A post-action probe result is authoritative until the status refetch
  // settles; the effect below only overwrites it once fresh data arrives.
  const statusData = statusQuery.data;
  const statusSettled = !statusQuery.isLoading;
  React.useEffect(() => {
    if (!statusSettled) return undefined;
    let cancelled = false;
    const accountEndpointDigests = statusData
      ? (statusData.subscriptions || [])
          .map((subscription) => subscription.endpoint_digest)
          .filter((digest) => typeof digest === "string" && digest)
      : null;
    getWebPushBrowserState({ accountEndpointDigests }).then(
      (state) => {
        if (!cancelled) setProbe(state);
      },
      () => {
        if (!cancelled) setProbe({ state: "unsupported" });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [statusSettled, statusData]);

  const refreshAfterAction = async (nextState) => {
    setProbe(nextState);
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
    browser: { ...(probe || {}), state: deriveDeviceState(probe) },
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
