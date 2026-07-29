// Settings → Tools fixtures: the `/settings/tools` operator-config export
// (auto-approve flag + `tool.*` permission entries) and the generic
// `/operator/config/:key` store.
//
// The permission-save response is validated strictly by
// `persistedToolFromConfigEntry` in settings-api.ts: `value.state` and
// `value.default_state` must be concrete states (never "default"),
// `entry.source` must equal `value.effective_source`, and the entry must
// confirm the requested state — the shapes below honor that contract.

type ConcreteToolState = "always_allow" | "ask_each_time" | "disabled";
type EffectiveSource = "default" | "global" | "override" | "locked";

type ToolRecord = {
  name: string;
  description: string;
  state: ConcreteToolState;
  defaultState: ConcreteToolState;
  locked: boolean;
  effectiveSource: EffectiveSource;
};

// Names deliberately match the `tools.description.*` i18n catalog so the tab
// shows localized descriptions where they exist.
const tools: ToolRecord[] = [
  {
    name: "builtin.shell",
    description: "Execute shell commands with validation",
    state: "ask_each_time",
    defaultState: "ask_each_time",
    locked: false,
    effectiveSource: "default",
  },
  {
    name: "builtin.http",
    description: "Perform an outbound HTTP request through host egress",
    state: "always_allow",
    defaultState: "ask_each_time",
    locked: false,
    effectiveSource: "override",
  },
  {
    name: "builtin.read_file",
    description: "Read text files through scoped mounts",
    state: "always_allow",
    defaultState: "always_allow",
    locked: false,
    effectiveSource: "default",
  },
  {
    name: "builtin.write_file",
    description: "Write content through scoped mounts",
    state: "ask_each_time",
    defaultState: "ask_each_time",
    locked: false,
    effectiveSource: "global",
  },
  {
    name: "builtin.apply_patch",
    description: "Apply search-replace edits through scoped mounts",
    state: "always_allow",
    defaultState: "ask_each_time",
    locked: false,
    effectiveSource: "override",
  },
  {
    name: "builtin.memory_search",
    description: "Search persistent memory documents in the current scope",
    state: "always_allow",
    defaultState: "always_allow",
    locked: false,
    effectiveSource: "default",
  },
  {
    name: "builtin.memory_write",
    description: "Write persistent memory documents in the current scope",
    state: "ask_each_time",
    defaultState: "ask_each_time",
    locked: false,
    effectiveSource: "default",
  },
  {
    name: "builtin.spawn_subagent",
    description: "Authorize a scoped child subagent run",
    state: "disabled",
    defaultState: "ask_each_time",
    locked: true,
    effectiveSource: "locked",
  },
  {
    name: "builtin.trigger_create",
    description: "Create a caller-scoped scheduled trigger",
    state: "ask_each_time",
    defaultState: "ask_each_time",
    locked: false,
    effectiveSource: "default",
  },
  {
    name: "builtin.extension_install",
    description: "Install a searched extension into durable lifecycle state",
    state: "disabled",
    defaultState: "ask_each_time",
    locked: false,
    effectiveSource: "override",
  },
  {
    name: "nearai.web_search",
    description: "Search the web through the NEAR AI MCP server",
    state: "always_allow",
    defaultState: "always_allow",
    locked: false,
    effectiveSource: "global",
  },
];

let autoApproveTools = true;

function toolEntry(tool: ToolRecord) {
  return {
    key: `tool.${tool.name}`,
    value: {
      name: tool.name,
      description: tool.description,
      state: tool.state,
      default_state: tool.defaultState,
      locked: tool.locked,
      effective_source: tool.effectiveSource,
    },
    source: tool.effectiveSource,
    mutable: !tool.locked,
  };
}

function autoApproveEntry() {
  return {
    key: "agent.auto_approve_tools",
    value: autoApproveTools,
    source: "global",
    mutable: true,
  };
}

export function settingsToolsExport() {
  return {
    entries: [autoApproveEntry(), ...tools.map(toolEntry)],
    diagnostics: [],
    precedence: ["default", "global", "override"],
  };
}

export function setAutoApproveTools(enabled: boolean) {
  autoApproveTools = enabled;
  return autoApproveEntry();
}

/** Apply a permission change and return the persisted entry (or null). */
export function updateToolPermission(name: string, requestedState: string) {
  const tool = tools.find((entry) => entry.name === name);
  if (!tool) return null;
  if (requestedState === "default") {
    tool.state = tool.defaultState;
    tool.effectiveSource = "default";
  } else if (
    requestedState === "always_allow" ||
    requestedState === "ask_each_time" ||
    requestedState === "disabled"
  ) {
    tool.state = requestedState;
    tool.effectiveSource = "override";
  } else {
    return null;
  }
  tool.locked = false;
  return toolEntry(tool);
}

/* ── Generic operator config (`/operator/config/:key`) ─────────────── */

const operatorConfig = new Map<string, unknown>([
  ["agent.name", "IronClaw (staging tour)"],
  ["agent.max_parallel_jobs", 4],
  ["agent.job_timeout_secs", 1800],
  ["agent.max_tool_iterations", 40],
  ["agent.use_planning", true],
  ["agent.default_timezone", "America/Los_Angeles"],
  ["heartbeat.enabled", true],
  ["heartbeat.interval_secs", 900],
  ["sandbox.enabled", true],
  ["sandbox.policy", "workspace_write"],
]);

export function operatorConfigEntry(key: string) {
  if (!operatorConfig.has(key)) return null;
  return {
    key,
    value: operatorConfig.get(key),
    source: "override",
    mutable: true,
  };
}

export function setOperatorConfig(key: string, value: unknown) {
  operatorConfig.set(key, value);
  return operatorConfigEntry(key);
}
