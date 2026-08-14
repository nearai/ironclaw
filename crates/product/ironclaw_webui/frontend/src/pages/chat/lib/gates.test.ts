// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

// The real frame normalizer is injected rather than stubbed: a gate carrying a
// device-link frame must be normalized by exactly the code the poll path uses,
// or the two disagree about the same wire object.
import { deviceLinkFrameFromWire } from "../../../lib/device-link-frame";

const GATES_EXPORTS = [
  "gateFromEvent",
  "gateFromProjectionGate",
  "channelConnectionFromGate",
  "deviceLinkFromGate",
  "gateIsDeviceLink",
];

function loadGates() {
  const source = readFileSync(new URL("./gates.ts", import.meta.url), "utf8")
    .split("\n")
    .filter((line) => !line.startsWith("import "))
    .join("\n")
    .replace(
      new RegExp(`export function (${GATES_EXPORTS.join("|")})`, "g"),
      "function $1",
    );
  const context = { globalThis: {}, deviceLinkFrameFromWire };
  vm.runInNewContext(
    `${source}\nglobalThis.__testExports = { ${GATES_EXPORTS.join(", ")} };`,
    context,
  );
  return context.globalThis.__testExports;
}

function plain(value) {
  return JSON.parse(JSON.stringify(value));
}

test("gateFromEvent maps approval always-allow affordance", () => {
  const { gateFromEvent } = loadGates();

  assert.deepEqual(
    plain(gateFromEvent("gate", {
      turn_run_id: "run-1",
      gate_ref: "gate:approval",
      headline: "Approval required",
      body: "Review the action.",
      allow_always: true,
    })),
    {
      kind: "gate",
      gateKind: "approval",
      runId: "run-1",
      gateRef: "gate:approval",
      invocationId: null,
      headline: "Approval required",
      body: "Review the action.",
      allowAlways: true,
    },
  );
});

test("gateFromEvent defaults missing always-allow affordance to false", () => {
  const { gateFromEvent } = loadGates();

  assert.deepEqual(
    plain(gateFromEvent("gate", {
      turn_run_id: "run-1",
      gate_ref: "gate:resource",
      headline: "Resource unavailable",
      body: "Try later.",
    })),
    {
      kind: "gate",
      gateKind: "approval",
      runId: "run-1",
      gateRef: "gate:resource",
      invocationId: null,
      headline: "Resource unavailable",
      body: "Try later.",
      allowAlways: false,
    },
  );
});
test("gateFromEvent maps approval context into readable approval card props", () => {
  const { gateFromEvent } = loadGates();

  const gate = plain(gateFromEvent("gate", {
    turn_run_id: "run-1",
    gate_ref: "gate:approval-1",
    headline: "Approval required",
    body: "capability requires approval",
    allow_always: true,
    approval_context: {
      tool_name: "builtin.http",
      action: { label: "Run tool" },
      scope: { label: "This request only", reusable: false },
      reason: "approval required for Dispatch of builtin.http",
      destination: {
        label: "GET https://example.com",
        url: "https://example.com",
        domain: "example.com",
      },
      details: [
        { label: "Capability", value: "builtin.http" },
        { label: "Estimated network egress", value: "4096 bytes" },
      ],
    },
  }));

  assert.equal(gate.allowAlways, true);
  assert.equal(gate.toolName, "builtin.http");
  assert.equal(gate.description, "approval required for Dispatch of builtin.http");
  assert.equal(gate.destination.domain, "example.com");
  assert.deepEqual(gate.approvalScope, {
    label: "This request only",
    reusable: false,
  });
  assert.deepEqual(gate.approvalDetails, [
    { label: "Action", labelKey: "approval.detail.action", value: "Run tool" },
    { label: "Destination", labelKey: "approval.detail.destination", value: "GET https://example.com" },
    { label: "Scope", labelKey: "approval.detail.scope", value: "This request only" },
    { label: "Capability", value: "builtin.http" },
    { label: "Estimated network egress", value: "4096 bytes" },
  ]);
  assert.match(gate.parameters, /Estimated network egress: 4096 bytes/);
});

test("gateFromProjectionGate ignores approval context from durable projection", () => {
  const { gateFromProjectionGate } = loadGates();

  const gate = plain(gateFromProjectionGate({
    run_id: "run-1",
    gate_kind: "approval",
    gate_ref: "gate:approval-1",
    invocation_id: "invocation-1",
    headline: "Approval required",
    body: "capability requires approval",
    allow_always: true,
    approval_context: {
      tool_name: "builtin.http",
      reason: "raw path /Users/test/.ssh/id_rsa and token sk-secret",
      details: [{ label: "Secret", value: "sk-secret" }],
    },
  }));

  assert.deepEqual(gate, {
    kind: "gate",
    gateKind: "approval",
    runId: "run-1",
    gateRef: "gate:approval-1",
    invocationId: "invocation-1",
    headline: "Approval required",
    body: "capability requires approval",
    allowAlways: true,
  });
});

test("gateFromEvent keeps modern auth prompts without challenge kind off token card", () => {
  const { gateFromEvent } = loadGates();

  assert.deepEqual(
    plain(gateFromEvent("auth_required", {
      turn_run_id: "run-auth",
      auth_request_ref: "gate:auth",
      headline: "Authentication required",
      body: "Google authentication required",
      provider: "google",
    })),
    {
      kind: "auth_required",
      gateKind: "auth",
      challengeKind: "other",
      connection: null,
      deviceLink: null,
      runId: "run-auth",
      gateRef: "gate:auth",
      invocationId: null,
      provider: "google",
      accountLabel: "",
      authorizationUrl: null,
      expiresAt: null,
      headline: "Authentication required",
      body: "Google authentication required",
    },
  );
});

test("gateFromEvent passes the oauth_url challenge kind through unchanged", () => {
  const { gateFromEvent } = loadGates();

  assert.deepEqual(
    plain(gateFromEvent("auth_required", {
      turn_run_id: "run-auth",
      auth_request_ref: "gate:auth",
      headline: "Authentication required",
      body: "Google authentication required",
      // Stable wire value for a browser OAuth relay challenge.
      challenge_kind: "oauth_url",
      provider: "google",
    })),
    {
      kind: "auth_required",
      gateKind: "auth",
      challengeKind: "oauth_url",
      connection: null,
      deviceLink: null,
      runId: "run-auth",
      gateRef: "gate:auth",
      invocationId: null,
      provider: "google",
      accountLabel: "",
      authorizationUrl: null,
      expiresAt: null,
      headline: "Authentication required",
      body: "Google authentication required",
    },
  );
});

test("gateFromEvent defaults, passes through challenge kinds, and carries channel-pairing connection context", () => {
  const { gateFromEvent } = loadGates();

  // Missing challenge_kind on a legacy prompt defaults to the paste-a-secret
  // kind.
  assert.equal(
    gateFromEvent("auth_required", {
      turn_run_id: "run-auth",
      auth_request_ref: "gate:auth",
    }).challengeKind,
    "manual_token",
  );

  // The `manual_token` value passes through unchanged.
  assert.equal(
    gateFromEvent("auth_required", {
      turn_run_id: "run-auth",
      auth_request_ref: "gate:auth",
      challenge_kind: "manual_token",
    }).challengeKind,
    "manual_token",
  );

  // A host-issued channel pairing gate carries normalized manifest context.
  const pairing = gateFromEvent("auth_required", {
    turn_run_id: "run-pair",
    auth_request_ref: "gate:pair",
    challenge_kind: "pairing",
    connection: {
      channel: "slack",
      strategy: "web_generated_code",
      instructions: "Open the app with the generated link.",
      submit_label: "Connect",
      error_message: "Invalid code.",
    },
  });
  assert.equal(pairing.challengeKind, "pairing");
  assert.deepEqual(plain(pairing.connection), {
    channel: "slack",
    strategy: "web_generated_code",
    instructions: "Open the app with the generated link.",
    inputPlaceholder: null,
    submitLabel: "Connect",
    errorMessage: "Invalid code.",
  });
});

test("channelConnectionFromGate selects only pairing gates that carry connection context", () => {
  const { gateFromEvent, channelConnectionFromGate } = loadGates();

  // A host-issued pairing gate WITH connection context is a channel-connection
  // gate. chat.tsx derives BOTH the composer affordance
  // (activeThreadHasChannelConnectionGate) and the pairing-card selector from
  // this single predicate so they cannot disagree.
  const pairing = gateFromEvent("auth_required", {
    turn_run_id: "run-pair",
    auth_request_ref: "gate:pair",
    challenge_kind: "pairing",
    connection: { channel: "slack", strategy: "web_generated_code" },
  });
  assert.equal(channelConnectionFromGate(pairing), pairing.connection);
  assert.equal(channelConnectionFromGate(pairing).channel, "slack");

  // A manual_token gate that somehow carried a connection must NOT be treated
  // as a channel-connection gate: it renders the token-paste card, so the
  // composer must not promise "finish pairing" for it. Backend invariant
  // (crates/ironclaw_product_workflow/src/auth_prompt.rs): `connection` is only
  // ever populated on `challenge_kind == pairing`, so this shape can't occur in
  // production — this pins the frontend against drift if that invariant changes.
  const manualWithConnection = {
    ...gateFromEvent("auth_required", {
      turn_run_id: "run-token",
      auth_request_ref: "gate:token",
      challenge_kind: "manual_token",
    }),
    connection: { channel: "slack", strategy: "web_generated_code" },
  };
  assert.equal(channelConnectionFromGate(manualWithConnection), null);

  // A pairing gate WITHOUT connection (the credential-requirement fallback that
  // sets challenge_kind=pairing but carries no manifest context) falls through
  // to the generic auth card, not the pairing card.
  const pairingNoConnection = gateFromEvent("auth_required", {
    turn_run_id: "run-pair2",
    auth_request_ref: "gate:pair2",
    challenge_kind: "pairing",
  });
  assert.equal(channelConnectionFromGate(pairingNoConnection), null);

  // Approval gates and a null gate never carry channel connection.
  assert.equal(
    channelConnectionFromGate(
      gateFromEvent("gate", {
        turn_run_id: "run-approval",
        gate_ref: "gate:approval",
        headline: "h",
        body: "b",
      }),
    ),
    null,
  );
  assert.equal(channelConnectionFromGate(null), null);
});

test("gateFromProjectionGate normalizes the connection context from auth_context", () => {
  const { gateFromProjectionGate } = loadGates();

  const gate = gateFromProjectionGate({
    run_id: "run-1",
    gate_kind: "auth",
    gate_ref: "gate:pair",
    auth_context: {
      challenge_kind: "pairing",
      connection: {
        channel: "slack",
        strategy: "web_generated_code",
      },
    },
  });

  assert.equal(gate.challengeKind, "pairing");
  assert.deepEqual(plain(gate.connection), {
    channel: "slack",
    strategy: "web_generated_code",
    instructions: null,
    inputPlaceholder: null,
    submitLabel: null,
    errorMessage: null,
  });
});

test("gateFromEvent normalizes a device-link frame and gates it behind the challenge kind", () => {
  const { gateFromEvent, deviceLinkFromGate, gateIsDeviceLink } = loadGates();

  const wire = {
    turn_run_id: "run-link",
    auth_request_ref: "gate:link",
    headline: "Link your account",
    body: "Scan the code to link this device.",
    challenge_kind: "device_link",
    provider: "telegram",
    device_link: {
      provider: "telegram",
      display_name: "Telegram",
      step: "display",
      instructions: "Open Telegram and scan this.",
      qr_payload: "tg://login?token=AAAA",
      expires_at: "2026-07-16T12:01:30Z",
      revision: 3,
      poll_interval_ms: 3000,
    },
  };

  const gate = gateFromEvent("auth_required", wire);

  assert.equal(gate.challengeKind, "device_link");
  assert.equal(gateIsDeviceLink(gate), true);
  assert.equal(gate.deviceLink.qrPayload, "tg://login?token=AAAA");
  assert.equal(gate.deviceLink.revision, 3);
  assert.equal(gate.deviceLink.step, "display");
  assert.equal(gate.deviceLink.terminal, false);
  assert.equal(deviceLinkFromGate(gate), gate.deviceLink);

  // A device-link gate is never also a channel-connection gate: the two
  // selectors must never both claim the same gate.
  assert.equal(gate.connection, null);

  // The frame is optional even on a device-link gate (a row written before the
  // field existed); the card starts its own flow rather than refusing.
  const frameless = gateFromEvent("auth_required", { ...wire, device_link: undefined });
  assert.equal(gateIsDeviceLink(frameless), true);
  assert.equal(deviceLinkFromGate(frameless), null);

  // And a frame riding a NON-device-link gate is never treated as one.
  const strayFrame = gateFromEvent("auth_required", { ...wire, challenge_kind: "manual_token" });
  assert.equal(gateIsDeviceLink(strayFrame), false);
  assert.equal(deviceLinkFromGate(strayFrame), null);
  assert.equal(deviceLinkFromGate(null), null);
});

test("gateFromProjectionGate normalizes the device-link frame from auth_context", () => {
  const { gateFromProjectionGate, deviceLinkFromGate } = loadGates();

  const gate = gateFromProjectionGate({
    run_id: "run-link",
    gate_kind: "auth",
    gate_ref: "gate:link",
    auth_context: {
      challenge_kind: "device_link",
      device_link: {
        provider: "telegram",
        display_name: "Telegram",
        step: "input_required",
        instructions: "Telegram needs one more value.",
        secret_label: "Login code",
        input_kind: "code",
        expires_at: "2026-07-16T12:01:30Z",
        revision: 5,
        poll_interval_ms: 3000,
      },
    },
  });

  assert.equal(gate.challengeKind, "device_link");
  assert.equal(deviceLinkFromGate(gate).secretLabel, "Login code");
  assert.equal(deviceLinkFromGate(gate).inputKind, "code");
  assert.equal(deviceLinkFromGate(gate).revision, 5);
});
