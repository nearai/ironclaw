// @ts-nocheck
import React from "react";
import { useT } from "../../../lib/i18n";
import { Panel, StatCard, StatusPill, Text } from "@ironclaw/design-system";
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
    return (<Text variant="body" tone="muted" className="py-4">{t("admin.dashboard.noUsers")}</Text>);
  }

  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-[var(--v2-panel-border)] text-left">
            <Text as="th" variant="eyebrow" tone="muted" className="pb-3 pr-4">{t("admin.dashboard.name")}</Text>
            <Text as="th" variant="eyebrow" tone="muted" className="pb-3 pr-4">{t("admin.dashboard.role")}</Text>
            <Text as="th" variant="eyebrow" tone="muted" className="pb-3 pr-4">{t("admin.dashboard.status")}</Text>
            <Text as="th" variant="eyebrow" tone="muted" className="hidden pb-3 pr-4 sm:table-cell">{t("admin.dashboard.jobs")}</Text>
            <Text as="th" variant="eyebrow" tone="muted" className="pb-3">{t("admin.dashboard.lastActive")}</Text>
          </tr>
        </thead>
        <tbody>
          {recent.map(
            (u) => (
              <tr key={u.id} className="border-b border-[var(--v2-panel-border)] last:border-0">
                <td className="py-3 pr-4">
                  <button
                    onClick={() => onSelectUser(u.id)}
                    className="text-sm font-medium text-[var(--v2-accent-text)] hover:underline"
                  >
                    {u.display_name || u.id}
                  </button>
                </td>
                <td className="py-3 pr-4"><StatusPill tone={roleTone(u.role)} label={formatUserRole(u.role, t)} /></td>
                <td className="py-3 pr-4"><StatusPill tone={statusTone(u.status)} label={formatUserStatus(u.status, t)} /></td>
                <Text as="td" variant="mono" tone="muted" className="hidden py-3 pr-4 sm:table-cell">{u.job_count ?? 0}</Text>
                <Text as="td" variant="caption" tone="muted" className="py-3">{formatRelativeTime(u.last_active_at, t)}</Text>
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
          <Text as="h3" variant="eyebrow" tone="accent">{t("admin.dashboard.systemOverview")}</Text>
          {summary.uptime_seconds != null && (
            <Text variant="mono" tone="muted">{t("admin.dashboard.uptime", { value: formatUptime(summary.uptime_seconds) })}</Text>
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
        <Text as="h3" variant="eyebrow" tone="accent" className="mb-5">{t("admin.dashboard.usage30d")}</Text>
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
          <Text as="h3" variant="eyebrow" tone="accent">{t("admin.dashboard.recentUsers")}</Text>
          <button
            onClick={() => onNavigateTab("users")}
            className="text-xs text-[var(--v2-accent-text)] hover:underline"
          >
            {t("admin.dashboard.viewAll")}
          </button>
        </div>
        <RecentUsersTable users={users} onSelectUser={onSelectUser} />
      </Panel>
    </div>
  );
}
