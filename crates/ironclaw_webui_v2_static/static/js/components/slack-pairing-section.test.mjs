import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function slackPairingSectionSourceForTest() {
  const source = readFileSync(new URL("./slack-pairing-section.js", import.meta.url), "utf8");
  const lines = [];
  for (const line of source.split("\n")) {
    if (line.startsWith("import ")) continue;
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { SlackPairingSection, slackPairingCopy, slackPairingError };`;
}

function createReactStub(state) {
  return {
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

function nestedValueAfter(rendered, fragment) {
  if (!rendered || typeof rendered !== "object") {
    return undefined;
  }
  if (Array.isArray(rendered)) {
    for (const value of rendered) {
      const found = nestedValueAfter(value, fragment);
      if (found !== undefined) return found;
    }
    return undefined;
  }
  if (Array.isArray(rendered.strings)) {
    const index = rendered.strings.findIndex((part) => part.includes(fragment));
    if (index !== -1) {
      return rendered.values[index];
    }
  }
  if (Array.isArray(rendered.values)) {
    for (const value of rendered.values) {
      const found = nestedValueAfter(value, fragment);
      if (found !== undefined) return found;
    }
  }
  return undefined;
}

function assertJsonEqual(actual, expected) {
  assert.equal(JSON.stringify(actual), JSON.stringify(expected));
}

function renderSlackPairingSection({
  state = {},
  mutationState = {},
  redeemPairingCode = () => ({ success: true, message: "Slack account connected." }),
  invalidations = [],
  action,
} = {}) {
  const mutation = {
    isPending: false,
    isSuccess: false,
    isError: false,
    data: null,
    error: null,
    ...mutationState,
  };
  const context = {
    Button: "button",
    React: createReactStub(state),
    globalThis: {},
    html,
    redeemSlackPairingCode: redeemPairingCode,
    useMutation: (config) => ({
      ...mutation,
      mutate: (variables) => {
        const data = config.mutationFn(variables);
        config.onSuccess?.(data, variables);
      },
    }),
    useQueryClient: () => ({
      invalidateQueries: (query) => invalidations.push(query.queryKey),
    }),
    useT: () => (key) => `t:${key}`,
  };
  vm.runInNewContext(slackPairingSectionSourceForTest(), context);
  return {
    rendered: context.globalThis.__testExports.SlackPairingSection({ action }),
    exports: context.globalThis.__testExports,
    state,
    invalidations,
  };
}

test("slackPairingCopy uses connect-action copy before localized defaults", () => {
  const { exports } = renderSlackPairingSection();
  const t = (key) => `default:${key}`;

  assertJsonEqual(
    exports.slackPairingCopy(
      {
        title: "Pair Slack",
        instructions: "Paste the code from Slack.",
        input_placeholder: "ABC123",
        submit_label: "Connect now",
        success_message: "Connected.",
        error_message: "Try again.",
      },
      t,
    ),
    {
      title: "Pair Slack",
      instructions: "Paste the code from Slack.",
      codePlaceholder: "ABC123",
      submitLabel: "Connect now",
      successMessage: "Connected.",
      errorMessage: "Try again.",
    },
  );
  assert.equal(
    exports.slackPairingCopy({ code_placeholder: "LEGACY" }, t).codePlaceholder,
    "LEGACY",
  );
  assert.equal(exports.slackPairingCopy({}, t).title, "default:pairing.slackTitle");
});

test("SlackPairingSection disables submit while blank or pending", () => {
  let view = renderSlackPairingSection({ state: { manualCode: "   " } });
  assert.equal(valueAfter(view.rendered, "disabled="), true);

  view = renderSlackPairingSection({
    state: { manualCode: "ABC123" },
    mutationState: { isPending: true },
  });
  assert.equal(valueAfter(view.rendered, "disabled="), true);

  view = renderSlackPairingSection({ state: { manualCode: "ABC123" } });
  assert.equal(valueAfter(view.rendered, "disabled="), false);
});

test("SlackPairingSection trims button submissions, clears input, and refreshes Slack state", () => {
  const calls = [];
  const invalidations = [];
  const view = renderSlackPairingSection({
    state: { manualCode: "  ABC123  " },
    invalidations,
    redeemPairingCode: (code) => {
      calls.push(code);
      return { success: true, message: "Slack account connected." };
    },
  });

  valueAfter(view.rendered, "onClick=")();

  assertJsonEqual(calls, ["ABC123"]);
  assert.equal(view.state.manualCode, "");
  assertJsonEqual(invalidations, [
    ["extensions"],
    ["connectable-channels"],
    ["pairing", "slack"],
  ]);
});

test("SlackPairingSection treats Enter as the same trimmed submit path", () => {
  const calls = [];
  const view = renderSlackPairingSection({
    state: { manualCode: "  ENTER123  " },
    redeemPairingCode: (code) => {
      calls.push(code);
      return { success: true };
    },
  });

  valueAfter(view.rendered, "onKeyDown=")({ key: "Escape" });
  assertJsonEqual(calls, []);

  valueAfter(view.rendered, "onKeyDown=")({ key: "Enter" });
  assertJsonEqual(calls, ["ENTER123"]);
  assert.equal(view.state.manualCode, "");
});

test("SlackPairingSection renders success and structured error messages", () => {
  let view = renderSlackPairingSection({
    mutationState: {
      isSuccess: true,
      data: { message: "Custom Slack success." },
    },
  });
  assert.equal(nestedValueAfter(view.rendered, "text-xs text-emerald-300"), "Custom Slack success.");

  view = renderSlackPairingSection({
    mutationState: {
      isError: true,
      error: { payload: { error: "expired_code" }, message: "generic" },
    },
  });
  assert.equal(nestedValueAfter(view.rendered, "text-xs text-red-300"), "expired_code");
});

test("slackPairingError prefers server envelopes before generic fallback", () => {
  const { exports } = renderSlackPairingSection();

  assert.equal(exports.slackPairingError({ payload: { error: "a" }, message: "x" }, "fb"), "a");
  assert.equal(exports.slackPairingError({ payload: { message: "b" }, message: "x" }, "fb"), "b");
  assert.equal(exports.slackPairingError({ message: "c" }, "fb"), "c");
  assert.equal(exports.slackPairingError(undefined, "fb"), "fb");
});
