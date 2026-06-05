export const EXTENSIONS_TABS = [
  { id: "installed", label: "Installed", icon: "bolt" },
  { id: "channels", label: "Channels", icon: "send" },
  { id: "mcp", label: "MCP Servers", icon: "pulse" },
  { id: "registry", label: "Registry", icon: "plus" },
];

export const KIND_LABELS = {
  wasm_tool: "WASM Tool",
  wasm_channel: "Channel",
  mcp_server: "MCP Server",
  first_party: "First-party",
  system: "System",
  channel_relay: "Relay",
};

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
