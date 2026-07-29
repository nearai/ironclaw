// DEMO fixtures for the "system" surfaces: settings (LLM providers, skills,
// tools, traces, operator config), admin (users, extension configuration),
// extensions (installed + registry + setup + pairing), and logs.
//
// Response shapes are derived from the consuming api modules:
//   - src/pages/settings/lib/settings-api.ts (+ hooks/components)
//   - src/pages/admin/lib/admin-api.ts
//   - src/pages/extensions/lib/extensions-api.ts (+ extensions-schema.ts)
//   - src/lib/api.ts (queryLogs/queryOperatorLogs), src/pages/logs/lib
//   - src/lib/telegram-setup-api.ts, src/lib/extension-pairing-api.ts
//
// Mutations mutate the in-module fixture state so the UI reflects them until
// reload; destructive-but-irrelevant operations no-op with `{ json: {} }`.

import type { DemoRoute } from "../types";
import {
  deleteLlmProvider,
  llmActive,
  llmSnapshot,
  modelsForAdapter,
  setLlmActive,
  upsertLlmProvider,
} from "../fixtures/system/llm";
import {
  autoActivateLearned,
  findSkill,
  getSkillContent,
  installSkill,
  removeSkill,
  setAutoActivateLearned,
  setSkillContent,
  skills,
} from "../fixtures/system/skills";
import {
  operatorConfigEntry,
  setAutoApproveTools,
  setOperatorConfig,
  settingsToolsExport,
  updateToolPermission,
} from "../fixtures/system/tools";
import {
  accountLoginLink,
  accountTracesView,
  authorizeTraceHold,
  traceCreditView,
} from "../fixtures/system/traces";
import {
  adminUsers,
  createAdminUser,
  deleteAdminUser,
  deleteUserSecret,
  extensionConfigurationGroups,
  findAdminUser,
  putUserSecret,
  replaceConfigurationGroup,
  secretsForUser,
} from "../fixtures/system/admin";
import {
  clearTelegramSetup,
  completeExtensionOauth,
  findInstalledExtension,
  installExtension,
  installedExtensions,
  mintPairingCode,
  pairingStatus,
  registrySnapshot,
  removeExtension,
  saveTelegramSetup,
  setupDescriptorFor,
  submitExtensionSetup,
  telegramSetup,
  unpair,
} from "../fixtures/system/extensions";
import { logsResponse } from "../fixtures/system/logs";

function param(match: RegExpExecArray, index = 1): string {
  return decodeURIComponent(match[index]);
}

function handleLogsRequest(req: { url: URL }, source: string) {
  const params = req.url.searchParams;
  const limitRaw = Number(params.get("limit"));
  return {
    json: logsResponse(
      {
        level: params.get("level"),
        target: params.get("target"),
        threadId: params.get("thread_id"),
        runId: params.get("run_id"),
        turnId: params.get("turn_id"),
        toolCallId: params.get("tool_call_id"),
        toolName: params.get("tool_name"),
        source: params.get("source"),
        limit: Number.isFinite(limitRaw) && limitRaw > 0 ? limitRaw : null,
        tail: params.get("tail") === "true",
      },
      source
    ),
  };
}

export const systemRoutes: DemoRoute[] = [
  /* ── Settings · LLM providers ────────────────────────────────────── */
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/llm\/providers$/,
    handle: () => ({ json: llmSnapshot() }),
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/llm\/providers$/,
    handle: (req) => {
      if (req.body) upsertLlmProvider(req.body);
      return { json: { success: true } };
    },
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/llm\/providers\/([^/]+)\/delete$/,
    handle: (_req, match) => {
      deleteLlmProvider(param(match));
      return { json: { success: true } };
    },
  },
  {
    // The onboarding gate keys off the snapshot's `active`; this GET is kept
    // for completeness so any direct consumer sees the same selection.
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/llm\/active$/,
    handle: () => ({ json: { active: llmActive } }),
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/llm\/active$/,
    handle: (req) => {
      const providerId = String(req.body?.provider_id || "");
      const model = String(req.body?.model || "");
      if (providerId) setLlmActive(providerId, model);
      return { json: { success: true, active: { provider_id: providerId, model } } };
    },
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/llm\/test-connection$/,
    handle: (req) => ({
      json: {
        ok: true,
        message: `Connection OK — ${String(req.body?.provider_id || "provider")} responded in 214 ms.`,
      },
    }),
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/llm\/list-models$/,
    handle: (req) => {
      const models = modelsForAdapter(req.body?.adapter);
      return { json: { ok: true, models, message: `Fetched ${models.length} models.` } };
    },
  },

  /* ── Settings · Skills ───────────────────────────────────────────── */
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/skills$/,
    handle: () => ({ json: { skills, auto_activate_learned: autoActivateLearned } }),
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/skills\/install$/,
    handle: (req) => {
      const name = installSkill(req.body || {});
      return { json: { success: true, message: `Installed skill "${name}".` } };
    },
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/skills\/auto-activate-learned$/,
    handle: (req) => {
      setAutoActivateLearned(req.body?.enabled !== false);
      return {
        json: {
          success: true,
          message:
            req.body?.enabled !== false
              ? "Automatic skill activation enabled."
              : "Automatic skill activation disabled.",
        },
      };
    },
  },
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/skills\/([^/]+)$/,
    handle: (_req, match) => ({
      json: { success: true, content: getSkillContent(param(match)) },
    }),
  },
  {
    method: "PUT",
    pattern: /^\/api\/webchat\/v2\/skills\/([^/]+)$/,
    handle: (req, match) => {
      const name = param(match);
      if (!findSkill(name)) {
        return { json: { success: false, message: `Unknown skill "${name}".` } };
      }
      setSkillContent(name, String(req.body?.content || ""));
      return { json: { success: true, message: `Updated skill "${name}".` } };
    },
  },
  {
    method: "DELETE",
    pattern: /^\/api\/webchat\/v2\/skills\/([^/]+)$/,
    handle: (_req, match) => {
      const name = param(match);
      const removed = removeSkill(name);
      return {
        json: removed
          ? { success: true, message: `Removed skill "${name}".` }
          : { success: false, message: `Unknown skill "${name}".` },
      };
    },
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/skills\/([^/]+)\/auto-activate$/,
    handle: (req, match) => {
      const skill = findSkill(param(match));
      if (!skill) {
        return { json: { success: false, message: "Unknown skill." } };
      }
      skill.auto_activate = req.body?.enabled !== false;
      return {
        json: {
          success: true,
          message: skill.auto_activate
            ? `"${skill.name}" will auto-activate.`
            : `"${skill.name}" now activates only via /${skill.name}.`,
        },
      };
    },
  },

  /* ── Settings · Tools + auto-approve ─────────────────────────────── */
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/settings\/tools$/,
    handle: () => ({ json: settingsToolsExport() }),
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/settings\/tools$/,
    handle: (req) => ({
      json: { success: true, entry: setAutoApproveTools(req.body?.enabled === true) },
    }),
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/settings\/tools\/([^/]+)$/,
    handle: (req, match) => {
      const entry = updateToolPermission(param(match), String(req.body?.state || ""));
      if (!entry) {
        return { status: 404, json: { error: "unknown_tool" } };
      }
      return { json: { success: true, entry } };
    },
  },

  /* ── Settings · Operator config ──────────────────────────────────── */
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/operator\/config\/([^/]+)$/,
    handle: (_req, match) => ({ json: { entry: operatorConfigEntry(param(match)) } }),
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/operator\/config\/([^/]+)$/,
    handle: (req, match) => ({
      json: { success: true, entry: setOperatorConfig(param(match), req.body?.value) },
    }),
  },

  /* ── Settings · Trace Commons ────────────────────────────────────── */
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/traces\/credit$/,
    handle: () => ({ json: traceCreditView() }),
  },
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/traces\/account$/,
    handle: () => ({ json: accountTracesView() }),
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/traces\/account-login-link$/,
    handle: () => ({ json: accountLoginLink() }),
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/traces\/holds\/([^/]+)\/authorize$/,
    handle: (_req, match) => ({ json: { authorized: authorizeTraceHold(param(match)) } }),
  },

  /* ── Admin · Users ───────────────────────────────────────────────── */
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/admin\/users$/,
    handle: (req) => {
      const status = req.url.searchParams.get("status");
      const limitRaw = Number(req.url.searchParams.get("limit"));
      let users = status
        ? adminUsers.filter((user) => user.status === status)
        : [...adminUsers];
      if (Number.isFinite(limitRaw) && limitRaw > 0) {
        users = users.slice(0, limitRaw);
      }
      return { json: { users, next_cursor: null } };
    },
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/admin\/users$/,
    handle: (req) => {
      const created = createAdminUser(req.body || {});
      return { json: { user: created.user, api_token: created.api_token } };
    },
  },
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/admin\/users\/([^/]+)$/,
    handle: (_req, match) => ({ json: { user: findAdminUser(param(match)) || null } }),
  },
  {
    method: "PATCH",
    pattern: /^\/api\/webchat\/v2\/admin\/users\/([^/]+)$/,
    handle: (req, match) => {
      const user = findAdminUser(param(match));
      if (!user) return { status: 404, json: { error: "not_found" } };
      if (typeof req.body?.display_name === "string" && req.body.display_name) {
        user.display_name = req.body.display_name;
      }
      return { json: { user } };
    },
  },
  {
    method: "DELETE",
    pattern: /^\/api\/webchat\/v2\/admin\/users\/([^/]+)$/,
    handle: (_req, match) => {
      deleteAdminUser(param(match));
      return { json: { success: true } };
    },
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/admin\/users\/([^/]+)\/role$/,
    handle: (req, match) => {
      const user = findAdminUser(param(match));
      if (!user) return { status: 404, json: { error: "not_found" } };
      user.role = req.body?.role === "admin" ? "admin" : "member";
      return { json: { user } };
    },
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/admin\/users\/([^/]+)\/status$/,
    handle: (req, match) => {
      const user = findAdminUser(param(match));
      if (!user) return { status: 404, json: { error: "not_found" } };
      user.status = req.body?.status === "suspended" ? "suspended" : "active";
      return { json: { user } };
    },
  },
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/admin\/users\/([^/]+)\/secrets$/,
    handle: (_req, match) => ({ json: { secrets: secretsForUser(param(match)) } }),
  },
  {
    method: "PUT",
    pattern: /^\/api\/webchat\/v2\/admin\/users\/([^/]+)\/secrets\/([^/]+)$/,
    handle: (_req, match) => ({
      json: { secret: putUserSecret(param(match), param(match, 2)) },
    }),
  },
  {
    method: "DELETE",
    pattern: /^\/api\/webchat\/v2\/admin\/users\/([^/]+)\/secrets\/([^/]+)$/,
    handle: (_req, match) => {
      deleteUserSecret(param(match), param(match, 2));
      return { json: {} };
    },
  },

  /* ── Admin · Extension configuration ─────────────────────────────── */
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/operator\/extension-configuration$/,
    handle: () => ({ json: { groups: extensionConfigurationGroups } }),
  },
  {
    // The client swaps the returned group straight into its cache, so the
    // saved group document is the top-level payload.
    method: "PUT",
    pattern: /^\/api\/webchat\/v2\/operator\/extension-configuration\/([^/]+)$/,
    handle: (req, match) => {
      const values = Array.isArray(req.body?.values)
        ? (req.body?.values as { handle: string; value: string }[])
        : [];
      const group = replaceConfigurationGroup(param(match), values);
      if (!group) return { status: 404, json: { error: "not_found" } };
      return { json: group };
    },
  },

  /* ── Extensions · installed list + registry ──────────────────────── */
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/extensions$/,
    handle: () => ({ json: { extensions: installedExtensions } }),
  },
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/extensions\/registry$/,
    handle: () => ({ json: registrySnapshot() }),
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/extensions\/install$/,
    handle: (req) => {
      const ref = req.body?.package_ref;
      const id =
        typeof ref === "string"
          ? ref
          : String((ref as { id?: string } | null)?.id || "");
      const installed = installExtension(id);
      if (!installed) {
        return { json: { success: false, message: `Unknown extension "${id}".` } };
      }
      return { json: { success: true, message: `Installed ${installed.name}.` } };
    },
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/extensions\/import$/,
    handle: () => ({
      json: {
        success: true,
        message: "Imported extension bundle (demo mode: catalog unchanged).",
      },
    }),
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/extensions\/([^/]+)\/remove$/,
    handle: (_req, match) => {
      removeExtension(param(match));
      return { json: { success: true } };
    },
  },

  /* ── Extensions · setup + OAuth ──────────────────────────────────── */
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/extensions\/([^/]+)\/setup$/,
    handle: (_req, match) => ({ json: setupDescriptorFor(param(match)) }),
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/extensions\/([^/]+)\/setup$/,
    handle: (req, match) => {
      const id = param(match);
      const payload = (req.body?.payload || {}) as { secrets?: Record<string, unknown> };
      const complete = submitExtensionSetup(id, payload.secrets || {});
      return {
        json: {
          success: true,
          message: complete
            ? "Credentials saved — extension is now active."
            : "Credentials saved.",
        },
      };
    },
  },
  {
    // Demo mode has no real vendor to hand off to: the "OAuth flow" completes
    // instantly server-side (account connected, extension active), and the
    // response carries no authorization_url so the UI simply refreshes.
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/extensions\/([^/]+)\/setup\/oauth\/start$/,
    handle: (_req, match) => {
      completeExtensionOauth(param(match));
      return { json: { success: true } };
    },
  },

  /* ── Extensions · WebGeneratedCode pairing ───────────────────────── */
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/extensions\/([^/]+)\/pairing\/mint$/,
    handle: (_req, match) => {
      const id = param(match);
      const pending = mintPairingCode(id);
      // Simulate the user completing pairing on their device a few seconds
      // later; the panel's status poll then reports connected.
      setTimeout(() => {
        const state = pairingStatus(id);
        state.connected = true;
        state.pending = null;
      }, 6000);
      return { json: pending };
    },
  },
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/extensions\/([^/]+)\/pairing\/status$/,
    handle: (_req, match) => ({ json: pairingStatus(param(match)) }),
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/extensions\/([^/]+)\/pairing\/unpair$/,
    handle: (_req, match) => {
      unpair(param(match));
      return { json: {} };
    },
  },

  /* ── Channels · Telegram setup + pairing ─────────────────────────── */
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/channels\/telegram\/setup$/,
    handle: () => ({ json: telegramSetup }),
  },
  {
    method: "PUT",
    pattern: /^\/api\/webchat\/v2\/channels\/telegram\/setup$/,
    handle: (req) => ({ json: saveTelegramSetup(req.body || {}) }),
  },
  {
    method: "DELETE",
    pattern: /^\/api\/webchat\/v2\/channels\/telegram\/setup$/,
    handle: () => {
      clearTelegramSetup();
      return { json: {} };
    },
  },
  {
    method: "POST",
    pattern: /^\/api\/webchat\/v2\/channels\/telegram\/pairing$/,
    handle: () => ({ json: mintPairingCode("nearai.telegram") }),
  },
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/channels\/telegram\/pairing$/,
    handle: () => ({ json: pairingStatus("nearai.telegram") }),
  },
  {
    method: "DELETE",
    pattern: /^\/api\/webchat\/v2\/channels\/telegram\/pairing$/,
    handle: () => {
      unpair("nearai.telegram");
      return { json: {} };
    },
  },

  /* ── Logs ────────────────────────────────────────────────────────── */
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/logs$/,
    handle: (req) => handleLogsRequest(req, "caller"),
  },
  {
    method: "GET",
    pattern: /^\/api\/webchat\/v2\/operator\/logs$/,
    handle: (req) => handleLogsRequest(req, "operator"),
  },
];
