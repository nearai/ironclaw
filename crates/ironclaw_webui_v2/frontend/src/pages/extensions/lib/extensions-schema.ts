// Tabs are product-taxonomy views over surfaces. Runtime (wasm/mcp/...) is
// an implementation badge on cards, never a grouping axis.
export const EXTENSIONS_TABS = [
  { id: "registry", labelKey: "extensions.registry", icon: "plus" },
  { id: "channels", labelKey: "extensions.channels", icon: "send" },
  { id: "tools", labelKey: "extensions.tools", icon: "pulse" },
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

// Channel discovery is extension-surface data: an extension's `surfaces`
// carry a typed `channel` entry with direction (inbound/outbound), the
// caller's connection state, and the connect affordance. There is no separate
// connectable-channel registry.
export function channelSurface(item) {
  return extensionSurfaces(item).find((surface) => surface?.kind === "channel") || null;
}

export function channelConnection(item) {
  return channelSurface(item)?.connection || null;
}

export function isInboundProofCodeConnection(connection) {
  return connection?.strategy === "inbound_proof_code";
}

// A channel extension whose connect affordance is a browser OAuth relay:
// connecting happens through the configure modal's OAuth secret, never a
// paste-a-code pairing panel. Derived from the wire only — the surface
// connection strategy, or an oauth-kind setup secret.
export function connectsViaOauth(item, secrets = []) {
  if (channelConnection(item)?.strategy === "oauth") return true;
  return secrets.some((secret) => secret?.setup?.kind === "oauth");
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
