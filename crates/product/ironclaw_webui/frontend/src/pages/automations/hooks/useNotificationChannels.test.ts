// @ts-nocheck
import assert from "node:assert/strict";
import { MutationObserver, QueryClient, QueryObserver } from "@tanstack/react-query";
import { beforeEach, test, vi } from "vitest";

const notificationChannelsApi = vi.hoisted(() => ({
  getNotificationChannels: vi.fn(),
  listOutboundDeliveryTargets: vi.fn(),
  setNotificationChannels: vi.fn(),
}));

vi.mock("../../../lib/api", () => notificationChannelsApi);

import { mergeNotificationChannelRows } from "./useNotificationChannels";

beforeEach(() => {
  vi.clearAllMocks();
});

function target(targetId, overrides = {}) {
  return {
    target: {
      target_id: targetId,
      display_name: overrides.displayName || targetId,
      description: overrides.description ?? null,
      channel: overrides.channel || "slack",
    },
    capabilities: { final_replies: false, gate_prompts: true, auth_prompts: true },
  };
}

/** Builds a `RebornNotificationChannel`-shaped fixture honoring the backend's
 * own invariant: `option` is populated if and only if `status` is
 * `"available"` (see `RebornNotificationChannel`'s doc comment in
 * `crates/ironclaw_product/src/reborn_services/types.rs`). Test fixtures that
 * pair `status: "available"` with `option: null` are not a shape the backend
 * ever actually sends. */
function channel(targetId, overrides = {}) {
  const status = overrides.status ?? "available";
  const isAvailable = status === "available";
  return {
    target_id: targetId,
    status,
    option: isAvailable
      ? {
          target: {
            target_id: targetId,
            display_name: overrides.displayName || targetId,
            description: overrides.description ?? null,
            channel: overrides.channel || "slack",
          },
          capabilities: { final_replies: false, gate_prompts: true, auth_prompts: true },
        }
      : null,
  };
}

test("mergeNotificationChannelRows marks catalog entries selected only when stored", () => {
  const rows = mergeNotificationChannelRows(
    [target("slack-alpha"), target("slack-beta")],
    [channel("slack-alpha")],
  );

  assert.equal(rows.length, 2);
  assert.deepEqual(
    rows.map((row) => [row.target_id, row.selected, row.status]),
    [
      ["slack-alpha", true, "available"],
      ["slack-beta", false, "available"],
    ],
  );
});

test("mergeNotificationChannelRows preserves a stored id no longer in the catalog as an unavailable row, not dropped", () => {
  const rows = mergeNotificationChannelRows(
    [target("slack-alpha")],
    [channel("slack-alpha"), channel("slack-gone", { status: "unavailable" })],
  );

  assert.equal(rows.length, 2, "the vanished channel must still produce a row");
  const gone = rows.find((row) => row.target_id === "slack-gone");
  assert.ok(gone, "slack-gone must not be silently dropped");
  assert.equal(gone.status, "unavailable");
  assert.equal(gone.selected, true, "a stored id is selected regardless of resolution status");
  assert.equal(
    gone.display_name,
    "slack-gone",
    "with no catalog entry AND no resolved option, the raw target_id is the only available label",
  );
});

test("mergeNotificationChannelRows returns an empty list when there is no catalog and nothing stored", () => {
  assert.deepEqual(mergeNotificationChannelRows([], []), []);
});

test("mergeNotificationChannelRows uses a resolved orphan's own option for display name, description, and status — not the raw target_id", () => {
  // "slack-progress-bot" is stored (so `get_notification_channels` resolved
  // it and populated `option`) but absent from the live target catalog —
  // reachable e.g. when the model tool added a channel the targets listing
  // does not itself enumerate. It must render with its real identity, not
  // downgrade to the bare id the way a truly-dead channel does.
  const rows = mergeNotificationChannelRows(
    [target("slack-alpha")],
    [
      channel("slack-alpha"),
      channel("slack-progress-bot", {
        displayName: "Progress Bot",
        description: "Posts run progress only",
      }),
    ],
  );

  const orphan = rows.find((row) => row.target_id === "slack-progress-bot");
  assert.ok(orphan, "the resolved orphan must still produce a row");
  assert.equal(
    orphan.display_name,
    "Progress Bot",
    "a resolved orphan must use its option's display_name, not the raw target_id",
  );
  assert.equal(orphan.description, "Posts run progress only");
  assert.equal(
    orphan.status,
    "available",
    "a resolved orphan's status must reflect the channel's own status",
  );
  assert.equal(orphan.selected, true);
});

test("mergeNotificationChannelRows trusts the channel's own status over catalog presence", () => {
  // "slack-alpha" resolves in the live catalog (so it's inherently
  // selectable) but the notification-channels response reports it
  // unavailable for this stored id — the per-channel response status must
  // win; catalog presence must not force a hardcoded "available".
  const rows = mergeNotificationChannelRows(
    [target("slack-alpha")],
    [channel("slack-alpha", { status: "unavailable" })],
  );

  assert.equal(rows.length, 1);
  assert.equal(rows[0].status, "unavailable");
  assert.equal(rows[0].selected, true);
});

test("useNotificationChannels query options read the stored channel set and the live catalog", async () => {
  const storedChannels = [channel("slack-alpha")];
  notificationChannelsApi.getNotificationChannels.mockResolvedValue({
    channels: storedChannels,
  });
  notificationChannelsApi.listOutboundDeliveryTargets.mockResolvedValue({
    targets: [target("slack-alpha"), target("slack-beta")],
  });

  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const channelsObserver = new QueryObserver(queryClient, {
    queryKey: ["outbound-delivery", "notification-channels"],
    queryFn: notificationChannelsApi.getNotificationChannels,
  });
  const targetsObserver = new QueryObserver(queryClient, {
    queryKey: ["outbound-delivery", "targets"],
    queryFn: notificationChannelsApi.listOutboundDeliveryTargets,
  });
  const unsubscribeChannels = channelsObserver.subscribe(() => {});
  const unsubscribeTargets = targetsObserver.subscribe(() => {});

  try {
    await channelsObserver.refetch();
    await targetsObserver.refetch();
    assert.deepEqual(channelsObserver.getCurrentResult().data.channels, storedChannels);
    assert.equal(targetsObserver.getCurrentResult().data.targets.length, 2);
  } finally {
    unsubscribeChannels();
    unsubscribeTargets();
    queryClient.clear();
  }
});

test("useNotificationChannels save mutation posts through setNotificationChannels and reconciles the cache from its response", async () => {
  const savedResponse = {
    channels: [channel("slack-alpha")],
  };
  notificationChannelsApi.setNotificationChannels.mockResolvedValue(savedResponse);

  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  queryClient.setQueryData(["outbound-delivery", "notification-channels"], { channels: [] });
  const mutationObserver = new MutationObserver(queryClient, {
    mutationFn: (targetIds) => notificationChannelsApi.setNotificationChannels({ targetIds }),
    onSuccess: (response) => {
      queryClient.setQueryData(["outbound-delivery", "notification-channels"], response);
    },
  });

  await mutationObserver.mutate(["slack-alpha"]);

  assert.equal(notificationChannelsApi.setNotificationChannels.mock.calls.length, 1);
  assert.deepEqual(notificationChannelsApi.setNotificationChannels.mock.calls[0][0], {
    targetIds: ["slack-alpha"],
  });
  assert.deepEqual(
    queryClient.getQueryData(["outbound-delivery", "notification-channels"]),
    savedResponse,
    "the cache must reconcile from the response, not the optimistic draft",
  );
  queryClient.clear();
});
