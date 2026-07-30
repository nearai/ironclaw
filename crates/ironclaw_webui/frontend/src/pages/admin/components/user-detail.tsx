// @ts-nocheck
import React from "react";
import { useT } from "../../../lib/i18n";
import {
  Button,
  ConfirmDialog,
  DetailList,
  DetailRow,
  Card,
  SelectMenu,
  Skeleton,
  Badge,
} from "@ironclaw/ui";
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
  adminUserActionErrorMessage,
} from "../lib/admin-presenters";

export function UserDetail({ userId, onBack }) {
  const userQuery = useAdminUserDetail(userId);
  const usageQuery = useUsage("month", userId);
  const adminState = useAdminUsers();
  return (
    <UserDetailView
      key={userId}
      onBack={onBack}
      userQuery={userQuery}
      usageQuery={usageQuery}
      adminState={adminState}
    />
  );
}

export function UserDetailView({ onBack, userQuery, usageQuery, adminState }) {
  const t = useT();
  const {
    suspendUser,
    activateUser,
    updateUser,
    deleteUser,
    isSuspending,
    suspendError,
    isActivating,
    activateError,
    isUpdating,
    updateError,
    resetUpdate,
    isDeleting,
    deleteError,
    resetDelete,
    resetActionErrors,
  } = adminState;

  const [role, setRole] = React.useState(null);
  const [confirmDelete, setConfirmDelete] = React.useState(false);
  const roleOptions = React.useMemo(() => buildRoleOptions(t), [t]);

  const user = userQuery.data;
  const usageEntries = usageQuery.data?.usage || [];
  const isActionPending = isSuspending || isActivating || isUpdating || isDeleting;
  const statusError = suspendError || activateError;

  React.useEffect(() => {
    if (user && role === null) setRole(user.role);
  }, [user]);

  if (userQuery.isLoading) {
    return (
      <div className="space-y-5">
        <Card className="p-5 sm:p-6">
          <Skeleton className="mb-2 h-6 w-48 rounded" />
          <Skeleton className="h-4 w-32 rounded" />
        </Card>
      </div>
    );
  }

  if (userQuery.error) {
    return (
      <Card className="p-5 sm:p-6">
        <p className="text-sm text-[var(--v2-danger-text)]">{t("error.loadFailed", { what: t("admin.users.user"), message: userQuery.error.message })}</p>
      </Card>
    );
  }

  if (!user) return null;

  const handleSaveRole = async () => {
    if (role && role !== user.role && !isActionPending) {
      resetActionErrors?.();
      try {
        await updateUser(user.id, { role });
      } catch (_) {
        // The mutation exposes its sanitized error below.
      }
    }
  };

  const handleStatusChange = async () => {
    if (isActionPending) return;
    resetActionErrors?.();
    try {
      if (user.status === "active") {
        await suspendUser(user.id);
      } else {
        await activateUser(user.id);
      }
    } catch (_) {
      // The mutation exposes its sanitized error below.
    }
  };

  const beginDelete = () => {
    if (isActionPending) return;
    resetDelete?.();
    setConfirmDelete(true);
  };

  const closeDelete = () => {
    if (isDeleting) return;
    setConfirmDelete(false);
    resetDelete?.();
  };

  const handleDelete = async () => {
    if (isActionPending) return;
    resetActionErrors?.();
    try {
      await deleteUser(user.id);
      setConfirmDelete(false);
      onBack();
    } catch (_) {
      // Keep the confirmation open so the administrator can retry.
    }
  };

  const handleRoleChange = (nextRole) => {
    if (isActionPending) return;
    resetUpdate?.();
    setRole(nextRole);
  };

  return (
    <div className="space-y-5">
      <button
        onClick={onBack}
        className="flex items-center gap-1.5 rounded text-xs text-iron-300 transition-colors hover:text-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)]"
      >
        <span>←</span>
        <span>{t("admin.users.backToUsers")}</span>
      </button>

      <Card className="p-5 sm:p-6">
        <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
          <div>
            <h2 className="text-2xl font-semibold tracking-tight text-white">{user.display_name || user.id}</h2>
            <div className="mt-2 flex items-center gap-2">
              <Badge tone={roleTone(user.role)} label={formatUserRole(user.role, t)} />
              <Badge tone={statusTone(user.status)} label={formatUserStatus(user.status, t)} />
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-2 sm:justify-end">
            {user.status === "active"
              ? (<Button variant="secondary" size="sm" className="min-w-24" loading={isSuspending} disabled={isActionPending} data-testid="admin-user-detail-status" onClick={handleStatusChange}>{isSuspending ? t("common.loading") : t("admin.users.suspend")}</Button>)
              : (<Button variant="secondary" size="sm" className="min-w-24" loading={isActivating} disabled={isActionPending} data-testid="admin-user-detail-status" onClick={handleStatusChange}>{isActivating ? t("common.loading") : t("admin.users.activate")}</Button>)}
            <Button
              variant="danger"
              size="sm"
              className="min-w-24"
              disabled={isActionPending}
              data-testid="admin-user-detail-delete"
              onClick={beginDelete}
            >
              {t("admin.users.delete")}
            </Button>
          </div>
        </div>
      </Card>

      {statusError && (
        <p className="text-sm text-[var(--v2-danger-text)]" role="alert" data-testid="admin-user-detail-status-error">
          {adminUserActionErrorMessage(statusError, t)}
        </p>
      )}

      <div className="grid gap-5 lg:grid-cols-2">
        <Card className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">{t("admin.user.profile")}</h3>
          <DetailList>
            <DetailRow term={t("admin.user.id")}>
              <span className="font-mono text-xs">{user.id}</span>
            </DetailRow>
            <DetailRow term={t("admin.user.email")}>{user.email || t("admin.user.notSet")}</DetailRow>
            <DetailRow term={t("admin.user.created")}>{formatRelativeTime(user.created_at, t)}</DetailRow>
            <DetailRow term={t("admin.user.lastLogin")}>{formatRelativeTime(user.last_login_at, t)}</DetailRow>
            {user.created_by && (
              <DetailRow term={t("admin.user.createdBy")}>
                <span className="font-mono text-xs">{truncateId(user.created_by)}</span>
              </DetailRow>
            )}
          </DetailList>
        </Card>

        <Card className="p-5 sm:p-6">
          <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">{t("admin.user.summary")}</h3>
          <DetailList>
            <DetailRow term={t("admin.user.jobs")}>{user.job_count ?? 0}</DetailRow>
            <DetailRow term={t("admin.user.totalCost")}>{formatCost(user.total_cost)}</DetailRow>
            <DetailRow term={t("admin.user.lastActive")}>{formatRelativeTime(user.last_active_at, t)}</DetailRow>
          </DetailList>
        </Card>
      </div>

      <Card className="p-5 sm:p-6">
        <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">{t("admin.user.roleManagement")}</h3>
        <div className="flex items-end gap-3">
          <div>
            <label className="mb-1 block text-xs text-iron-300">{t("admin.user.currentRole")}</label>
            <SelectMenu
              value={role || user.role}
              options={roleOptions}
              onChange={handleRoleChange}
              disabled={isActionPending}
              ariaLabel={t("admin.user.currentRole")}
              className="!min-w-0 w-36"
              buttonClassName="h-9 rounded-md border-white/12 bg-white/[0.04] px-3 font-sans text-sm text-iron-100"
            />
          </div>
          <Button data-testid="admin-user-detail-save-role" onClick={handleSaveRole} loading={isUpdating} disabled={isActionPending || !role || role === user.role}>
            {isUpdating ? t("common.saving") : t("admin.user.saveRole")}
          </Button>
        </div>
        {updateError && (
          <p className="mt-4 text-sm text-[var(--v2-danger-text)]" role="alert" data-testid="admin-user-detail-role-error">
            {adminUserActionErrorMessage(updateError, t)}
          </p>
        )}
      </Card>

      <UserSecretsPanel key={user.id} userId={user.id} />

      <Card className="p-5 sm:p-6">
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
      </Card>

      {confirmDelete && (
        <ConfirmDialog
          open
          title={t("admin.users.deleteUserTitle")}
          description={
            <>
              {t("admin.users.deleteUserDesc", { name: user.display_name })}
              {deleteError && (
                <span
                  className="mt-2 block text-[var(--v2-danger-text)]"
                  role="alert"
                  data-testid="admin-user-delete-error"
                >
                  {adminUserActionErrorMessage(deleteError, t)}
                </span>
              )}
            </>
          }
          confirmLabel={t("admin.users.delete")}
          cancelLabel={t("admin.users.cancel")}
          isConfirming={isDeleting}
          onConfirm={handleDelete}
          onCancel={closeDelete}
        />
      )}
    </div>
  );
}
