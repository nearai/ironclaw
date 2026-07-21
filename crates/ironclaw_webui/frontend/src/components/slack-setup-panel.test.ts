// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

function slackSetupPanelSourceForTest() {
  const source = readFileSync(new URL("./slack-setup-panel.tsx", import.meta.url), "utf8");
  const lines = [];
  for (const line of source.split("\n")) {
    if (line.startsWith("import ")) continue;
    lines.push(
      line
        .replace("export function SlackAdminManagedSection", "function SlackAdminManagedSection")
        .replace("export function SlackSetupPanel", "function SlackSetupPanel"),
    );
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { SlackSetupPanel, FIELD_HELP, FieldHint, slackSetupCopy };`;
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

function html(strings, ...values) {
  return { strings: Array.from(strings), values };
}

function renderPanel(context, state, setupQuery) {
  state.hookIndex = 0;
  return context.globalThis.__testExports.SlackSetupPanel({
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

function setupContext(state, saveResponses = []) {
  const invalidations = [];
  const setQueryDataCalls = [];
  const context = {
    Button: "button",
    Icon() {},
    React: createReactStub(state),
    SlackChannelPicker() {},
    globalThis: {},
    html,
    getSlackSetup: () => ({}),
    saveSlackSetup: () => saveResponses.shift(),
    slackSetupError: () => "error",
    useQuery: () => ({}),
    useQueryClient: () => ({
      setQueryData: (...args) => setQueryDataCalls.push(args),
      invalidateQueries: (query) => invalidations.push(query.queryKey),
    }),
    useMutation: (config) => ({
      isPending: false,
      isSuccess: false,
      isError: false,
      mutate: (variables) => {
        const data = config.mutationFn(variables);
        config.onSuccess(data, variables);
      },
    }),
    useT: () => (key) => key,
  };
  vm.runInNewContext(slackSetupPanelSourceForTest(), context);
  return { context, invalidations, setQueryDataCalls };
}

test("SlackSetupPanel does not reset dirty form fields on background setup refetch", () => {
  const state = { hookIndex: 0, values: {}, refs: {}, effectDeps: {} };
  const { context } = setupContext(state);
  const initialStatus = {
    configured: true,
    installation_id: "install_saved",
    team_id: "T0SAVED",
    api_app_id: "A0SAVED",
    user_id: "user:saved",
    shared_subject_user_id: "user:shared",
    bot_token_configured: true,
    signing_secret_configured: true,
  };

  renderPanel(context, state, { data: initialStatus });
  const rendered = renderPanel(context, state, { data: initialStatus });

  valuesAfter(rendered, "onChange=")[0]({ target: { value: "install_dirty" } });
  valuesAfter(rendered, "onChange=")[5]({ target: { value: "xoxb-dirty" } });
  assert.equal(state.values[0].installation_id, "install_dirty");
  assert.equal(state.values[0].bot_token, "xoxb-dirty");

  renderPanel(context, state, {
    data: {
      ...initialStatus,
      revision: 2,
    },
  });

  assert.equal(state.values[0].installation_id, "install_dirty");
  assert.equal(state.values[0].bot_token, "xoxb-dirty");
});

test("FieldHint falls back to literal help copy when no translator is supplied", () => {
  const state = { hookIndex: 0, values: {}, refs: {}, effectDeps: {} };
  const { context } = setupContext(state);
  const { FieldHint } = context.globalThis.__testExports;

  const rendered = FieldHint({
    help: {
      bodyKey: "slackSetup.help.body",
      body: "Fallback body",
      exampleKey: "slackSetup.help.example",
      example: "Fallback example",
    },
    t: null,
  });

  const body = JSON.stringify(rendered);
  assert.match(body, /Fallback body/);
  assert.match(body, /Fallback example/);
});

test("SlackSetupPanel does not overwrite user input when initial setup load resolves", () => {
  const state = { hookIndex: 0, values: {}, refs: {}, effectDeps: {} };
  const { context } = setupContext(state);
  const loadedStatus = {
    configured: true,
    installation_id: "install_loaded",
    team_id: "T0LOADED",
    api_app_id: "A0LOADED",
    user_id: "user:loaded",
    shared_subject_user_id: "user:shared-loaded",
    bot_token_configured: true,
    signing_secret_configured: true,
  };

  let rendered = renderPanel(context, state, { isLoading: true });
  valuesAfter(rendered, "onChange=")[0]({ target: { value: "install_typing" } });
  valuesAfter(rendered, "onChange=")[1]({ target: { value: "T0TYPING" } });

  renderPanel(context, state, { data: loadedStatus, isLoading: false });

  assert.equal(state.values[0].installation_id, "install_typing");
  assert.equal(state.values[0].team_id, "T0TYPING");
});

test("SlackSetupPanel clears secrets and accepts saved status after successful save", () => {
  const state = { hookIndex: 0, values: {}, refs: {}, effectDeps: {} };
  const savedStatus = {
    configured: true,
    installation_id: "install_saved_after_submit",
    team_id: "T0SAVED2",
    api_app_id: "A0SAVED2",
    user_id: "user:saved-after-submit",
    shared_subject_user_id: null,
    bot_token_configured: true,
    signing_secret_configured: true,
    revision: 3,
  };
  const { context, invalidations, setQueryDataCalls } = setupContext(state, [savedStatus]);
  const initialStatus = {
    configured: false,
    installation_id: "install_saved",
    team_id: "T0SAVED",
    api_app_id: "A0SAVED",
    user_id: "",
    shared_subject_user_id: "",
    bot_token_configured: false,
    signing_secret_configured: false,
  };

  renderPanel(context, state, { data: initialStatus });
  let rendered = renderPanel(context, state, { data: initialStatus });
  valuesAfter(rendered, "onChange=")[5]({ target: { value: "xoxb-new" } });
  valuesAfter(rendered, "onChange=")[6]({ target: { value: "signing-new" } });
  rendered = renderPanel(context, state, { data: initialStatus });

  valuesAfter(rendered, "onClick=")[0]();

  assert.deepEqual(JSON.parse(JSON.stringify(state.values[0])), {
    installation_id: "install_saved_after_submit",
    team_id: "T0SAVED2",
    api_app_id: "A0SAVED2",
    user_id: "user:saved-after-submit",
    shared_subject_user_id: "",
    bot_token: "",
    signing_secret: "",
    oauth_client_id: "",
    oauth_client_secret: "",
  });
  assert.deepEqual(JSON.parse(JSON.stringify(setQueryDataCalls)), [
    [["slack-setup"], savedStatus],
  ]);
  assert.deepEqual(JSON.parse(JSON.stringify(invalidations)), [
    ["slack-setup"],
    ["slack-allowed-channels"],
    ["slack-routable-subjects"],
    ["connectable-channels"],
    ["extensions"],
  ]);
});

test("SlackSetupPanel rejects whitespace-only fresh secrets", () => {
  const state = { hookIndex: 0, values: {}, refs: {}, effectDeps: {} };
  const { context } = setupContext(state);
  const status = {
    configured: false,
    installation_id: "install_saved",
    team_id: "T0SAVED",
    api_app_id: "A0SAVED",
    user_id: "",
    shared_subject_user_id: "",
    bot_token_configured: false,
    signing_secret_configured: false,
  };

  renderPanel(context, state, { data: status });
  let rendered = renderPanel(context, state, { data: status });
  valuesAfter(rendered, "onChange=")[5]({ target: { value: "   " } });
  valuesAfter(rendered, "onChange=")[6]({ target: { value: "   " } });
  rendered = renderPanel(context, state, { data: status });

  assert.equal(valuesAfter(rendered, "disabled=")[0], true);
});

test("SlackSetupPanel defines field guidance for Slack credentials", () => {
  const state = { hookIndex: 0, values: {}, refs: {}, effectDeps: {} };
  const { context } = setupContext(state);
  const help = context.globalThis.__testExports.FIELD_HELP;

  assert.equal(help.installationId.bodyKey, "slackSetup.help.installationId");
  assert.equal(help.installationId.exampleKey, "slackSetup.example.localSlack");
  assert.equal(help.teamId.bodyKey, "slackSetup.help.teamId");
  assert.equal(help.teamId.exampleKey, "slackSetup.example.teamId");
  assert.equal(help.appId.bodyKey, "slackSetup.help.appId");
  assert.equal(help.appId.exampleKey, "slackSetup.example.appId");
  assert.equal(help.botToken.bodyKey, "slackSetup.help.botToken");
  assert.equal(help.signingSecret.bodyKey, "slackSetup.help.signingSecret");
  assert.match(help.oauthClientId.body, /Client ID/);
  assert.match(help.oauthClientSecret.body, /Client Secret/);
});
