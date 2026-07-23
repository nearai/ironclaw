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

export function hasAuthSurface(item) {
  return extensionSurfaces(item).some((surface) => surface?.kind === "auth");
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

export function isWebGeneratedCodeConnection(connection) {
  return connection?.strategy === "web_generated_code";
}

// Caller-visible lifecycle has only two listed states. Absence from the
// installed list is `uninstalled`; internal install/discovery/publication
// checkpoints never become extra card states.
export const STATE_TONES = {
  setup_needed: "warning",
  active: "success",
};

export const STATE_LABELS = {
  setup_needed: "setup needed",
  active: "active",
};

// The primary vendor account on the extensions wire
// (auth_accounts[0].accounts[0]; §6.4 / ADR 0001 — list length ≤ 1 today). It
// carries the shared §6.3 auth-account `state` and typed `last_error` the
// connect affordance and expiry notice key off.
export function primaryAuthAccount(item) {
  const vendor = (item?.auth_accounts || [])[0];
  return (vendor?.accounts || [])[0] || null;
}

// Whether the caller's account needs re-authentication rather than a
// first-time connect (§6.3 `AuthAccountState`): `expired` (token/refresh
// lapsed), or `disconnected` while carrying a typed `last_error` — a live
// grant was revoked, a stored credential went missing, or a prior auth
// attempt failed/expired before completing. A `disconnected` account with NO
// `last_error` is a fresh, never-connected extension and stays a plain
// Connect. Drives the distinct "Reconnect (expired)" affordance and the
// expiry/failure notice. There is no `revoking` state on the wire: disconnect
// and removal delete the account synchronously (overview §6.3), so no
// in-progress revoking window is ever produced or observed here.
export function authAccountNeedsReconnect(item) {
  const account = primaryAuthAccount(item);
  if (!account) return false;
  if (account.state === "expired") return true;
  return account.state === "disconnected" && Boolean(account.last_error);
}

// Typed last-transition reason (§6.3 `AuthAccountLastError`) mapped to a
// distinct i18n key, so the card explains WHY re-authentication is needed
// instead of one generic "expired" notice. Falls back to the generic expiry
// copy for an account that needs reconnecting with no typed reason attached.
const AUTH_ACCOUNT_REASON_LABELS = {
  flow_expired: "extensions.accountFlowExpired",
  vendor_denied: "extensions.accountVendorDenied",
  exchange_failed: "extensions.accountExchangeFailed",
  refresh_failed: "extensions.accountExpired",
  grant_revoked: "extensions.accountRevoked",
  validation_probe_failed: "extensions.accountValidationFailed",
  credential_missing: "extensions.accountCredentialMissing",
};

export function authAccountReasonLabelKey(account) {
  return AUTH_ACCOUNT_REASON_LABELS[account?.last_error] || "extensions.accountExpired";
}
