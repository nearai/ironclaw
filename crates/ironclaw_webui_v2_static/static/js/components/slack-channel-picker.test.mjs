import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function slackChannelPickerSourceForTest() {
  const source = readFileSync(new URL("./slack-channel-picker.js", import.meta.url), "utf8");
  const lines = [];
  let skippingImport = false;
  for (const line of source.split("\n")) {
    if (skippingImport) {
      if (line.includes(";")) {
        skippingImport = false;
      }
      continue;
    }
    if (line.startsWith("import ")) {
      if (!line.includes(";")) {
        skippingImport = true;
      }
      continue;
    }
    lines.push(line.replace("export function SlackChannelPicker", "function SlackChannelPicker"));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { SlackChannelPicker };`;
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
    useEffect: (effect, deps) => {
      const dep = deps?.[0];
      if (state.lastEffectDep !== dep) {
        state.lastEffectDep = dep;
        effect();
      }
    },
  };
}

function html(strings, ...values) {
  return { strings: Array.from(strings), values };
}

function renderPicker(context, state, props = {}) {
  state.hookIndex = 0;
  return context.globalThis.__testExports.SlackChannelPicker(props);
}

function valueAfter(rendered, fragment) {
  const index = rendered.strings.findIndex((part) => part.includes(fragment));
  assert.notEqual(index, -1, `expected template fragment ${fragment}`);
  return rendered.values[index];
}

function valuesAfter(rendered, fragment) {
  return rendered.strings.flatMap((part, index) =>
    part.includes(fragment) ? [rendered.values[index]] : [],
  );
}

function channelRows(rendered) {
  return rendered.values.find(
    (value) =>
      Array.isArray(value) &&
      value.every((row) => row?.strings?.some((part) => part.includes("key="))),
  );
}

test("SlackChannelPicker edits saved channels and blocks save after load failure", () => {
  const state = { hookIndex: 0, values: {} };
  const saveCalls = [];
  const invalidations = [];
  const query = {
    data: {
      team_id: "T0HOST",
      channels: [{ channel_id: " C0OPS " }, { channel_id: "C0ENG" }],
    },
    isLoading: false,
    isSuccess: true,
    isError: false,
  };
  const context = {
    Button: "button",
    React: createReactStub(state),
    globalThis: {},
    html,
    listSlackAllowedChannels: () => query.data,
    normalizeSlackChannelIds: (channelIds = []) =>
      Array.from(
        new Set(
          channelIds
            .map((channelId) => String(channelId || "").trim())
            .filter(Boolean),
        ),
      ).sort(),
    saveSlackAllowedChannels: (channelIds) => {
      saveCalls.push(channelIds);
      return {
        channels: channelIds.map((channel_id) => ({ channel_id })),
      };
    },
    slackChannelPickerError: () => "error",
    useT: () => (key, params = {}) =>
      ({
        "channels.slackAccessTitle": "Slack channel access",
        "channels.slackAccessInstructions":
          "Choose the Slack channels this tenant app may answer in.",
        "channels.slackAccessAdd": "Add",
        "channels.slackAccessLoading": "Loading Slack channels...",
        "channels.slackAccessEmpty": "No Slack channels allowed yet.",
        "channels.slackAccessAllow": `Allow ${params.channelId}`,
        "channels.slackAccessSave": "Save channels",
        "channels.slackAccessSaving": "Saving...",
        "channels.slackAccessSuccess": "Slack channels saved.",
        "channels.slackAccessError": "Slack channel update failed.",
      })[key] || key,
    useQuery: () => query,
    useQueryClient: () => ({
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
  };
  vm.runInNewContext(slackChannelPickerSourceForTest(), context);

  renderPicker(context, state);
  let rendered = renderPicker(context, state);
  assert.deepEqual(state.values[1], ["C0ENG", "C0OPS"]);

  rendered.values[4]({ target: { value: " C0NEW " } });
  rendered = renderPicker(context, state);
  rendered.values[8]();
  assert.deepEqual(state.values[1], ["C0ENG", "C0NEW", "C0OPS"]);

  rendered = renderPicker(context, state);
  channelRows(rendered)[0].values[4]();
  assert.deepEqual(state.values[1], ["C0NEW", "C0OPS"]);

  rendered = renderPicker(context, state);
  valuesAfter(rendered, "onClick=").at(-1)();
  assert.deepEqual(saveCalls, [["C0NEW", "C0OPS"]]);
  assert.deepEqual(JSON.parse(JSON.stringify(invalidations)), [
    ["slack-allowed-channels"],
    ["extensions"],
    ["connectable-channels"],
  ]);

  query.isSuccess = false;
  query.isError = true;
  rendered = renderPicker(context, state);
  assert.equal(valuesAfter(rendered, "disabled=").at(-1), true);
});
