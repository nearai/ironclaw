// @ts-nocheck
import React from "react";
import { useT } from "../../../lib/i18n";
import { Panel, StatCard, StatusPill } from "@ironclaw/design-system";
import { Button } from "@ironclaw/design-system";
import { SelectMenu } from "@ironclaw/design-system";
import { ConfirmDialog } from "@ironclaw/design-system";
import { useAdminUserDetail, useAdminUsers } from "../hooks/useAdminUsers";
import { useUsage } from "../hooks/useAdminUsage";
import { UserSecretsPanel } from "./user-secrets-panel";
import {
  formatRelativeTime,
  formatCost,
  formatTokenCount,
  truncateId,
  statusTone,
  roleTone,
  formatUserRole,
  formatUserStatus,
  buildRoleOptions,
} from "../lib/admin-presenters";

function DetailRow({ label, children }) {
  return (
    <div className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3 first:border-0 first:pt-0">
      <span className="text-xs text-[var(--v2-text-muted)]">{label}</span>
      <span className="text-right text-sm text-[var(--v2-text-strong)]">{children}</span>
    </div>
  );
}

export function UserDetail({ userId, onBack }) {
  const t = useT();
  const userQuery = useAdminUserDetail(userId);
  const usageQuery = useUsage("month", userId);
  const { suspendUser, activateUser, updateUser, deleteUser } = useAdminUsers();

  const [role, setRole] = React.useState(null);
  const [confirmDelete, setConfirmDelete] = React.useState(false);
  const roleOptions = React.useMemo(() => buildRoleOptions(t), [t]);

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
        <p className="text-sm text-[var(--v2-danger-text)]">{t("error.loadFailed", { what: t("admin.users.user"), message: userQuery.error.message })}</p>
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

  return (
    <div className="space-y-5">
      <button
        onClick={onBack}
        className="flex items-center gap-1.5 text-xs text-[var(--v2-text-muted)] hover:text-[var(--v2-text-strong)]"
      >
        <span>←</span>
        <span>{t("admin.users.backToUsers")}</span>
      </button>

      <Panel className="p-5 sm:p-6">
        <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
          <div>
            <h2 className="text-2xl font-medium tracking-tight text-[var(--v2-text-strong)]">{user.display_name || user.id}</h2>
            <div className="mt-2 flex items-center gap-2">
              <StatusPill tone={roleTone(user.role)} label={formatUserRole(user.role, t)} />
              <StatusPill tone={statusTone(user.status)} label={formatUserStatus(user.status, t)} />
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-2 sm:justify-end">
            {user.status === "active"
              ? (<Button variant="secondary" size="sm" className="min-w-24" onClick={() => suspendUser(user.id)}>{t("admin.users.suspend")}</Button>)
              : (<Button variant="secondary" size="sm" className="min-w-24" onClick={() => activateUser(user.id)}>{t("admin.users.activate")}</Button>)}
            <Button
              variant="danger"
              size="sm"
              className="min-w-24"
              onClick={() => setConfirmDelete(true)}
            >
              {t("admin.users.delete")}
            </Button>
          </div>
        </div>
      </Panel>

      <div className="grid gap-5 lg:grid-cols-2">
        <Panel className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">{t("admin.user.profile")}</h3>
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
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">{t("admin.user.summary")}</h3>
          <DetailRow label={t("admin.user.jobs")}>{user.job_count ?? 0}</DetailRow>
          <DetailRow label={t("admin.user.totalCost")}>{formatCost(user.total_cost)}</DetailRow>
          <DetailRow label={t("admin.user.lastActive")}>{formatRelativeTime(user.last_active_at, t)}</DetailRow>
        </Panel>
      </div>

      <Panel className="p-5 sm:p-6">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">{t("admin.user.roleManagement")}</h3>
        <div className="flex items-end gap-3">
          <div>
            <label className="mb-1 block text-xs text-[var(--v2-text-muted)]">{t("admin.user.currentRole")}</label>
            <SelectMenu
              value={role || user.role}
              options={roleOptions}
              onChange={setRole}
              ariaLabel={t("admin.user.currentRole")}
              className="!min-w-0 w-36"
              buttonClassName="h-9 rounded-md border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] px-3 font-sans text-sm text-[var(--v2-text-strong)]"
            />
          </div>
          <Button onClick={handleSaveRole} disabled={!role || role === user.role}>
            {t("admin.user.saveRole")}
          </Button>
        </div>
      </Panel>

      <UserSecretsPanel key={user.id} userId={user.id} />

      <Panel className="p-5 sm:p-6">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">{t("admin.user.usage30Days")}</h3>
        {usageEntries.length === 0
          ? (<p className="py-4 text-sm text-[var(--v2-text-muted)]">{t("admin.user.noUsage")}</p>)
          : (
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b border-[var(--v2-panel-border)] text-left">
                      <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]">{t("admin.usage.model")}</th>
                      <th className="pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]">{t("admin.usage.calls")}</th>
                      <th className="hidden pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-text-muted)] sm:table-cell">{t("admin.usage.input")}</th>
                      <th className="hidden pb-3 pr-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-text-muted)] sm:table-cell">{t("admin.usage.output")}</th>
                      <th className="pb-3 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-text-muted)]">{t("admin.usage.cost")}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {usageEntries.map(
                      (e, i) => (
                        <tr key={i} className="border-b border-[var(--v2-panel-border)] last:border-0">
                          <td className="py-3 pr-4 font-mono text-xs text-[var(--v2-text-strong)]">{e.model}</td>
                          <td className="py-3 pr-4 font-mono text-xs text-[var(--v2-text-muted)]">{(e.call_count || 0).toLocaleString()}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-[var(--v2-text-muted)] sm:table-cell">{formatTokenCount(e.input_tokens)}</td>
                          <td className="hidden py-3 pr-4 font-mono text-xs text-[var(--v2-text-muted)] sm:table-cell">{formatTokenCount(e.output_tokens)}</td>
                          <td className="py-3 font-mono text-xs text-[var(--v2-text-strong)]">{formatCost(e.total_cost)}</td>
                        </tr>
                      )
                    )}
                  </tbody>
                </table>
              </div>
            )}
      </Panel>

      {confirmDelete && (
        <ConfirmDialog
          open
          title={t("admin.users.deleteUserTitle")}
          description={t("admin.users.deleteUserDesc", { name: user.display_name })}
          confirmLabel={t("admin.users.delete")}
          cancelLabel={t("admin.users.cancel")}
          onConfirm={handleDelete}
          onCancel={() => setConfirmDelete(false)}
        />
      )}
    </div>
  );
}
