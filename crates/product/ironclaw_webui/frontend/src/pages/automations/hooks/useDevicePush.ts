// @ts-nocheck
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import React from "react";
import { getNotificationSetupStatus } from "../../../lib/api";
import {
  enrollThisBrowser,
  getDevicePushState,
  unenrollThisBrowser,
} from "../../../lib/device-push";

const setupStatusQueryKey = (extensionId) => [
  "notification-setup",
  extensionId,
  "status",
];

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
 * The signed-in account's endpoint digests from the setup-status `detail`,
 * for correlation against this browser's own subscription digest.
 *
 * Correlation needs digest coverage of the FULL registration set: a
 * registration without a digest could be the local subscription's match, so
 * partial coverage must read "unavailable" (`null`), never "not mine". An
 * empty registration set is a real answer ([]): the account holds no
 * enrollment, so a local subscription is provably not this account's.
 */
export function accountEndpointDigestsFromDetail(detail) {
  const registrations = detail ? detail.registrations || [] : null;
  if (!registrations) return null;
  const digests = registrations
    .map((registration) => registration && registration.endpoint_digest)
    .filter((digest) => typeof digest === "string" && digest);
  return digests.length === registrations.length ? digests : null;
}

/** The channel's client bootstrap key from the setup-status `detail`
 * (`detail.bootstrap.vapid_public_key`), or "" when unpublished. */
export function vapidPublicKeyFromDetail(detail) {
  return (detail && detail.bootstrap && detail.bootstrap.vapid_public_key) || "";
}

/**
 * Per-browser push-device state for the notification-channels panel's
 * session-channel row: the account-level enrollment summary (the generic
 * notification-setup status view, keyed by the session channel's
 * `extensionId`) plus this browser's own push state, with enroll/unenroll
 * actions.
 *
 * The status response's channel-opaque `detail` is interpreted HERE — this
 * hook is part of the session channel's own client — as
 * `{ registration_count, registrations: [{ registration_id, created_at,
 * endpoint_digest }], bootstrap: { vapid_public_key } }` (the shape
 * `RegistrationChannelNotificationSetupService::project` emits, with
 * `bootstrap` published by the channel's `DeliveryClientBootstrap`).
 * The account-level toggle (whether the channel is a notification channel)
 * stays in the ordinary draft/save set — this hook only manages the device
 * dimension underneath it. Browser state is correlated with the signed-in
 * account through the registrations' `endpoint_digest` values, so a
 * subscription enrolled by a DIFFERENT account in the same browser profile is
 * never presented as this account's enrollment (and never offered a local
 * unsubscribe that would sever the other account). A registration set whose
 * digests are incomplete yields `null` (correlation unavailable), never a
 * false "other account".
 */
export function useDevicePush({ extensionId } = {}) {
  const queryClient = useQueryClient();
  const statusQuery = useQuery({
    queryKey: setupStatusQueryKey(extensionId),
    queryFn: () => getNotificationSetupStatus({ extensionId }),
    enabled: Boolean(extensionId),
  });

  const [probe, setProbe] = React.useState(null);
  // A post-action probe result is authoritative until the status refetch
  // settles; the effect below only overwrites it once fresh data arrives.
  const statusDetail = statusQuery.data?.detail;
  const statusSettled = !statusQuery.isLoading;
  React.useEffect(() => {
    if (!statusSettled) return undefined;
    let cancelled = false;
    const accountEndpointDigests = accountEndpointDigestsFromDetail(statusDetail);
    getDevicePushState({ accountEndpointDigests }).then(
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
  }, [statusSettled, statusDetail]);

  const refreshAfterAction = async (nextState) => {
    setProbe(nextState);
    await queryClient.invalidateQueries({
      queryKey: setupStatusQueryKey(extensionId),
    });
    return nextState;
  };

  const enrollMutation = useMutation({
    mutationFn: () => {
      if (!extensionId) {
        throw new Error("session channel extension is unavailable");
      }
      const vapidPublicKey = vapidPublicKeyFromDetail(statusQuery.data?.detail);
      if (!vapidPublicKey) {
        throw new Error("notification setup key is unavailable");
      }
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
    subscriptionCount: statusQuery.data?.detail?.registration_count ?? 0,
    vapidPublicKey: vapidPublicKeyFromDetail(statusQuery.data?.detail),
    isStatusLoading: statusQuery.isLoading,
    statusError: statusQuery.error || null,
    isBusy: enrollMutation.isPending || unenrollMutation.isPending,
    actionError: enrollMutation.error || unenrollMutation.error || null,
    enroll: () => enrollMutation.mutateAsync(),
    unenroll: () => unenrollMutation.mutateAsync(),
  };
}
