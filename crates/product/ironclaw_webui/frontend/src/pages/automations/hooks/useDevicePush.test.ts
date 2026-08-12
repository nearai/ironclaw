// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import {
  accountEndpointDigestsFromDetail,
  vapidPublicKeyFromDetail,
} from "./useDevicePush";

// These helpers ARE the client half of the notification-setup wire contract
// (`RegistrationChannelNotificationSetupService::project`). Regression: the
// hook kept reading the retired flat detail shape (`vapid_public_key`,
// `subscription_count`, `subscriptions[]`) after the backend moved to
// `{ registration_count, registrations[], bootstrap }`, which left the enroll
// button permanently dead and misread enrolled browsers as another account's.

test("digests: absent detail reads correlation-unavailable, never not-mine", () => {
  assert.equal(accountEndpointDigestsFromDetail(undefined), null);
  assert.equal(accountEndpointDigestsFromDetail(null), null);
});

test("digests: a fully digested registration set is returned in order", () => {
  const detail = {
    registration_count: 2,
    registrations: [
      { registration_id: "r-1", endpoint_digest: "aa" },
      { registration_id: "r-2", endpoint_digest: "bb" },
    ],
  };
  assert.deepEqual(accountEndpointDigestsFromDetail(detail), ["aa", "bb"]);
});

test("digests: partial digest coverage is unavailable, not a false non-match", () => {
  // A registration without a digest could be the local subscription's match,
  // so the set must read null (correlation unavailable) — a [] here would
  // present a genuinely enrolled browser as another account's enrollment.
  const detail = {
    registrations: [
      { registration_id: "r-1", endpoint_digest: "aa" },
      { registration_id: "r-2" },
    ],
  };
  assert.equal(accountEndpointDigestsFromDetail(detail), null);
});

test("digests: an empty registration set is a real empty answer", () => {
  // No enrollment on this account: a local subscription is provably not ours.
  assert.deepEqual(accountEndpointDigestsFromDetail({ registrations: [] }), []);
});

test("bootstrap key: read from detail.bootstrap, absent reads empty", () => {
  assert.equal(
    vapidPublicKeyFromDetail({ bootstrap: { vapid_public_key: "key-b64url" } }),
    "key-b64url",
  );
  assert.equal(vapidPublicKeyFromDetail({}), "");
  assert.equal(vapidPublicKeyFromDetail(undefined), "");
  // The retired flat spelling is dead; the client must not read it.
  assert.equal(vapidPublicKeyFromDetail({ vapid_public_key: "stale" }), "");
});
