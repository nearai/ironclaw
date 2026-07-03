import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function slackPairingSectionSourceForTest() {
  const source = readFileSync(new URL("./slack-pairing-section.js", import.meta.url), "utf8");
  const lines = [];
  let skippingImport = false;
  for (const line of source.split("\n")) {
    if (!skippingImport && line.startsWith("import ")) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    if (skippingImport) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { SlackPairingSection, slackPairingCopy };`;
}

test("SlackPairingSection fallback copy tells users to DM the Slack app first", () => {
  const context = {
    globalThis: {},
    html: () => ({}),
  };

  vm.runInNewContext(slackPairingSectionSourceForTest(), context);
  const copy = context.globalThis.__testExports.slackPairingCopy(
    {},
    (key) => ({
      "pairing.slackTitle": "Slack account connection",
      "pairing.slackInstructions":
        "Message the IronClaw Reborn app in Slack to get a pairing code, then paste it here. Codes expire in 10 minutes. If a code is invalid or expired, run /pair in Slack for a fresh one.",
      "pairing.slackPlaceholder": "Enter Slack pairing code...",
      "pairing.connect": "Connect",
      "pairing.slackSuccess": "Slack account connected.",
      "pairing.slackError":
        "Invalid or expired Slack pairing code. Run /pair in Slack to get a new one.",
    })[key],
  );

  assert.match(copy.instructions, /Message the IronClaw Reborn app/);
  assert.match(copy.instructions, /run \/pair in Slack/);
});

test("SlackPairingSection activates Slack after redeeming a pairing code", async () => {
  const calls = [];
  const invalidations = [];
  let mutationConfig = null;
  const context = {
    activateExtension: async (packageRef) => {
      calls.push(["activate", packageRef]);
    },
    Button() {},
    globalThis: {},
    html: () => ({}),
    React: { useState: (initial) => [initial, () => {}] },
    redeemSlackPairingCode: async (code) => {
      calls.push(["redeem", code]);
      return { success: true, message: "Slack account connected." };
    },
    slackPairingError: () => "",
    useMutation: (config) => {
      mutationConfig = config;
      return {
        data: null,
        error: null,
        isError: false,
        isPending: false,
        isSuccess: false,
        mutate() {},
      };
    },
    useQueryClient: () => ({
      invalidateQueries: ({ queryKey }) => invalidations.push(queryKey),
    }),
    useT: () => (key) => key,
  };

  vm.runInNewContext(slackPairingSectionSourceForTest(), context);
  context.globalThis.__testExports.SlackPairingSection({ action: {} });

  const result = await mutationConfig.mutationFn({ code: "ABCD1234" });
  mutationConfig.onSuccess();

  assert.deepEqual(result, { success: true, message: "Slack account connected." });
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [
    ["redeem", "ABCD1234"],
    ["activate", { id: "slack" }],
  ]);
  assert.deepEqual(JSON.parse(JSON.stringify(invalidations)), [
    ["extensions"],
    ["connectable-channels"],
    ["pairing", "slack"],
  ]);
});

test("SlackPairingSection treats post-redeem activation failure as best-effort", async () => {
  const calls = [];
  const consoleErrors = [];
  const invalidations = [];
  let mutationConfig = null;
  const context = {
    activateExtension: async () => {
      throw new Error("activation boom");
    },
    Button() {},
    console: { error: (...args) => consoleErrors.push(args) },
    globalThis: {},
    html: () => ({}),
    React: { useState: (initial) => [initial, () => {}] },
    redeemSlackPairingCode: async (code) => {
      calls.push(["redeem", code]);
      return { success: true, message: "Slack account connected." };
    },
    slackPairingError: () => "",
    useMutation: (config) => {
      mutationConfig = config;
      return {
        data: null,
        error: null,
        isError: false,
        isPending: false,
        isSuccess: false,
        mutate() {},
      };
    },
    useQueryClient: () => ({
      invalidateQueries: ({ queryKey }) => invalidations.push(queryKey),
    }),
    useT: () => (key) => key,
  };

  vm.runInNewContext(slackPairingSectionSourceForTest(), context);
  context.globalThis.__testExports.SlackPairingSection({ action: {} });

  // Redemption succeeded; the failing activation must not reject the mutation,
  // otherwise a connected account is shown to the user as a pairing failure.
  const result = await mutationConfig.mutationFn({ code: "ABCD1234" });
  mutationConfig.onSuccess();

  assert.deepEqual(result, { success: true, message: "Slack account connected." });
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [["redeem", "ABCD1234"]]);
  assert.deepEqual(consoleErrors, [["Slack activation after pairing failed."]]);
  assert.deepEqual(JSON.parse(JSON.stringify(invalidations)), [
    ["extensions"],
    ["connectable-channels"],
    ["pairing", "slack"],
  ]);
});
