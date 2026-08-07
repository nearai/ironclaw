// @vitest-environment jsdom
// @ts-nocheck
import assert from "node:assert/strict";
import { QueryClient, QueryClientProvider, QueryObserver } from "@tanstack/react-query";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, test, vi } from "vitest";

const notificationChannelsApi = vi.hoisted(() => ({
  getNotificationChannels: vi.fn(),
  listOutboundDeliveryTargets: vi.fn(),
  setNotificationChannels: vi.fn(),
}));

vi.mock("../../../lib/api", () => notificationChannelsApi);

import { mergeNotificationChannelRows, useNotificationChannels } from "./useNotificationChannels";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const CHANNELS_QUERY_KEY = ["outbound-delivery", "notification-channels"];

const mountedRoots = [];

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(async () => {
  while (mountedRoots.length > 0) {
    const root = mountedRoots.pop();
    await act(async () => root.unmount());
  }
  document.body.replaceChildren();
});

/**
 * Mount the REAL `useNotificationChannels` (not a re-declared react-query
 * observer pair) so a test asserts the hook's own returned contract —
 * `selectedIds`, `saveError`, `isSaving`, and the cache its `onSuccess`
 * does or does not write.
 */
async function mountNotificationChannelsHook(queryClient) {
  const state = { current: null, renders: 0 };
  function Probe() {
    state.current = useNotificationChannels();
    state.renders += 1;
    return null;
  }
  const container = document.body.appendChild(document.createElement("div"));
  const root = createRoot(container);
  mountedRoots.push(root);
  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(Probe),
      ),
    );
  });
  return state;
}

/**
 * Pump React + react-query's scheduler until `predicate` holds.
 *
 * react-query notifies its observers through a `setTimeout(0)` batch, so a
 * settled query/mutation lands one macrotask AFTER its promise resolves.
 * Waiting on a render count (rather than on the value under assertion) keeps
 * a test from reading the stale previous render and silently passing.
 */
async function settleUntil(predicate, description) {
  for (let attempt = 0; attempt < 50; attempt += 1) {
    if (predicate()) return;
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }
  throw new Error(`timed out waiting for: ${description}`);
}

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
  // Driven through the mounted hook, not a re-declared MutationObserver: the
  // claim under test is that THIS hook's `onSuccess` reconciles from the
  // backend response, which a hand-rolled copy of the mutation config cannot
  // pin. The response deliberately differs from the posted draft
  // ("slack-alpha" resolved unavailable) so a cache written from the draft
  // would not accidentally match.
  const savedResponse = {
    channels: [channel("slack-alpha", { status: "unavailable" })],
  };
  notificationChannelsApi.getNotificationChannels.mockResolvedValue({ channels: [] });
  notificationChannelsApi.listOutboundDeliveryTargets.mockResolvedValue({
    targets: [target("slack-alpha")],
  });
  notificationChannelsApi.setNotificationChannels.mockResolvedValue(savedResponse);

  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const state = await mountNotificationChannelsHook(queryClient);
  await settleUntil(() => state.current?.isLoading === false, "both reads to settle");
  assert.deepEqual(state.current.selectedIds, []);

  const rendersBeforeSave = state.renders;
  await act(async () => {
    await state.current.saveNotificationChannels(["slack-alpha"]);
  });
  await settleUntil(
    () => state.renders > rendersBeforeSave,
    "the successful save to re-render the hook",
  );

  assert.equal(notificationChannelsApi.setNotificationChannels.mock.calls.length, 1);
  assert.deepEqual(notificationChannelsApi.setNotificationChannels.mock.calls[0][0], {
    targetIds: ["slack-alpha"],
  });
  assert.deepEqual(
    queryClient.getQueryData(CHANNELS_QUERY_KEY),
    savedResponse,
    "the cache must reconcile from the response, not the optimistic draft",
  );
  assert.deepEqual(
    state.current.rows.map((row) => [row.target_id, row.status, row.selected]),
    [["slack-alpha", "unavailable", true]],
    "the rendered rows follow the response's resolution status, not the posted ids",
  );
  assert.equal(state.current.saveError, null);
  queryClient.clear();
});

test("useNotificationChannels keeps the stored set cached and surfaces saveError when the save rejects", async () => {
  // The failure half of the tool-evidence UI rule: a rejected write produces
  // no backend evidence, so the hook must publish nothing — the cache keeps
  // the last real response, `selectedIds` keeps rendering the stored set, and
  // the rejection surfaces through `saveError` instead of a fabricated
  // success. (The success half is pinned by the test above.)
  const storedResponse = { channels: [channel("slack-alpha")] };
  notificationChannelsApi.getNotificationChannels.mockResolvedValue(storedResponse);
  notificationChannelsApi.listOutboundDeliveryTargets.mockResolvedValue({
    targets: [target("slack-alpha"), target("slack-beta")],
  });
  notificationChannelsApi.setNotificationChannels.mockRejectedValue(
    new Error("notification_channel_not_found"),
  );

  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const state = await mountNotificationChannelsHook(queryClient);
  await settleUntil(() => state.current?.isLoading === false, "both reads to settle");
  assert.deepEqual(state.current.selectedIds, ["slack-alpha"]);

  const rendersBeforeSave = state.renders;
  await act(async () => {
    await state.current.saveNotificationChannels(["slack-beta"]).catch(() => {});
  });
  await settleUntil(
    () => state.renders > rendersBeforeSave,
    "the rejected save to re-render the hook",
  );

  assert.equal(notificationChannelsApi.setNotificationChannels.mock.calls.length, 1);
  assert.deepEqual(notificationChannelsApi.setNotificationChannels.mock.calls[0][0], {
    targetIds: ["slack-beta"],
  });
  assert.deepEqual(
    queryClient.getQueryData(CHANNELS_QUERY_KEY),
    storedResponse,
    "a rejected save must leave the pre-save response in the cache — never write the posted draft",
  );
  assert.deepEqual(
    state.current.selectedIds,
    ["slack-alpha"],
    "the panel keeps rendering the stored set, not the draft the backend refused",
  );
  assert.ok(state.current.saveError instanceof Error, "saveError must carry the rejection");
  assert.match(String(state.current.saveError.message), /notification_channel_not_found/);
  assert.equal(state.current.isSaving, false, "the failed save must not stay pending");
  queryClient.clear();
});
