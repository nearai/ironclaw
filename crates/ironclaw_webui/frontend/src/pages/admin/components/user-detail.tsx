// @ts-nocheck
import React from "react";
import { useT } from "../../../lib/i18n";
import { Panel, StatCard, StatusPill } from "@ironclaw/design-system";
import { Button } from "@ironclaw/design-system";
import { Heading, Text } from "@ironclaw/design-system";
import { SelectMenu } from "@ironclaw/design-system";
import { Modal, ModalBody, ModalFooter } from "@ironclaw/design-system";
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

function DetailRow({ label, children }) {
  return (
    <div className="flex items-start justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3 first:border-0 first:pt-0">
      <Text variant="caption" tone="muted">{label}</Text>
      <Text as="span" variant="body" tone="strong" className="text-right">{children}</Text>
    </div>
  );
}

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
        <Text variant="body" tone="danger">{t("error.loadFailed", { what: t("admin.users.user"), message: userQuery.error.message })}</Text>
      </Panel>
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
        className="flex items-center gap-1.5 text-xs text-[var(--v2-text-muted)] hover:text-[var(--v2-text-strong)]"
      >
        <span>←</span>
        <span>{t("admin.users.backToUsers")}</span>
      </button>

      <Panel className="p-5 sm:p-6">
        <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
          <div>
            <Heading level={2}>{user.display_name || user.id}</Heading>
            <div className="mt-2 flex items-center gap-2">
              <StatusPill tone={roleTone(user.role)} label={formatUserRole(user.role, t)} />
              <StatusPill tone={statusTone(user.status)} label={formatUserStatus(user.status, t)} />
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
      </Panel>

      {statusError && (
        <Text variant="body" tone="danger" role="alert" data-testid="admin-user-detail-status-error">
          {adminUserActionErrorMessage(statusError, t)}
        </Text>
      )}

      <div className="grid gap-5 lg:grid-cols-2">
        <Panel className="p-5 sm:p-6">
          <Text as="h3" variant="eyebrow" tone="accent" className="mb-4">{t("admin.user.profile")}</Text>
          <DetailRow label={t("admin.user.id")}>
            <Text variant="mono" tone="inherit">{user.id}</Text>
          </DetailRow>
          <DetailRow label={t("admin.user.email")}>{user.email || t("admin.user.notSet")}</DetailRow>
          <DetailRow label={t("admin.user.created")}>{formatRelativeTime(user.created_at, t)}</DetailRow>
          <DetailRow label={t("admin.user.lastLogin")}>{formatRelativeTime(user.last_login_at, t)}</DetailRow>
          {user.created_by && (
            <DetailRow label={t("admin.user.createdBy")}>
              <Text variant="mono" tone="inherit">{truncateId(user.created_by)}</Text>
            </DetailRow>
          )}
        </Panel>

        <Panel className="p-5 sm:p-6">
          <Text as="h3" variant="eyebrow" tone="accent" className="mb-4">{t("admin.user.summary")}</Text>
          <DetailRow label={t("admin.user.jobs")}>{user.job_count ?? 0}</DetailRow>
          <DetailRow label={t("admin.user.totalCost")}>{formatCost(user.total_cost)}</DetailRow>
          <DetailRow label={t("admin.user.lastActive")}>{formatRelativeTime(user.last_active_at, t)}</DetailRow>
        </Panel>
      </div>

      <Panel className="p-5 sm:p-6">
        <Text as="h3" variant="eyebrow" tone="accent" className="mb-4">{t("admin.user.roleManagement")}</Text>
        <div className="flex items-end gap-3">
          <div>
            <Text as="label" variant="caption" tone="muted" className="mb-1 block">{t("admin.user.currentRole")}</Text>
            <SelectMenu
              value={role || user.role}
              options={roleOptions}
              onChange={handleRoleChange}
              disabled={isActionPending}
              ariaLabel={t("admin.user.currentRole")}
              className="!min-w-0 w-36"
              buttonClassName="h-9 rounded-md border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] px-3 font-sans text-sm text-[var(--v2-text-strong)]"
            />
          </div>
          <Button data-testid="admin-user-detail-save-role" onClick={handleSaveRole} loading={isUpdating} disabled={isActionPending || !role || role === user.role}>
            {isUpdating ? t("common.saving") : t("admin.user.saveRole")}
          </Button>
        </div>
        {updateError && (
          <Text variant="body" tone="danger" className="mt-4" role="alert" data-testid="admin-user-detail-role-error">
            {adminUserActionErrorMessage(updateError, t)}
          </Text>
        )}
      </Panel>

      <UserSecretsPanel key={user.id} userId={user.id} />

      <Panel className="p-5 sm:p-6">
        <Text as="h3" variant="eyebrow" tone="accent" className="mb-4">{t("admin.user.usage30Days")}</Text>
        {usageEntries.length === 0
          ? (<Text variant="body" tone="muted" className="py-4">{t("admin.user.noUsage")}</Text>)
          : (
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b border-[var(--v2-panel-border)] text-left">
                      <Text as="th" variant="eyebrow" tone="muted" className="pb-3 pr-4">{t("admin.usage.model")}</Text>
                      <Text as="th" variant="eyebrow" tone="muted" className="pb-3 pr-4">{t("admin.usage.calls")}</Text>
                      <Text as="th" variant="eyebrow" tone="muted" className="hidden pb-3 pr-4 sm:table-cell">{t("admin.usage.input")}</Text>
                      <Text as="th" variant="eyebrow" tone="muted" className="hidden pb-3 pr-4 sm:table-cell">{t("admin.usage.output")}</Text>
                      <Text as="th" variant="eyebrow" tone="muted" className="pb-3">{t("admin.usage.cost")}</Text>
                    </tr>
                  </thead>
                  <tbody>
                    {usageEntries.map(
                      (e, i) => (
                        <tr key={i} className="border-b border-[var(--v2-panel-border)] last:border-0">
                          <Text as="td" variant="mono" tone="strong" className="py-3 pr-4">{e.model}</Text>
                          <Text as="td" variant="mono" tone="muted" className="py-3 pr-4">{(e.call_count || 0).toLocaleString()}</Text>
                          <Text as="td" variant="mono" tone="muted" className="hidden py-3 pr-4 sm:table-cell">{formatTokenCount(e.input_tokens)}</Text>
                          <Text as="td" variant="mono" tone="muted" className="hidden py-3 pr-4 sm:table-cell">{formatTokenCount(e.output_tokens)}</Text>
                          <Text as="td" variant="mono" tone="strong" className="py-3">{formatCost(e.total_cost)}</Text>
                        </tr>
                      )
                    )}
                  </tbody>
                </table>
              </div>
            )}
      </Panel>

      {confirmDelete && (
        <Modal
          open
          size="sm"
          data-testid="admin-user-delete-dialog"
          title={t("admin.users.deleteUserTitle")}
          closeLabel={t("admin.users.cancel")}
          onClose={closeDelete}
        >
          <ModalBody>
            <Text variant="body" tone="muted">
              {t("admin.users.deleteUserDesc", { name: user.display_name })}
            </Text>
            {deleteError && (
              <Text
                variant="body"
                tone="danger"
                className="mt-4"
                role="alert"
                data-testid="admin-user-delete-error"
              >
                {adminUserActionErrorMessage(deleteError, t)}
              </Text>
            )}
          </ModalBody>
          <ModalFooter>
            <Button
              data-testid="admin-user-delete-cancel"
              variant="ghost"
              size="sm"
              disabled={isDeleting}
              onClick={closeDelete}
            >
              {t("admin.users.cancel")}
            </Button>
            <Button
              variant="danger"
              size="sm"
              loading={isDeleting}
              disabled={isDeleting}
              data-testid="admin-user-delete-confirm"
              onClick={handleDelete}
            >
              {isDeleting ? t("common.loading") : t("admin.users.delete")}
            </Button>
          </ModalFooter>
        </Modal>
      )}
    </div>
  );
}
