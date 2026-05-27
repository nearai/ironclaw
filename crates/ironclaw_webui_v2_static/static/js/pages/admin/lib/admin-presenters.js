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

export function formatRelativeTime(iso) {
  if (!iso) return "Never";
  const diff = (Date.now() - new Date(iso).getTime()) / 1000;
  if (diff < 0) return "Just now";
  if (diff < 60) return "Just now";
  if (diff < 3600) return Math.floor(diff / 60) + "m ago";
  if (diff < 86400) return Math.floor(diff / 3600) + "h ago";
  if (diff < 2592000) return Math.floor(diff / 86400) + "d ago";
  return new Date(iso).toLocaleDateString();
}

export function truncateId(id) {
  if (!id) return "";
  return id.length > 12 ? id.slice(0, 12) + "…" : id;
}

export function statusTone(status) {
  if (status === "active") return "success";
  if (status === "suspended") return "danger";
  return "muted";
}

export function roleTone(role) {
  if (role === "admin") return "signal";
  return "muted";
}

export function summarizeUsers(users) {
  const total = users.length;
  const active = users.filter((u) => u.status === "active").length;
  const suspended = users.filter((u) => u.status === "suspended").length;
  const admins = users.filter((u) => u.role === "admin").length;
  return { total, active, suspended, admins };
}

export function filterUsers(users, { search = "", filter = "all" }) {
  let result = users;
  if (filter === "active") result = result.filter((u) => u.status === "active");
  else if (filter === "suspended") result = result.filter((u) => u.status === "suspended");
  else if (filter === "admin") result = result.filter((u) => u.role === "admin");

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
