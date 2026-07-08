export const EXTENSIONS_TABS = [
  { id: "registry", labelKey: "extensions.registry", icon: "plus" },
  { id: "channels", labelKey: "extensions.channels", icon: "send" },
  { id: "mcp", labelKey: "extensions.mcp", icon: "pulse" },
];

// Runtime implementation labels — the wire's `runtime` field is an honest
// implementation name; product taxonomy travels in `surfaces`.
export const RUNTIME_LABELS = {
  wasm: "WASM",
  mcp: "MCP",
  first_party: "First-party",
  system: "System",
  script: "Script",
};

// Product taxonomy is surface-based: an extension is a channel view when its
// surfaces declare a channel; a tools view when they declare tools.
export function extensionSurfaces(item) {
  return item?.surfaces || [];
}

export function hasChannelSurface(item) {
  return extensionSurfaces(item).some((surface) => surface?.kind === "channel");
}

export function hasToolSurface(item) {
  return extensionSurfaces(item).some((surface) => surface?.kind === "tool");
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
