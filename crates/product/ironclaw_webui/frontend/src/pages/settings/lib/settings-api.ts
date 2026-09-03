import { apiFetch, type ApiRecord } from "../../../lib/api";

const OPERATOR_CONFIG_BASE = "/api/webchat/v2/operator/config";
const SETTINGS_TOOLS_BASE = "/api/webchat/v2/settings/tools";
const AUTO_APPROVE_KEY = "agent.auto_approve_tools";
const TOOL_PREFIX = "tool.";
const TOOL_PERMISSION_UPDATE_TIMEOUT_MS = 30_000;
const TOOL_PERMISSION_STATES = new Set(["always_allow", "ask_each_time", "disabled"]);
const TOOL_PERMISSION_UPDATE_STATES = new Set([
  "default",
  "always_allow",
  "ask_each_time",
  "disabled",
]);

function recordField(
  response: ApiRecord,
  field: string,
  responseName: string,
): ApiRecord {
  const value = response[field];
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new TypeError(`invalid ${responseName} response`);
  }
  return value as ApiRecord;
}

function recordArrayField(
  response: ApiRecord,
  field: string,
  responseName: string,
): ApiRecord[] {
  const value = response[field];
  if (
    !Array.isArray(value) ||
    !value.every(
      (entry) => typeof entry === "object" && entry !== null && !Array.isArray(entry),
    )
  ) {
    throw new TypeError(`invalid ${responseName} response`);
  }
  return value as ApiRecord[];
}

function unknownArrayField(
  response: ApiRecord,
  field: string,
  responseName: string,
): unknown[] {
  const value = response[field];
  if (!Array.isArray(value)) {
    throw new TypeError(`invalid ${responseName} response`);
  }
  return value;
}

function optionalUnknownArrayField(
  response: ApiRecord,
  field: string,
  responseName: string,
): unknown[] {
  return response[field] === undefined
    ? []
    : unknownArrayField(response, field, responseName);
}

function normalizeToolState(state) {
  if (state === "ask") return "ask_each_time";
  return TOOL_PERMISSION_STATES.has(state) ? state : "ask_each_time";
}

function normalizeToolUpdateState(state) {
  if (state === "ask") return "ask_each_time";
  return TOOL_PERMISSION_UPDATE_STATES.has(state) ? state : "default";
}

function normalizeEffectiveSource(source) {
  return ["default", "global", "override"].includes(source) ? source : "default";
}

function persistedToolFromConfigEntry(entry, expectedName, requestedState) {
  const value = entry?.value;
  const hasPersistedShape =
    entry?.key === `${TOOL_PREFIX}${expectedName}` &&
    value != null &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    value.name === expectedName &&
    TOOL_PERMISSION_STATES.has(value.state) &&
    TOOL_PERMISSION_STATES.has(value.default_state) &&
    typeof value.locked === "boolean" &&
    ["default", "global", "override", "locked"].includes(value.effective_source) &&
    entry.source === value.effective_source &&
    typeof entry.mutable === "boolean";
  if (!hasPersistedShape) {
    throw new Error("Permission save response is missing a valid persisted tool entry");
  }

  const tool = toolFromConfigEntry(entry);
  const confirmsRequestedState =
    requestedState === "default"
      ? value.effective_source !== "override"
      : tool?.state === requestedState;
  if (!tool || !confirmsRequestedState) {
    throw new Error("Permission save response did not confirm the requested tool state");
  }
  return tool;
}

export function toolFromConfigEntry(entry) {
  if (!entry?.key?.startsWith(TOOL_PREFIX)) return null;
  const value = entry.value || {};
  const name = value.name || entry.key.slice(TOOL_PREFIX.length);
  return {
    name,
    description: value.description || "",
    state: normalizeToolState(value.state),
    default_state: normalizeToolState(value.default_state),
    locked: Boolean(value.locked || entry.mutable === false),
    effective_source: normalizeEffectiveSource(value.effective_source || entry.source),
  };
}

export function settingsFromOperatorConfig(data) {
  const settings = {};
  for (const entry of data.entries || []) {
    if (entry?.key === AUTO_APPROVE_KEY) {
      settings[AUTO_APPROVE_KEY] = Boolean(entry.value);
    }
  }
  return settings;
}

export async function fetchSettingsExport() {
  const data = await apiFetch(SETTINGS_TOOLS_BASE);
  return {
    settings: settingsFromOperatorConfig(data),
    diagnostics: optionalUnknownArrayField(data, "diagnostics", "tool settings"),
    precedence: optionalUnknownArrayField(data, "precedence", "tool settings"),
  };
}
export async function fetchSetting(key) {
  if (key === AUTO_APPROVE_KEY) {
    const data = await fetchSettingsExport();
    // Default ON when unset, mirroring backend AUTO_APPROVE_DEFAULT_ENABLED.
    return data.settings[AUTO_APPROVE_KEY] ?? true;
  }
  const data = await apiFetch(`${OPERATOR_CONFIG_BASE}/${encodeURIComponent(key)}`);
  return recordField(data, "entry", "operator config").value ?? null;
}

type SettingEntry = {
  key: string;
  value: unknown;
  mutable?: boolean;
  source?: string;
};

export type SettingUpdateSuccess = {
  success: true;
  entry: SettingEntry;
  value: unknown;
};

export type SettingUpdateFailure = {
  success: false;
  message?: string;
};

export type SettingUpdateResult = SettingUpdateSuccess | SettingUpdateFailure;

function settingUpdateResult(data: unknown, expectedKey: string): SettingUpdateResult {
  if (typeof data !== "object" || data === null || Array.isArray(data)) {
    throw new Error("Save response is not an object");
  }
  const success = Reflect.get(data, "success");
  if (success === false) {
    const message = Reflect.get(data, "message");
    return {
      success: false,
      ...(typeof message === "string" ? { message } : {}),
    };
  }
  if (success !== undefined && success !== true) {
    throw new Error("Save response has an invalid success flag");
  }

  const entry = Reflect.get(data, "entry");
  if (
    typeof entry !== "object" ||
    entry === null ||
    Array.isArray(entry) ||
    Reflect.get(entry, "key") !== expectedKey ||
    !Object.prototype.hasOwnProperty.call(entry, "value")
  ) {
    throw new Error("Save response is missing the confirmed setting entry");
  }

  const mutable = Reflect.get(entry, "mutable");
  const source = Reflect.get(entry, "source");
  const confirmedEntry: SettingEntry = {
    key: expectedKey,
    value: Reflect.get(entry, "value"),
    ...(typeof mutable === "boolean" ? { mutable } : {}),
    ...(typeof source === "string" ? { source } : {}),
  };
  return { success: true, entry: confirmedEntry, value: confirmedEntry.value };
}

export async function updateSetting(
  key: string,
  value: unknown,
): Promise<SettingUpdateResult> {
  if (key === AUTO_APPROVE_KEY) {
    const data = await apiFetch(SETTINGS_TOOLS_BASE, {
      method: "POST",
      body: JSON.stringify({ enabled: Boolean(value) }),
    });
    return settingUpdateResult(data, key);
  }
  const data = await apiFetch(`${OPERATOR_CONFIG_BASE}/${encodeURIComponent(key)}`, {
    method: "POST",
    body: JSON.stringify({ value }),
  });
  return settingUpdateResult(data, key);
}

type SettingsImportUpdateResult = Awaited<ReturnType<typeof updateSetting>>;

export type SettingsImportSuccess = {
  success: true;
  imported: number;
  results: SettingsImportUpdateResult[];
};

export type NoSupportedSettingsImportFailure = {
  success: false;
  imported: 0;
  results: SettingsImportUpdateResult[];
  message: string;
};

export type SettingsImportResult =
  | SettingsImportSuccess
  | NoSupportedSettingsImportFailure;

export class NoSupportedSettingsImportError extends Error {
  constructor(failure: NoSupportedSettingsImportFailure) {
    super(failure.message);
    this.name = "NoSupportedSettingsImportError";
  }
}

export async function importSettings(
  payload: { settings?: Record<string, unknown> } | null | undefined
): Promise<SettingsImportResult> {
  const settings = payload?.settings || {};
  const imported: SettingsImportUpdateResult[] = [];
  if (Object.prototype.hasOwnProperty.call(settings, AUTO_APPROVE_KEY)) {
    const result = await updateSetting(
      AUTO_APPROVE_KEY,
      Boolean(settings[AUTO_APPROVE_KEY]),
    );
    imported.push(result);
    if (result.success === false) {
      return {
        success: false,
        imported: 0,
        results: imported,
        message: result.message || "The setting could not be saved",
      };
    }
  }
  if (imported.length === 0) {
    return {
      success: false,
      imported: 0,
      results: imported,
      message: "No supported settings were found in the selected file",
    };
  }
  return { success: true, imported: imported.length, results: imported };
}
export async function fetchTools() {
  const data = await apiFetch(SETTINGS_TOOLS_BASE);
  return {
    tools: recordArrayField(data, "entries", "tool settings")
      .map(toolFromConfigEntry)
      .filter(Boolean),
    diagnostics: optionalUnknownArrayField(data, "diagnostics", "tool settings"),
  };
}
export async function updateToolPermission(name, state) {
  const normalized = normalizeToolUpdateState(state);
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), TOOL_PERMISSION_UPDATE_TIMEOUT_MS);
  try {
    const data = await apiFetch(`${SETTINGS_TOOLS_BASE}/${encodeURIComponent(name)}`, {
      method: "POST",
      body: JSON.stringify({ state: normalized }),
      signal: controller.signal,
    });
    const tool = persistedToolFromConfigEntry(data.entry, name, normalized);
    const entry = recordField(data, "entry", "tool permission update");
    return { success: true, tool, entry };
  } finally {
    clearTimeout(timeoutId);
  }
}
export async function fetchExtensions() {
  const response = await apiFetch("/api/webchat/v2/extensions");
  return {
    ...response,
    extensions: recordArrayField(response, "extensions", "extensions"),
  };
}
export async function fetchExtensionRegistry() {
  const response = await apiFetch("/api/webchat/v2/extensions/registry");
  return {
    ...response,
    entries: recordArrayField(response, "entries", "extension registry"),
  };
}
export function fetchSkills(): Promise<ApiRecord & { skills: ApiRecord[] }> {
  return apiFetch<ApiRecord & { skills: ApiRecord[] }>("/api/webchat/v2/skills");
}
export function fetchSkillContent(name) {
  return apiFetch(`/api/webchat/v2/skills/${encodeURIComponent(name)}`);
}
export function installSkill(payload) {
  return apiFetch("/api/webchat/v2/skills/install", {
    method: "POST",
    headers: { "X-Confirm-Action": "true" },
    body: JSON.stringify(payload),
  });
}
export function updateSkill(name, payload) {
  return apiFetch(`/api/webchat/v2/skills/${encodeURIComponent(name)}`, {
    method: "PUT",
    headers: { "X-Confirm-Action": "true" },
    body: JSON.stringify(payload),
  });
}
export function removeSkill(name) {
  return apiFetch(`/api/webchat/v2/skills/${encodeURIComponent(name)}`, {
    method: "DELETE",
    headers: { "X-Confirm-Action": "true" },
  });
}
export function setSkillAutoActivate(name, enabled) {
  return apiFetch(`/api/webchat/v2/skills/${encodeURIComponent(name)}/auto-activate`, {
    method: "POST",
    headers: { "X-Confirm-Action": "true" },
    body: JSON.stringify({ enabled }),
  });
}
// Global "auto-activate learned skills" master switch. When disabled, learned
// skills activate only via an explicit /name mention.
export function setAutoActivateLearned(enabled) {
  return apiFetch(`/api/webchat/v2/skills/auto-activate-learned`, {
    method: "POST",
    headers: { "X-Confirm-Action": "true" },
    body: JSON.stringify({ enabled }),
  });
}
export function fetchUsers() {
  return Promise.resolve({ users: [], todo: true });
}
export function createUser(_payload) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 users endpoint" });
}
export function updateUser(_id, _payload) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 users endpoint" });
}
