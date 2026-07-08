// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

function pairingSectionSourceForTest() {
  const source = readFileSync(new URL("./pairing-section.ts", import.meta.url), "utf8");
  const lines = [];
  for (const line of source.split("\n")) {
    if (line.startsWith("import ")) continue;
    lines.push(line.replace("export function PairingSection", "function PairingSection"));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { PairingSection };`;
}

function createReactStub(state) {
  return {
    useCallback: (fn) => fn,
    useEffect: () => {},
    useState: (initial) => {
      if (state.manualCode === undefined) {
        state.manualCode = typeof initial === "function" ? initial() : initial;
      }
      return [
        state.manualCode,
        (next) => {
          state.manualCode = typeof next === "function" ? next(state.manualCode) : next;
        },
      ];
    },
  };
}

function html(strings, ...values) {
  return { strings: Array.from(strings), values };
}

function valueAfter(rendered, fragment) {
  const index = rendered.strings.findIndex((part) => part.includes(fragment));
  assert.notEqual(index, -1, `expected template fragment ${fragment}`);
  return rendered.values[index];
}

function renderPairingSection(context, props) {
  return context.globalThis.__testExports.PairingSection(props);
}

test("PairingSection custom redeem trims code and invalidates configured queries on success", () => {
  const state = {};
  const redeemCalls = [];
  const invalidations = [];
  const context = {
    Button: "button",
    React: createReactStub(state),
    globalThis: {},
    html,
    pairingErrorMessage: () => "error",
    useMutation: (config) => ({
      isPending: false,
      isSuccess: false,
      isError: false,
      mutate: (variables) => {
        const data = config.mutationFn(variables);
        config.onSuccess(data, variables);
      },
    }),
    usePairing: () => ({
      requests: [],
      isLoading: false,
      approve: () => {
        throw new Error("default pairing approve should not be used");
      },
      isApproving: false,
      result: null,
      error: null,
    }),
    useQueryClient: () => ({
      invalidateQueries: (query) => invalidations.push(query.queryKey),
    }),
    useT: () => (key) => key,
  };
  vm.runInNewContext(pairingSectionSourceForTest(), context);

  let rendered = renderPairingSection(context, {
    channel: "telegram",
    redeemFn: (channel, code) => {
      redeemCalls.push({ channel, code });
      return { success: true };
    },
    queryKeys: [["extensions"], ["pairing", "telegram"]],
    showPendingRequests: false,
  });
  valueAfter(rendered, "onChange=")({ target: { value: "  A1B2C3  " } });

  rendered = renderPairingSection(context, {
    channel: "telegram",
    redeemFn: (channel, code) => {
      redeemCalls.push({ channel, code });
      return { success: true };
    },
    queryKeys: [["extensions"], ["pairing", "telegram"]],
    showPendingRequests: false,
  });
  valueAfter(rendered, "onClick=")();

  assert.deepEqual(redeemCalls, [{ channel: "telegram", code: "A1B2C3" }]);
  assert.deepEqual(invalidations, [["extensions"], ["pairing", "telegram"]]);
  assert.equal(state.manualCode, "");
});

test("PairingSection custom redeem is a no-op for blank manual input", () => {
  const state = { manualCode: "   " };
  let redeemCount = 0;
  const context = {
    Button: "button",
    React: createReactStub(state),
    globalThis: {},
    html,
    pairingErrorMessage: () => "error",
    useMutation: (config) => ({
      isPending: false,
      isSuccess: false,
      isError: false,
      mutate: (variables) => {
        redeemCount += 1;
        config.mutationFn(variables);
      },
    }),
    usePairing: () => ({
      requests: [],
      isLoading: false,
      approve: () => {},
      isApproving: false,
      result: null,
      error: null,
    }),
    useQueryClient: () => ({ invalidateQueries: () => {} }),
    useT: () => (key) => key,
  };
  vm.runInNewContext(pairingSectionSourceForTest(), context);

  const rendered = renderPairingSection(context, {
    channel: "telegram",
    redeemFn: () => ({ success: true }),
    showPendingRequests: false,
  });
  valueAfter(rendered, "onClick=")();

  assert.equal(redeemCount, 0);
  assert.equal(state.manualCode, "   ");
});
