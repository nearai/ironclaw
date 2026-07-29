// Admin fixtures: the `/admin/users` roster (list + CRUD + status/role +
// per-user secrets) and the manifest-driven extension-configuration groups.

import { DAY, HOUR, MINUTE, demoId, iso } from "./helpers";

export type AdminUser = {
  user_id: string;
  display_name: string;
  email: string | null;
  role: "admin" | "member";
  status: "active" | "suspended";
  created_at: string;
  created_by?: string;
  last_login_at: string | null;
  last_active_at: string | null;
  job_count: number;
  total_cost: number;
};

export const adminUsers: AdminUser[] = [
  {
    user_id: "demo-operator",
    display_name: "Avery Chen",
    email: "avery@ironclaw.dev",
    role: "admin",
    status: "active",
    created_at: iso(120 * DAY),
    last_login_at: iso(2 * HOUR),
    last_active_at: iso(8 * MINUTE),
    job_count: 412,
    total_cost: 128.4,
  },
  {
    user_id: "user-mira-oduya",
    display_name: "Mira Oduya",
    email: "mira@ironclaw.dev",
    role: "admin",
    status: "active",
    created_at: iso(96 * DAY),
    created_by: "demo-operator",
    last_login_at: iso(26 * HOUR),
    last_active_at: iso(25 * HOUR),
    job_count: 187,
    total_cost: 64.02,
  },
  {
    user_id: "user-jonas-lindqvist",
    display_name: "Jonas Lindqvist",
    email: "jonas@ironclaw.dev",
    role: "member",
    status: "active",
    created_at: iso(74 * DAY),
    created_by: "demo-operator",
    last_login_at: iso(3 * HOUR),
    last_active_at: iso(40 * MINUTE),
    job_count: 231,
    total_cost: 71.66,
  },
  {
    user_id: "user-priya-raman",
    display_name: "Priya Raman",
    email: "priya@ironclaw.dev",
    role: "member",
    status: "active",
    created_at: iso(41 * DAY),
    created_by: "user-mira-oduya",
    last_login_at: iso(5 * DAY),
    last_active_at: iso(5 * DAY),
    job_count: 58,
    total_cost: 12.9,
  },
  {
    user_id: "user-tomas-vega",
    display_name: "Tomás Vega",
    email: "tomas@ironclaw.dev",
    role: "member",
    status: "suspended",
    created_at: iso(60 * DAY),
    created_by: "demo-operator",
    last_login_at: iso(20 * DAY),
    last_active_at: iso(19 * DAY),
    job_count: 96,
    total_cost: 33.75,
  },
  {
    user_id: "user-noor-hassan",
    display_name: "Noor Hassan",
    email: "noor@ironclaw.dev",
    role: "member",
    status: "active",
    created_at: iso(3 * DAY),
    created_by: "user-mira-oduya",
    last_login_at: iso(2 * DAY),
    last_active_at: iso(6 * HOUR),
    job_count: 4,
    total_cost: 0.82,
  },
];

export function findAdminUser(id: string): AdminUser | undefined {
  return adminUsers.find((user) => user.user_id === id);
}

export function createAdminUser(body: Record<string, unknown>) {
  const user: AdminUser = {
    user_id: demoId("user"),
    display_name: String(body.display_name || "New teammate"),
    email: typeof body.email === "string" && body.email ? body.email : null,
    role: body.role === "admin" ? "admin" : "member",
    status: "active",
    created_at: new Date().toISOString(),
    created_by: "demo-operator",
    last_login_at: null,
    last_active_at: null,
    job_count: 0,
    total_cost: 0,
  };
  adminUsers.unshift(user);
  return {
    user,
    api_token: `icw_${user.user_id}_${Math.random().toString(36).slice(2, 14)}`,
  };
}

export function deleteAdminUser(id: string): boolean {
  const index = adminUsers.findIndex((user) => user.user_id === id);
  if (index < 0) return false;
  adminUsers.splice(index, 1);
  userSecrets.delete(id);
  return true;
}

/* ── Per-user secrets ──────────────────────────────────────────────── */

type UserSecret = { handle: string; created_at: string; updated_at: string };

const userSecrets = new Map<string, UserSecret[]>([
  [
    "demo-operator",
    [
      { handle: "github.token", created_at: iso(90 * DAY), updated_at: iso(12 * DAY) },
      { handle: "openai.api_key", created_at: iso(60 * DAY), updated_at: iso(60 * DAY) },
    ],
  ],
  [
    "user-jonas-lindqvist",
    [{ handle: "jira.api_token", created_at: iso(30 * DAY), updated_at: iso(30 * DAY) }],
  ],
]);

export function secretsForUser(userId: string): UserSecret[] {
  return userSecrets.get(userId) || [];
}

export function putUserSecret(userId: string, handle: string) {
  const list = userSecrets.get(userId) || [];
  userSecrets.set(userId, list);
  const nowIso = new Date().toISOString();
  const existing = list.find((secret) => secret.handle === handle);
  if (existing) {
    existing.updated_at = nowIso;
    return existing;
  }
  const secret = { handle, created_at: nowIso, updated_at: nowIso };
  list.push(secret);
  return secret;
}

export function deleteUserSecret(userId: string, handle: string) {
  const list = userSecrets.get(userId);
  if (!list) return;
  const index = list.findIndex((secret) => secret.handle === handle);
  if (index >= 0) list.splice(index, 1);
}

/* ── Extension configuration groups ────────────────────────────────── */

type ConfigurationField = {
  handle: string;
  label: string;
  required: boolean;
  secret: boolean;
  provided: boolean;
  value: string;
};

type ConfigurationGroup = {
  group_id: string;
  display_name: string;
  description: string;
  complete: boolean;
  revision: number;
  used_by: { package_id: string; display_name: string; installed: boolean }[];
  fields: ConfigurationField[];
};

export const extensionConfigurationGroups: ConfigurationGroup[] = [
  {
    group_id: "slack-app",
    display_name: "Slack app credentials",
    description:
      "Deployment-owned Slack app used by the Slack channel extension for OAuth and event delivery.",
    complete: true,
    revision: 4,
    used_by: [{ package_id: "nearai.slack", display_name: "Slack", installed: true }],
    fields: [
      {
        handle: "client_id",
        label: "Client ID",
        required: true,
        secret: false,
        provided: true,
        value: "8123456789.1234567890123",
      },
      {
        handle: "client_secret",
        label: "Client secret",
        required: true,
        secret: true,
        provided: true,
        value: "",
      },
      {
        handle: "signing_secret",
        label: "Signing secret",
        required: true,
        secret: true,
        provided: true,
        value: "",
      },
    ],
  },
  {
    group_id: "github-app",
    display_name: "GitHub App",
    description:
      "GitHub App identity shared by the GitHub tools extension across all users of this deployment.",
    complete: false,
    revision: 2,
    used_by: [{ package_id: "nearai.github", display_name: "GitHub", installed: true }],
    fields: [
      {
        handle: "app_id",
        label: "App ID",
        required: true,
        secret: false,
        provided: true,
        value: "412009",
      },
      {
        handle: "private_key",
        label: "Private key (PEM)",
        required: true,
        secret: true,
        provided: false,
        value: "",
      },
    ],
  },
];

export function replaceConfigurationGroup(
  groupId: string,
  values: { handle: string; value: string }[]
) {
  const group = extensionConfigurationGroups.find(
    (entry) => entry.group_id === groupId
  );
  if (!group) return null;
  for (const field of group.fields) {
    const incoming = values.find((entry) => entry.handle === field.handle);
    const value = (incoming?.value || "").trim();
    if (field.secret) {
      // Blank keeps the stored secret; a typed value replaces it.
      if (value) field.provided = true;
    } else {
      field.value = value;
      field.provided = value.length > 0;
    }
  }
  group.revision += 1;
  group.complete = group.fields.every((field) => !field.required || field.provided);
  return group;
}
