export const EXTENSIONS_TABS = [
  { id: "registry", label: "Registry", icon: "plus" },
  { id: "channels", label: "Channels", icon: "send" },
  { id: "mcp", label: "MCP Servers", icon: "pulse" },
];

export const KIND_LABELS = {
  wasm_tool: "WASM Tool",
  wasm_channel: "Channel",
  channel: "Channel",
  mcp_server: "MCP Server",
  first_party: "First-party",
  system: "System",
  channel_relay: "Relay",
};

export function isChannelExtensionKind(kind) {
  return kind === "wasm_channel" || kind === "channel";
}

export const STATE_TONES = {
  active: "success",
  ready: "success",
  pairing_required: "warning",
  pairing: "warning",
  auth_required: "warning",
  setup_required: "muted",
  failed: "danger",
  installed: "muted",
};

export const STATE_LABELS = {
  active: "active",
  ready: "ready",
  pairing_required: "pairing",
  pairing: "pairing",
  auth_required: "auth needed",
  setup_required: "setup needed",
  failed: "failed",
  installed: "installed",
};
