// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

function telegramSetupPanelSourceForTest() {
  const source = readFileSync(new URL("./telegram-setup-panel.tsx", import.meta.url), "utf8");
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { TelegramAdminManagedSection, TelegramSetupPanel, FIELD_HELP, telegramSetupCopy };`;
}

function createReactStub(state) {
  return {
    useState: (initial) => {
      const index = state.hookIndex++;
      if (!(index in state.values)) {
        state.values[index] = typeof initial === "function" ? initial() : initial;
      }
      return [
        state.values[index],
        (next) => {
          state.values[index] =
            typeof next === "function" ? next(state.values[index]) : next;
        },
      ];
    },
    useRef: (initial) => {
      const index = state.hookIndex++;
      if (!(index in state.refs)) {
        state.refs[index] = { current: initial };
      }
      return state.refs[index];
    },
    useEffect: (effect, deps) => {
      const index = state.hookIndex++;
      const dep = deps?.[0];
      if (state.effectDeps[index] !== dep) {
        state.effectDeps[index] = dep;
        effect();
      }
    },
  };
}

function renderPanel(context, state, setupQuery) {
  state.hookIndex = 0;
  state.mutationIndex = 0;
  return context.globalThis.__testExports.TelegramSetupPanel({
    action: {},
    setupQuery,
  });
}

function valuesAfter(rendered, fragment) {
  const matches = [];
  collectValuesAfter(rendered, fragment, matches);
  return matches;
}

function collectValuesAfter(value, fragment, matches) {
  if (Array.isArray(value)) {
    for (const item of value) collectValuesAfter(item, fragment, matches);
    return;
  }
  if (!value || !Array.isArray(value.strings) || !Array.isArray(value.values)) {
    return;
  }
  value.strings.forEach((part, index) => {
    if (part.includes(fragment)) {
      matches.push(value.values[index]);
    }
  });
  value.values.forEach((item) => collectValuesAfter(item, fragment, matches));
}

function setupContext(state, { saveResponses = [], mutationOverrides = {}, confirmResult = true } = {}) {
  const invalidations = [];
  const setQueryDataCalls = [];
  const clearCalls = [];
  const saveCalls = [];
  const confirmCalls = [];
  state.mutationSuccess = state.mutationSuccess || {};
  const context = {
    Button: "button",
    React: createReactStub(state),
    globalThis: {},
    getTelegramSetup: () => ({}),
    saveTelegramSetup: (form) => {
      saveCalls.push(JSON.parse(JSON.stringify(form)));
      return saveResponses.shift();
    },
    clearTelegramSetup: () => {
      clearCalls.push("delete");
      return undefined;
    },
    telegramSetupError: (error, fallback) => error?.payload?.error || fallback,
    useQuery: () => ({}),
    useQueryClient: () => ({
      setQueryData: (...args) => setQueryDataCalls.push(args),
      invalidateQueries: (query) => invalidations.push(query.queryKey),
    }),
    useMutation: (config) => {
      const index = state.mutationIndex++;
      if (!(index in state.mutationSuccess)) {
        state.mutationSuccess[index] = false;
      }
      return {
        isPending: false,
        isSuccess: state.mutationSuccess[index],
        isError: false,
        mutate: (variables) => {
          const data = config.mutationFn(variables);
          state.mutationSuccess[index] = true;
          config.onSuccess(data, variables);
        },
        reset: () => {
          state.mutationSuccess[index] = false;
        },
        ...(mutationOverrides[index] || {}),
      };
    },
    useT: () => (key) => key,
    window: {
      confirm: (message) => {
        confirmCalls.push(message);
        return confirmResult;
      },
    },
  };
  vm.runInNewContext(telegramSetupPanelSourceForTest(), context);
  return { context, invalidations, setQueryDataCalls, clearCalls, saveCalls, confirmCalls };
}

test("TelegramSetupPanel never echoes a saved token: secret field stays blank with keep-placeholder", () => {
  const state = { hookIndex: 0, mutationIndex: 0, values: {}, refs: {}, effectDeps: {} };
  const { context } = setupContext(state);
  const status = {
    configured: true,
    bot_username: "ironclaw_bot",
    bot_token_configured: true,
    webhook_url: "https://assistant.example.com",
    revision: 4,
  };

  renderPanel(context, state, { data: status });
  const rendered = renderPanel(context, state, { data: status });

  // The secret input's value is the blank form field, never a stored secret.
  assert.equal(state.values[0].bot_token, "");
  const passwordValues = valuesAfter(rendered, 'type="password"');
  assert.equal(passwordValues.length, 1);
  const placeholders = valuesAfter(rendered, "placeholder=");
  assert.ok(
    placeholders.includes("telegramSetup.placeholder.keepSecret"),
    "configured token renders the keep-blank placeholder",
  );
  // Status line surfaces the configured bot identity.
  assert.ok(JSON.stringify(rendered).includes("ironclaw_bot"));
});

test("TelegramSetupPanel lets a blank token ride when one is saved, and requires it otherwise", () => {
  const state = { hookIndex: 0, mutationIndex: 0, values: {}, refs: {}, effectDeps: {} };
  const savedStatus = {
    configured: true,
    bot_username: "ironclaw_bot",
    bot_token_configured: true,
    webhook_url: null,
    revision: 2,
  };
  const { context, saveCalls } = setupContext(state, { saveResponses: [savedStatus] });
  const configuredStatus = {
    configured: true,
    bot_username: "ironclaw_bot",
    bot_token_configured: true,
    webhook_url: "",
    revision: 1,
  };

  renderPanel(context, state, { data: configuredStatus });
  let rendered = renderPanel(context, state, { data: configuredStatus });
  // Configured: blank token is submittable ("leave blank to keep").
  assert.equal(valuesAfter(rendered, "disabled=")[0], false);
  valuesAfter(rendered, "onClick=")[0]();
  assert.deepEqual(saveCalls, [{ bot_token: "", webhook_url: "" }]);

  const freshState = { hookIndex: 0, mutationIndex: 0, values: {}, refs: {}, effectDeps: {} };
  const fresh = setupContext(freshState);
  const unconfigured = { configured: false, bot_username: null, bot_token_configured: false, webhook_url: null };
  renderPanel(fresh.context, freshState, { data: unconfigured });
  rendered = renderPanel(fresh.context, freshState, { data: unconfigured });
  // Unconfigured: a token is required before save enables.
  assert.equal(valuesAfter(rendered, "disabled=")[0], true);
});

test("TelegramSetupPanel clears the typed secret and refreshes caches after a successful save", () => {
  const state = { hookIndex: 0, mutationIndex: 0, values: {}, refs: {}, effectDeps: {} };
  const savedStatus = {
    configured: true,
    bot_username: "ironclaw_bot",
    bot_token_configured: true,
    webhook_url: "https://assistant.example.com",
    revision: 3,
  };
  const { context, invalidations, setQueryDataCalls, saveCalls } = setupContext(state, {
    saveResponses: [savedStatus],
  });
  const initialStatus = {
    configured: false,
    bot_username: null,
    bot_token_configured: false,
    webhook_url: null,
  };

  renderPanel(context, state, { data: initialStatus });
  let rendered = renderPanel(context, state, { data: initialStatus });
  valuesAfter(rendered, "onChange=")[0]({ target: { value: " 123456789:AAnew " } });
  valuesAfter(rendered, "onChange=")[1]({ target: { value: "https://assistant.example.com" } });
  rendered = renderPanel(context, state, { data: initialStatus });

  valuesAfter(rendered, "onClick=")[0]();

  assert.deepEqual(saveCalls, [
    { bot_token: " 123456789:AAnew ", webhook_url: "https://assistant.example.com" },
  ]);
  assert.deepEqual(JSON.parse(JSON.stringify(state.values[0])), {
    bot_token: "",
    webhook_url: "https://assistant.example.com",
  });
  assert.deepEqual(JSON.parse(JSON.stringify(setQueryDataCalls)), [
    [["telegram-setup"], savedStatus],
  ]);
  assert.deepEqual(JSON.parse(JSON.stringify(invalidations)), [
    ["telegram-setup"],
    ["connectable-channels"],
    ["extensions"],
  ]);
  const afterSave = renderPanel(context, state, { data: savedStatus });
  assert.ok(!JSON.stringify(afterSave).includes("123456789:AAnew"), "saved token never re-renders");
});

test("TelegramSetupPanel renders the API error body when a save fails", () => {
  const state = { hookIndex: 0, mutationIndex: 0, values: {}, refs: {}, effectDeps: {} };
  const { context } = setupContext(state, {
    mutationOverrides: {
      0: { isError: true, error: { payload: { error: "invalid bot token" } } },
    },
  });
  const status = { configured: false, bot_username: null, bot_token_configured: false, webhook_url: null };

  renderPanel(context, state, { data: status });
  const rendered = renderPanel(context, state, { data: status });

  assert.ok(JSON.stringify(rendered).includes("invalid bot token"));
});

test("TelegramSetupPanel removes the bot only after the confirm prompt", () => {
  const declinedState = { hookIndex: 0, mutationIndex: 0, values: {}, refs: {}, effectDeps: {} };
  const declined = setupContext(declinedState, { confirmResult: false });
  const status = {
    configured: true,
    bot_username: "ironclaw_bot",
    bot_token_configured: true,
    webhook_url: null,
  };

  renderPanel(declined.context, declinedState, { data: status });
  let rendered = renderPanel(declined.context, declinedState, { data: status });
  // onClick order: [save, remove] once configured.
  valuesAfter(rendered, "onClick=")[1]();
  assert.deepEqual(declined.confirmCalls, ["telegramSetup.removeConfirm"]);
  assert.deepEqual(declined.clearCalls, []);

  const acceptedState = { hookIndex: 0, mutationIndex: 0, values: {}, refs: {}, effectDeps: {} };
  const accepted = setupContext(acceptedState, { confirmResult: true });
  renderPanel(accepted.context, acceptedState, { data: status });
  rendered = renderPanel(accepted.context, acceptedState, { data: status });
  valuesAfter(rendered, "onClick=")[1]();
  assert.deepEqual(accepted.clearCalls, ["delete"]);
  assert.deepEqual(JSON.parse(JSON.stringify(accepted.invalidations)), [
    ["telegram-setup"],
    ["connectable-channels"],
    ["extensions"],
  ]);
});

test("TelegramSetupPanel keeps dirty fields during a background setup refetch", () => {
  const state = { hookIndex: 0, mutationIndex: 0, values: {}, refs: {}, effectDeps: {} };
  const { context } = setupContext(state);
  const initialStatus = {
    configured: true,
    bot_username: "ironclaw_bot",
    bot_token_configured: true,
    webhook_url: "https://old.example.com",
  };

  renderPanel(context, state, { data: initialStatus });
  const rendered = renderPanel(context, state, { data: initialStatus });
  valuesAfter(rendered, "onChange=")[0]({ target: { value: "123456789:AAdirty" } });
  valuesAfter(rendered, "onChange=")[1]({ target: { value: "https://new.example.com" } });

  renderPanel(context, state, { data: { ...initialStatus, revision: 9 } });

  assert.equal(state.values[0].bot_token, "123456789:AAdirty");
  assert.equal(state.values[0].webhook_url, "https://new.example.com");
});

test("TelegramSetupPanel adopts a newer setup status while the form is pristine", () => {
  const state = { hookIndex: 0, mutationIndex: 0, values: {}, refs: {}, effectDeps: {} };
  const { context } = setupContext(state);
  const initialStatus = {
    configured: true,
    bot_username: "ironclaw_bot",
    bot_token_configured: true,
    webhook_url: "https://old.example.com",
    revision: 4,
  };

  renderPanel(context, state, { data: initialStatus });
  renderPanel(context, state, { data: initialStatus });
  assert.equal(state.values[0].webhook_url, "https://old.example.com");

  const newerStatus = {
    ...initialStatus,
    webhook_url: "https://new.example.com",
    revision: 5,
  };
  renderPanel(context, state, { data: newerStatus });

  assert.equal(
    state.values[0].webhook_url,
    "https://new.example.com",
    "a clean form tracks the latest durable setup revision",
  );
  assert.equal(state.values[0].bot_token, "", "the write-only token remains blank");
});

test("TelegramSetupPanel clears the stale save-success note when a field is edited", () => {
  const state = { hookIndex: 0, mutationIndex: 0, values: {}, refs: {}, effectDeps: {} };
  const savedStatus = {
    configured: true,
    bot_username: "ironclaw_bot",
    bot_token_configured: true,
    webhook_url: null,
    revision: 2,
  };
  const { context } = setupContext(state, { saveResponses: [savedStatus] });
  const status = {
    configured: true,
    bot_username: "ironclaw_bot",
    bot_token_configured: true,
    webhook_url: null,
    revision: 1,
  };

  renderPanel(context, state, { data: status });
  let rendered = renderPanel(context, state, { data: status });
  valuesAfter(rendered, "onClick=")[0]();
  rendered = renderPanel(context, state, { data: status });
  assert.ok(
    JSON.stringify(rendered).includes("telegramSetup.saved"),
    "a successful save shows the success note",
  );

  // Editing a field invalidates the success claim: the shown values no longer
  // have backend evidence.
  valuesAfter(rendered, "onChange=")[0]({ currentTarget: { value: "123456789:AAedit" } });
  rendered = renderPanel(context, state, { data: status });
  assert.ok(
    !JSON.stringify(rendered).includes("telegramSetup.saved"),
    "editing after a save clears the stale success note",
  );
});

test("TelegramSetupPanel serializes save and removal mutations", () => {
  // Remove pending: the save handler must refuse to fire a concurrent PUT.
  const state = { hookIndex: 0, mutationIndex: 0, values: {}, refs: {}, effectDeps: {} };
  const { context, saveCalls } = setupContext(state, {
    mutationOverrides: { 1: { isPending: true } },
  });
  const status = {
    configured: true,
    bot_username: "ironclaw_bot",
    bot_token_configured: true,
    webhook_url: null,
  };
  renderPanel(context, state, { data: status });
  const rendered = renderPanel(context, state, { data: status });
  assert.equal(
    valuesAfter(rendered, "disabled=")[0],
    true,
    "save button disables while removal is pending",
  );
  valuesAfter(rendered, "onClick=")[0]();
  assert.deepEqual(saveCalls, [], "no PUT while the DELETE is in flight");

  // Save pending: the remove handler must refuse before even confirming.
  const saveState = { hookIndex: 0, mutationIndex: 0, values: {}, refs: {}, effectDeps: {} };
  const savePending = setupContext(saveState, {
    mutationOverrides: { 0: { isPending: true } },
  });
  renderPanel(savePending.context, saveState, { data: status });
  const renderedSavePending = renderPanel(savePending.context, saveState, { data: status });
  valuesAfter(renderedSavePending, "onClick=")[1]();
  assert.deepEqual(savePending.confirmCalls, [], "no confirm prompt while a save is in flight");
  assert.deepEqual(savePending.clearCalls, [], "no DELETE while the PUT is in flight");
});

test("TelegramSetupPanel defines field guidance for the bot token and webhook override", () => {
  const state = { hookIndex: 0, mutationIndex: 0, values: {}, refs: {}, effectDeps: {} };
  const { context } = setupContext(state);
  const help = context.globalThis.__testExports.FIELD_HELP;

  assert.equal(help.botToken.bodyKey, "telegramSetup.help.botToken");
  assert.equal(help.botToken.exampleKey, "telegramSetup.example.botToken");
  assert.equal(help.webhookUrl.bodyKey, "telegramSetup.help.webhookUrl");
  assert.equal(help.webhookUrl.exampleKey, "telegramSetup.example.webhookUrl");
});
