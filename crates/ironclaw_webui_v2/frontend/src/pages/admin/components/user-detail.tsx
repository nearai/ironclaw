// @ts-nocheck
import React from "react";
import { useT } from "../../../lib/i18n";
import { Panel, StatCard, StatusPill } from "../../../design-system/primitives";
import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { useAdminUserDetail, useAdminUsers } from "../hooks/useAdminUsers";
import { useUsage } from "../hooks/useAdminUsage";
import {
  formatRelativeTime,
  formatCost,
  formatTokenCount,
  truncateId,
  statusTone,
  roleTone,
  formatUserRole,
  formatUserStatus,
} from "../lib/admin-presenters";

function DetailRow({ label, children }) {
  return (
    <div className="flex items-start justify-between gap-4 border-t border-white/[0.06] py-3 first:border-0 first:pt-0">
      <span className="text-xs text-iron-300">{label}</span>
      <span className="text-right text-sm text-iron-100">{children}</span>
    </div>
  );
}

export function UserDetail({ userId, onBack }) {
  const t = useT();
  const userQuery = useAdminUserDetail(userId);
  const usageQuery = useUsage("month", userId);
  const { suspendUser, activateUser, updateUser, deleteUser, createToken, newToken, clearToken } = useAdminUsers();

  const [role, setRole] = React.useState(null);
  const [confirmDelete, setConfirmDelete] = React.useState(false);

  const user = userQuery.data;
  const usageEntries = usageQuery.data?.usage || [];

  React.useEffect(() => {
    if (user && role === null) setRole(user.role);
  }, [user]);

  if (userQuery.isLoading) {
    return (
      <div className="space-y-5">
        <Panel className="p-5 sm:p-6">
          <div className="v2-skeleton mb-2 h-6 w-48 rounded" />
          <div className="v2-skeleton h-4 w-32 rounded" />
        </Panel>
      </div>
    );
  }

  if (userQuery.error) {
    return (
      <Panel className="p-5 sm:p-6">
        <p className="text-sm text-red-200">{t("error.loadFailed", { what: t("admin.users.user"), message: userQuery.error.message })}</p>
      </Panel>
    );
  }

  if (!user) return null;

  const handleSaveRole = async () => {
    if (role && role !== user.role) {
      await updateUser(user.id, { role });
    }
  };

  const handleDelete = async () => {
    await deleteUser(user.id);
    onBack();
  };

  const handleCreateToken = async () => {
    const name = window.prompt(t("admin.users.tokenNamePrompt", { name: user.display_name || t("admin.users.userFallback") }));
    if (!name) return;
    await createToken(user.id, name);
  };

  return (
    <div className="space-y-5">
      <button
        onClick={onBack}
        className="flex items-center gap-1.5 text-xs text-iron-300 hover:text-white"
      >
        <span>←</span>
        <span>{t("admin.users.backToUsers")}</span>
      </button>

      <Panel className="p-5 sm:p-6">
        <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
          <div>
            <h2 className="text-2xl font-semibold tracking-tight text-white">{user.display_name || user.id}</h2>
            <div className="mt-2 flex items-center gap-2">
              <StatusPill tone={roleTone(user.role)} label={formatUserRole(user.role, t)} />
              <StatusPill tone={statusTone(user.status)} label={formatUserStatus(user.status, t)} />
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            {user.status === "active"
              ? (<Button variant="secondary" onClick={() => suspendUser(user.id)}>{t("admin.users.suspend")}</Button>)
              : (<Button variant="secondary" onClick={() => activateUser(user.id)}>{t("admin.users.activate")}</Button>)}
            <Button variant="secondary" onClick={handleCreateToken}>{t("admin.users.createToken")}</Button>
            <button
              onClick={() => setConfirmDelete(true)}
              className="v2-button inline-flex h-10 items-center justify-center rounded-md border border-red-400/30 bg-red-500/10 px-4 text-sm font-semibold text-red-200 hover:bg-red-500/20"
            >
              {t("admin.users.delete")}
            </button>
          </div>
        </div>
      </Panel>

      {(newToken?.token || newToken?.plaintext_token) && (
        <div className="rounded-xl border border-signal/30 bg-signal/10 p-4 sm:p-5">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0 flex-1">
              <p className="text-sm font-semibold text-white">{t("admin.users.tokenCreated")}</p>
              <p className="mt-1 text-xs text-iron-300">{t("admin.users.tokenCreatedDesc")}</p>
              <code className="mt-2 block truncate rounded-md border border-white/10 bg-white/[0.04] px-3 py-2 font-mono text-xs text-iron-100">
                {newToken.token || newToken.plaintext_token}
              </code>
            </div>
            <button onClick={clearToken} className="text-iron-300 hover:text-white">
              <Icon name="close" className="h-4 w-4" />
            </button>
          </div>
        </div>
      )}

      <div className="grid gap-5 lg:grid-cols-2">
        <Panel className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">{t("admin.user.profile")}</h3>
          <DetailRow label={t("admin.user.id")}>
            <span className="font-mono text-xs">{user.id}</span>
          </DetailRow>
          <DetailRow label={t("admin.user.email")}>{user.email || t("admin.user.notSet")}</DetailRow>
          <DetailRow label={t("admin.user.created")}>{formatRelativeTime(user.created_at, t)}</DetailRow>
          <DetailRow label={t("admin.user.lastLogin")}>{formatRelativeTime(user.last_login_at, t)}</DetailRow>
          {user.created_by && (
            <DetailRow label={t("admin.user.createdBy")}>
              <span className="font-mono text-xs">{truncateId(user.created_by)}</span>
            </DetailRow>
          )}
        </Panel>

        <Panel className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">{t("admin.user.summary")}</h3>
          <DetailRow label={t("admin.user.jobs")}>{user.job_count ?? 0}</DetailRow>
          <DetailRow label={t("admin.user.totalCost")}>{formatCost(user.total_cost)}</DetailRow>
          <DetailRow label={t("admin.user.lastActive")}>{formatRelativeTime(user.last_active_at, t)}</DetailRow>
        </Panel>
      </div>

      <Panel className="p-5 sm:p-6">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">{t("admin.user.roleManagement")}</h3>
        <div className="flex items-end gap-3">
          <div>
            <label className="mb-1 block text-xs text-iron-300">{t("admin.user.currentRole")}</label>
            <select
              value={role || user.role}
              onChange={(e) => setRole(e.currentTarget.value)}
              className="v2-select h-9 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
            >
              <option value="member">{t("admin.users.member")}</option>
              <option value="admin">{t("admin.users.admin")}</option>
            </select>
          </div>
          <Button onClick={handleSaveRole} disabled={!role || role === user.role}>
            {t("admin.user.saveRole")}
          </Button>
        </div>
      </Panel>

      <Panel className="p-5 sm:p-6">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">{t("admin.user.usage30Days")}</h3>
        {usageEntries.length === 0
          ? (<p className="py-4 text-sm text-iron-300">{t("admin.user.noUsage")}</p>)
          : (
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b border-white/10 text-left">
                      <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">{t("admin.usage.model")}</th>
                      <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">{t("admin.usage.calls")}</th>
                      <th className="hidden pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300 sm:table-cell">{t("admin.usage.input")}</th>
                      <th className="hidden pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300 sm:table-cell">{t("admin.usage.output")}</th>
                      <th className="pb-3 font-mono text-[11px] uppercase tracking-[0.14em] text-iron-300">{t("admin.usage.cost")}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {usageEntries.map(
                      (e, i) => (
                        <tr key={i} className="border-b border-white/[0.06] last:border-0">
                          <td className="py-3 pr-4 font-mono text-xs text-iron-100">{e.model}</td>
                          <td className="py-3 pr-4 font-mono text-xs text-iron-300">{(e.call_count || 0).toLocaleString()}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">{formatTokenCount(e.input_tokens)}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-iron-300 sm:table-cell">{formatTokenCount(e.output_tokens)}</td>
                          <td className="py-3 font-mono text-xs text-iron-100">{formatCost(e.total_cost)}</td>
                        </tr>
                      )
                    )}
                  </tbody>
                </table>
              </div>
            )}
      </Panel>

      {confirmDelete && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" onClick={() => setConfirmDelete(false)}>
          <div className="w-full max-w-md rounded-xl border border-white/10 bg-iron-900 p-6" onClick={(e) => e.stopPropagation()}>
            <h3 className="text-lg font-semibold text-white">{t("admin.users.deleteUserTitle")}</h3>
            <p className="mt-2 text-sm text-iron-300">
              {t("admin.users.deleteUserDesc", { name: user.display_name })}
            </p>
            <div className="mt-5 flex justify-end gap-2">
              <Button variant="ghost" onClick={() => setConfirmDelete(false)}>{t("admin.users.cancel")}</Button>
              <button
                onClick={handleDelete}
                className="v2-button inline-flex h-10 items-center justify-center rounded-md bg-red-500/20 px-4 text-sm font-semibold text-red-200 hover:bg-red-500/30"
              >
                {t("admin.users.delete")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
