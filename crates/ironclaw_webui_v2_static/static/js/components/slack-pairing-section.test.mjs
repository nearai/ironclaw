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
  return `${lines.join("\n")}\nglobalThis.__testExports = { SlackPairingSection };`;
}

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
  const invalidations = [];
  let mutationConfig = null;
  const context = {
    activateExtension: async () => {
      throw new Error("activation boom");
    },
    Button() {},
    console: { error: () => {} },
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
  assert.deepEqual(JSON.parse(JSON.stringify(invalidations)), [
    ["extensions"],
    ["connectable-channels"],
    ["pairing", "slack"],
  ]);
});
