// @ts-nocheck
import { interpolateParams } from "../../../lib/i18n-format.js";

function tx(t, key, params = {}, fallback = key) {
  return typeof t === "function" ? t(key, params) : interpolateParams(fallback, params);
}

const USER_ROLE = Object.freeze({
  MEMBER: "member",
  ADMIN: "admin",
});

const USER_STATUS = Object.freeze({
  ACTIVE: "active",
  SUSPENDED: "suspended",
});

const USER_ROLE_LABELS = {
  [USER_ROLE.MEMBER]: "Member",
  [USER_ROLE.ADMIN]: "Admin",
};

const USER_ROLE_KEYS = {
  [USER_ROLE.MEMBER]: "admin.users.member",
  [USER_ROLE.ADMIN]: "admin.users.admin",
};

const USER_STATUS_LABELS = {
  [USER_STATUS.ACTIVE]: "Active",
  [USER_STATUS.SUSPENDED]: "Suspended",
};

const USER_STATUS_KEYS = {
  [USER_STATUS.ACTIVE]: "admin.users.status.active",
  [USER_STATUS.SUSPENDED]: "admin.users.status.suspended",
};

export function formatTokenCount(n) {
  if (n == null || n === 0) return "0";
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "K";
  return String(n);
}

export function formatCost(v) {
  if (v == null) return "$0.00";
  const n = parseFloat(v);
  if (isNaN(n)) return "$0.00";
  return "$" + n.toFixed(2);
}

export function formatUptime(secs) {
  if (!secs) return "0s";
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

export function formatRelativeTime(iso, t) {
  if (!iso) return tx(t, "admin.relative.never", {}, "Never");
  const parsed = new Date(iso).getTime();
  if (Number.isNaN(parsed)) return tx(t, "admin.relative.never", {}, "Never");
  const diff = (Date.now() - parsed) / 1000;
  if (diff < 0) return tx(t, "admin.relative.justNow", {}, "Just now");
  if (diff < 60) return tx(t, "admin.relative.justNow", {}, "Just now");
  if (diff < 3600) {
    return tx(t, "admin.relative.minutesAgo", { count: Math.floor(diff / 60) }, "{count}m ago");
  }
  if (diff < 86400) {
    return tx(t, "admin.relative.hoursAgo", { count: Math.floor(diff / 3600) }, "{count}h ago");
  }
  if (diff < 2592000) {
    return tx(t, "admin.relative.daysAgo", { count: Math.floor(diff / 86400) }, "{count}d ago");
  }
  return new Date(iso).toLocaleDateString();
}

export function truncateId(id) {
  if (!id) return "";
  return id.length > 12 ? id.slice(0, 12) + "…" : id;
}

export function statusTone(status) {
  if (status === USER_STATUS.ACTIVE) return "success";
  if (status === USER_STATUS.SUSPENDED) return "danger";
  return "muted";
}

export function roleTone(role) {
  if (role === USER_ROLE.ADMIN) return "signal";
  return "muted";
}

export function formatUserRole(role, t) {
  const value = String(role || USER_ROLE.MEMBER).toLowerCase();
  const fallback = USER_ROLE_LABELS[value];
  if (!fallback) return String(role || USER_ROLE_LABELS[USER_ROLE.MEMBER]);
  return tx(t, USER_ROLE_KEYS[value], {}, fallback);
}

export function formatUserStatus(status, t) {
  const value = String(status || USER_STATUS.ACTIVE).toLowerCase();
  const fallback = USER_STATUS_LABELS[value];
  if (!fallback) return String(status || USER_STATUS_LABELS[USER_STATUS.ACTIVE]);
  return tx(t, USER_STATUS_KEYS[value], {}, fallback);
}

export function summarizeUsers(users) {
  const total = users.length;
  const active = users.filter((u) => u.status === USER_STATUS.ACTIVE).length;
  const suspended = users.filter((u) => u.status === USER_STATUS.SUSPENDED).length;
  const admins = users.filter((u) => u.role === USER_ROLE.ADMIN).length;
  return { total, active, suspended, admins };
}

export function filterUsers(users, { search = "", filter = "all" }) {
  let result = users;
  if (filter === USER_STATUS.ACTIVE) result = result.filter((u) => u.status === USER_STATUS.ACTIVE);
  else if (filter === USER_STATUS.SUSPENDED) result = result.filter((u) => u.status === USER_STATUS.SUSPENDED);
  else if (filter === USER_ROLE.ADMIN) result = result.filter((u) => u.role === USER_ROLE.ADMIN);

  if (search.trim()) {
    const q = search.toLowerCase();
    result = result.filter(
      (u) =>
        (u.display_name && u.display_name.toLowerCase().includes(q)) ||
        (u.email && u.email.toLowerCase().includes(q)) ||
        (u.id && u.id.toLowerCase().includes(q))
    );
  }
  return result;
}

export function aggregateUsageByUser(entries) {
  const byUser = {};
  for (const e of entries) {
    if (!byUser[e.user_id]) {
      byUser[e.user_id] = { user_id: e.user_id, calls: 0, input_tokens: 0, output_tokens: 0, cost: 0 };
    }
    byUser[e.user_id].calls += e.call_count || 0;
    byUser[e.user_id].input_tokens += e.input_tokens || 0;
    byUser[e.user_id].output_tokens += e.output_tokens || 0;
    byUser[e.user_id].cost += parseFloat(e.total_cost) || 0;
  }
  return Object.values(byUser).sort((a, b) => b.cost - a.cost);
}

export function aggregateUsageByModel(entries) {
  const byModel = {};
  for (const e of entries) {
    if (!byModel[e.model]) {
      byModel[e.model] = { model: e.model, calls: 0, input_tokens: 0, output_tokens: 0, cost: 0 };
    }
    byModel[e.model].calls += e.call_count || 0;
    byModel[e.model].input_tokens += e.input_tokens || 0;
    byModel[e.model].output_tokens += e.output_tokens || 0;
    byModel[e.model].cost += parseFloat(e.total_cost) || 0;
  }
  return Object.values(byModel).sort((a, b) => b.cost - a.cost);
}

export function totalUsage(rows) {
  return rows.reduce(
    (acc, r) => ({
      calls: acc.calls + r.calls,
      input_tokens: acc.input_tokens + r.input_tokens,
      output_tokens: acc.output_tokens + r.output_tokens,
      cost: acc.cost + r.cost,
    }),
    { calls: 0, input_tokens: 0, output_tokens: 0, cost: 0 }
  );
}
