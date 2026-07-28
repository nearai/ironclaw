// @ts-nocheck
import React from "react";
import { useT } from "../../../lib/i18n";
import { Panel, StatCard, StatusPill } from "../../../design-system/primitives";
import { useUsageSummary } from "../hooks/useAdminUsage";
import { useAdminUsers } from "../hooks/useAdminUsers";
import {
  formatCost,
  formatUptime,
  formatRelativeTime,
  statusTone,
  roleTone,
  formatUserRole,
  formatUserStatus,
  summarizeUsers,
} from "../lib/admin-presenters";

function RecentUsersTable({ users, onSelectUser }) {
  const t = useT();
  const recent = [...users]
    .sort((a, b) => {
      const ta = a.last_active_at || a.created_at || "";
      const tb = b.last_active_at || b.created_at || "";
      return tb.localeCompare(ta);
    })
    .slice(0, 8);

  if (!recent.length) {
    return (<p className="py-4 text-sm text-iron-300">{t("admin.dashboard.noUsers")}</p>);
  }

  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-white/10 text-left">
            <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">{t("admin.dashboard.name")}</th>
            <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">{t("admin.dashboard.role")}</th>
            <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">{t("admin.dashboard.status")}</th>
            <th className="hidden pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300 sm:table-cell">{t("admin.dashboard.jobs")}</th>
            <th className="pb-3 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">{t("admin.dashboard.lastActive")}</th>
          </tr>
        </thead>
        <tbody>
          {recent.map(
            (u) => (
              <tr key={u.id} className="border-b border-white/[0.06] last:border-0">
                <td className="py-3 pr-4">
                  <button
                    onClick={() => onSelectUser(u.id)}
                    className="text-sm font-medium text-signal hover:underline"
                  >
                    {u.display_name || u.id}
                  </button>
                </td>
                <td className="py-3 pr-4"><StatusPill tone={roleTone(u.role)} label={formatUserRole(u.role, t)} /></td>
                <td className="py-3 pr-4"><StatusPill tone={statusTone(u.status)} label={formatUserStatus(u.status, t)} /></td>
                <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">{u.job_count ?? 0}</td>
                <td className="py-3 text-xs text-iron-300">{formatRelativeTime(u.last_active_at, t)}</td>
              </tr>
            )
          )}
        </tbody>
      </table>
    </div>
  );
}

export function DashboardTab({ onSelectUser, onNavigateTab }) {
  const t = useT();
  const summaryQuery = useUsageSummary();
  const { users, query: usersQuery } = useAdminUsers();
  const summary = summaryQuery.data || {};
  const userStats = summarizeUsers(users);
  const usage30d = summary.usage_30d || {};
  const jobs = summary.jobs || {};

  const isLoading = summaryQuery.isLoading || usersQuery.isLoading;

  if (isLoading) {
    return (
      <div className="space-y-5">
        <Panel className="p-5 sm:p-6">
          <div className="v2-skeleton mb-4 h-4 w-32 rounded" />
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            {[1, 2, 3, 4].map((i) => (<div key={i} className="v2-skeleton h-28 rounded-lg" />))}
          </div>
        </Panel>
      </div>
    );
  }

  return (
    <div className="space-y-5">
      <Panel className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">{t("admin.dashboard.systemOverview")}</h3>
          {summary.uptime_seconds != null && (
            <span className="font-mono text-xs text-iron-300">{t("admin.dashboard.uptime", { value: formatUptime(summary.uptime_seconds) })}</span>
          )}
        </div>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <StatCard
            label={t("admin.dashboard.totalUsers")}
            value={String(userStats.total)}
            tone={userStats.total > 0 ? "success" : "muted"}
          />
          <StatCard
            label={t("admin.dashboard.activeUsers")}
            value={String(userStats.active)}
            tone="success"
          />
          <StatCard
            label={t("admin.dashboard.suspended")}
            value={String(userStats.suspended)}
            tone={userStats.suspended > 0 ? "danger" : "muted"}
          />
          <StatCard
            label={t("admin.dashboard.admins")}
            value={String(userStats.admins)}
            tone="signal"
          />
        </div>
      </Panel>

      <Panel className="p-5 sm:p-6">
        <h3 className="mb-5 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">{t("admin.dashboard.usage30d")}</h3>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <StatCard
            label={t("admin.dashboard.totalJobs")}
            value={String(jobs.total || 0)}
            tone="muted"
          />
          <StatCard
            label={t("admin.dashboard.llmCalls")}
            value={String(usage30d.llm_calls || 0)}
            tone="muted"
          />
          <StatCard
            label={t("admin.dashboard.totalCost")}
            value={formatCost(usage30d.total_cost)}
            tone="signal"
          />
          <StatCard
            label={t("admin.dashboard.activeJobs")}
            value={String(jobs.in_progress || 0)}
            tone={(jobs.in_progress || 0) > 0 ? "success" : "muted"}
          />
        </div>
      </Panel>

      <Panel className="p-5 sm:p-6">
        <div className="mb-5 flex items-center justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">{t("admin.dashboard.recentUsers")}</h3>
          <button
            onClick={() => onNavigateTab("users")}
            className="text-xs text-signal hover:underline"
          >
            {t("admin.dashboard.viewAll")}
          </button>
        </div>
        <RecentUsersTable users={users} onSelectUser={onSelectUser} />
      </Panel>
    </div>
  );
}
