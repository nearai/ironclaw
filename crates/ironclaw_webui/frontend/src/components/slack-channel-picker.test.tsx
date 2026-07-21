// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

function slackChannelPickerSourceForTest() {
  const source = readFileSync(new URL("./slack-channel-picker.tsx", import.meta.url), "utf8");
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { SlackChannelPicker, subjectOptionsForChannel };`;
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
      const index = state.hookIndex++;
      const dep = deps?.[0];
      state.effectDeps = state.effectDeps || {};
      if (state.effectDeps[index] !== dep) {
        state.effectDeps[index] = dep;
        effect();
      }
    },
    useMemo: (factory) => factory(),
  };
}

function SelectMenu(props) {
  return (
    <select
      aria-label={props.ariaLabel}
      disabled={props.disabled}
      onChange={props.onChange}
      value={props.value}
    />
  );
}

function visit(node, fn) {
  if (Array.isArray(node)) {
    for (const item of node) visit(item, fn);
    return;
  }
  if (!node || typeof node !== "object") return;
  fn(node);
  if (Array.isArray(node.values)) {
    for (const value of node.values) visit(value, fn);
  }
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
  const values = [];
  visit(rendered, (node) => {
    if (!Array.isArray(node.strings) || !Array.isArray(node.values)) return;
    node.strings.forEach((part, index) => {
      if (part.includes(fragment)) values.push(node.values[index]);
    });
  });
  return values;
}

function channelRows(rendered) {
  const rows = [];
  visit(rendered, (node) => {
    if (node.strings?.some((part) => part.includes("min-h-10"))) rows.push(node);
  });
  return rows;
}

function functionValues(node) {
  const values = [];
  visit(node, (child) => {
    if (!Array.isArray(child.values)) return;
    for (const value of child.values) {
      if (typeof value === "function") values.push(value);
    }
  });
  return values;
}

test("SlackChannelPicker edits saved channels and blocks save after load failure", () => {
  const state = { hookIndex: 0, values: {} };
  const saveCalls = [];
  const invalidations = [];
  const query = {
    data: {
      team_id: "T0HOST",
      channels: [
        {
          channel_id: " C0OPS ",
          subject_user_id: "user:ops-team-agent",
          subject_display_name: "Ops",
        },
        { channel_id: "C0ENG", subject_user_id: "user:eng-team-agent" },
      ],
    },
    isLoading: false,
    isSuccess: true,
    isError: false,
  };
  const subjectsQuery = {
    data: {
      team_id: "T0HOST",
      subjects: [
        { subject_user_id: "user:eng-team-agent", display_name: "Eng" },
        { subject_user_id: "user:ops-team-agent", display_name: "Ops" },
      ],
    },
    isLoading: false,
    isSuccess: true,
    isError: false,
  };
  const context = {
    Button: "button",
    React: createReactStub(state),
    SelectMenu,
    globalThis: {},
    listSlackAllowedChannels: () => query.data,
    normalizeSlackChannelIds: (channelIds = []) =>
      Array.from(
        new Set(
          channelIds
            .map((channelId) => String(channelId || "").trim())
            .filter(Boolean),
        ),
      ).sort(),
    listSlackRoutableSubjects: () => subjectsQuery.data,
    saveSlackAllowedChannels: (channels) => {
      saveCalls.push(channels);
      return {
        channels,
      };
    },
    slackChannelPickerError: () => "error",
    useT: () => (key, params = {}) =>
      ({
        "channels.slackAccessTitle": "Slack team agents",
        "channels.slackAccessInstructions":
          "Map Slack channels to the team agents that should answer there.",
        "channels.slackAccessAdd": "Add",
        "channels.slackAccessLoading": "Loading Slack channels...",
        "channels.slackAccessEmpty": "No Slack channels allowed yet.",
        "channels.slackAccessAllow": `Remove ${params.channelId}`,
        "channels.slackAccessAutoSubject": "Auto-generated team subject",
        "channels.slackAccessNoSubjects": "No team agents available",
        "channels.slackAccessSave": "Save channels",
        "channels.slackAccessSaving": "Saving...",
        "channels.slackAccessSuccess": "Slack channels saved.",
        "channels.slackAccessError": "Slack channel update failed.",
      })[key] || key,
    useQuery: ({ queryKey }) =>
      queryKey[0] === "slack-routable-subjects" ? subjectsQuery : query,
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
  assert.deepEqual(JSON.parse(JSON.stringify(state.values[2])), [
    { channel_id: "C0ENG", subject_user_id: "user:eng-team-agent" },
    {
      channel_id: "C0OPS",
      subject_user_id: "user:ops-team-agent",
      subject_display_name: "Ops",
    },
  ]);
  assert.ok(valuesAfter(rendered, "ariaLabel=").includes("Auto-generated team subject (C0ENG)"));
  assert.ok(valuesAfter(rendered, "ariaLabel=").includes("Auto-generated team subject (C0OPS)"));

  valuesAfter(rendered, "onChange=")[0]({ target: { value: " C0NEW " } });
  rendered = renderPicker(context, state);
  valuesAfter(rendered, "onClick=")[0]();
  assert.deepEqual(JSON.parse(JSON.stringify(state.values[2])), [
    { channel_id: "C0ENG", subject_user_id: "user:eng-team-agent" },
    { channel_id: "C0NEW", subject_user_id: "" },
    {
      channel_id: "C0OPS",
      subject_user_id: "user:ops-team-agent",
      subject_display_name: "Ops",
    },
  ]);

  rendered = renderPicker(context, state);
  {
    const rowFunctions = functionValues(channelRows(rendered)[0]);
    rowFunctions[rowFunctions.length - 1]();
  }
  assert.deepEqual(JSON.parse(JSON.stringify(state.values[2])), [
    { channel_id: "C0NEW", subject_user_id: "" },
    {
      channel_id: "C0OPS",
      subject_user_id: "user:ops-team-agent",
      subject_display_name: "Ops",
    },
  ]);

  rendered = renderPicker(context, state);
  valuesAfter(rendered, "onClick=").at(-1)();
  assert.deepEqual(JSON.parse(JSON.stringify(saveCalls)), [
    [
      { channel_id: "C0NEW", subject_user_id: "" },
      { channel_id: "C0OPS", subject_user_id: "user:ops-team-agent" },
    ],
  ]);
  assert.deepEqual(JSON.parse(JSON.stringify(invalidations)), [
    ["slack-allowed-channels"],
    ["slack-routable-subjects"],
    ["extensions"],
    ["connectable-channels"],
  ]);

  query.isSuccess = false;
  query.isError = true;
  rendered = renderPicker(context, state);
  assert.equal(valuesAfter(rendered, "disabled=").at(-1), true);
});

test("subjectOptionsForChannel keeps current route subjects row-scoped with friendly labels", () => {
  const context = {
    globalThis: {},
  };
  vm.runInNewContext(slackChannelPickerSourceForTest(), context);

  const subjects = [
    { subject_user_id: "user:eng-team-agent", display_name: "Eng" },
  ];
  const rawRowOptions = context.globalThis.__testExports.subjectOptionsForChannel(subjects, {
    channel_id: "C0RAW",
    subject_user_id: "user:raw-route-subject",
    subject_display_name: "Raw Route Subject",
  });
  const otherRowOptions = context.globalThis.__testExports.subjectOptionsForChannel(subjects, {
    channel_id: "C0ENG",
    subject_user_id: "user:eng-team-agent",
  });

  assert.deepEqual(
    JSON.parse(JSON.stringify(rawRowOptions.map((subject) => subject.subject_user_id))),
    ["user:eng-team-agent", "user:raw-route-subject"],
  );
  assert.deepEqual(
    JSON.parse(JSON.stringify(rawRowOptions.map((subject) => subject.display_name))),
    ["Eng", "Raw Route Subject"],
  );
  assert.deepEqual(
    JSON.parse(JSON.stringify(otherRowOptions.map((subject) => subject.subject_user_id))),
    ["user:eng-team-agent"],
  );
});

test("SlackChannelPicker preserves row subjects when subject catalog fails", () => {
  const state = { hookIndex: 0, values: {} };
  const saveCalls = [];
  const query = {
    data: {
      team_id: "T0HOST",
      channels: [
        {
          channel_id: "C0RAW",
          subject_user_id: "user:raw-route-subject",
          subject_display_name: "Raw Route Subject",
        },
      ],
    },
    isLoading: false,
    isSuccess: true,
    isError: false,
  };
  const subjectsQuery = {
    data: undefined,
    isLoading: false,
    isSuccess: false,
    isError: true,
    error: new Error("subject catalog unavailable"),
  };
  const context = {
    Button: "button",
    React: createReactStub(state),
    SelectMenu,
    globalThis: {},
    listSlackAllowedChannels: () => query.data,
    normalizeSlackChannelIds: (channelIds = []) =>
      Array.from(
        new Set(
          channelIds
            .map((channelId) => String(channelId || "").trim())
            .filter(Boolean),
        ),
      ).sort(),
    listSlackRoutableSubjects: () => subjectsQuery.data,
    saveSlackAllowedChannels: (channels) => {
      saveCalls.push(channels);
      return { channels: [{ channel_id: "C0RAW", subject_user_id: "" }] };
    },
    slackChannelPickerError: (error) => error.message,
    useT: () => (key, params = {}) =>
      ({
        "channels.slackAccessTitle": "Slack team agents",
        "channels.slackAccessInstructions":
          "Map Slack channels to the team agents that should answer there.",
        "channels.slackAccessAdd": "Add",
        "channels.slackAccessLoading": "Loading Slack channels...",
        "channels.slackAccessEmpty": "No Slack channels allowed yet.",
        "channels.slackAccessAllow": `Remove ${params.channelId}`,
        "channels.slackAccessAutoSubject": "Auto-generated team subject",
        "channels.slackAccessNoSubjects": "No team agents available",
        "channels.slackAccessSave": "Save channels",
        "channels.slackAccessSaving": "Saving...",
        "channels.slackAccessSuccess": "Slack channels saved.",
        "channels.slackAccessError": "Slack channel update failed.",
      })[key] || key,
    useQuery: ({ queryKey }) =>
      queryKey[0] === "slack-routable-subjects" ? subjectsQuery : query,
    useQueryClient: () => ({
      invalidateQueries: () => {},
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
  assert.equal(valuesAfter(rendered, "disabled=").at(-1), false);

  valuesAfter(rendered, "onChange=")[0]({ target: { value: " C0NEW " } });
  rendered = renderPicker(context, state);
  assert.equal(valuesAfter(rendered, "disabled=")[1], true);
  valuesAfter(rendered, "onClick=")[0]();
  assert.deepEqual(JSON.parse(JSON.stringify(state.values[2])), [
    {
      channel_id: "C0RAW",
      subject_user_id: "user:raw-route-subject",
      subject_display_name: "Raw Route Subject",
    },
  ]);

  valuesAfter(rendered, "onClick=").at(-1)();

  assert.deepEqual(JSON.parse(JSON.stringify(saveCalls)), [
    [{ channel_id: "C0RAW", subject_user_id: "user:raw-route-subject" }],
  ]);
});
