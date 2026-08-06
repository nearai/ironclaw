// @ts-nocheck
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import React from "react";
import {
  getNotificationChannels,
  listOutboundDeliveryTargets,
  setNotificationChannels,
} from "../../../lib/api";

const CHANNELS_QUERY_KEY = ["outbound-delivery", "notification-channels"];
const TARGETS_QUERY_KEY = ["outbound-delivery", "targets"];

/**
 * Merge the live delivery-target catalog with the caller's stored
 * notification-channel set (Task 8's `RebornNotificationChannelsResponse`,
 * spec §7) into one row list for the multi-select panel.
 *
 * Two behaviors this merge exists to preserve — both driven by the stored
 * `channel` entry, never assumed from the catalog alone:
 *
 * 1. **Status is per-channel, not per-catalog-presence.** A catalog entry is
 *    resolvable, but resolvable is not the same claim as "this is what the
 *    notification-channels response says about this stored id" — when the id
 *    is stored, the channel's own `status` wins over a hardcoded "available".
 *    An id that is merely selectable (in the catalog, not stored) has no
 *    per-channel status opinion, so it defaults to "available".
 * 2. **A stored id absent from the catalog is not dropped, and it is not
 *    downgraded to its raw id when the backend already resolved it.** Such a
 *    "channel" carries `option: null` only when `status` is `"unavailable"`;
 *    when the backend *did* resolve it (`option` populated — e.g. reachable
 *    only through the model tool's own resolution, not the catalog listing),
 *    the row must use that option's real `display_name` / `description` /
 *    `channel`, falling back to the raw `target_id` only when `option` is
 *    genuinely absent. See `RebornNotificationChannel`'s doc comment in
 *    `crates/ironclaw_product/src/reborn_services/types.rs`.
 */
export function mergeNotificationChannelRows(targets, channels) {
  const channelById = new Map(
    channels.map((channel) => [channel.target_id, channel]),
  );
  const catalogIds = new Set(
    targets.map((option) => option?.target?.target_id).filter(Boolean),
  );
  const catalogRows = targets.map((option) => {
    const targetId = option?.target?.target_id ?? "";
    const stored = channelById.get(targetId);
    return {
      target_id: targetId,
      display_name: option?.target?.display_name || targetId,
      description: option?.target?.description || "",
      channel: option?.target?.channel || "",
      status: stored ? stored.status : "available",
      selected: channelById.has(targetId),
    };
  });
  const orphanRows = channels
    .filter((channel) => !catalogIds.has(channel.target_id))
    .map((channel) => {
      const optionTarget = channel.option?.target;
      return {
        target_id: channel.target_id,
        display_name: optionTarget?.display_name || channel.target_id,
        description: optionTarget?.description || "",
        channel: optionTarget?.channel || "",
        status: channel.status,
        selected: true,
      };
    });
  return [...catalogRows, ...orphanRows];
}

export function useNotificationChannels() {
  const queryClient = useQueryClient();
  const channelsQuery = useQuery({
    queryKey: CHANNELS_QUERY_KEY,
    queryFn: getNotificationChannels,
  });
  const targetsQuery = useQuery({
    queryKey: TARGETS_QUERY_KEY,
    queryFn: listOutboundDeliveryTargets,
  });
  const saveMutation = useMutation({
    mutationFn: (targetIds) => setNotificationChannels({ targetIds }),
    onSuccess: (response) => {
      // Reconcile from the backend response, not the posted draft — the
      // response is authoritative on resolution status (tool-evidence UI
      // rule: no optimistic success).
      queryClient.setQueryData(CHANNELS_QUERY_KEY, response);
      queryClient.invalidateQueries({ queryKey: TARGETS_QUERY_KEY });
    },
  });

  const channels = React.useMemo(
    () => channelsQuery.data?.channels ?? [],
    [channelsQuery.data],
  );
  const targets = React.useMemo(
    () => targetsQuery.data?.targets ?? [],
    [targetsQuery.data],
  );
  const rows = React.useMemo(
    () => mergeNotificationChannelRows(targets, channels),
    [targets, channels],
  );
  const selectedIds = React.useMemo(
    () => channels.map((channel) => channel.target_id),
    [channels],
  );

  return {
    rows,
    selectedIds,
    isLoading: channelsQuery.isLoading || targetsQuery.isLoading,
    isRefreshing: channelsQuery.isFetching || targetsQuery.isFetching,
    isSaving: saveMutation.isPending,
    error: channelsQuery.error || targetsQuery.error || null,
    saveError: saveMutation.error || null,
    saveNotificationChannels: (targetIds) => saveMutation.mutateAsync(targetIds),
    refetch: () => {
      channelsQuery.refetch();
      targetsQuery.refetch();
    },
  };
}
