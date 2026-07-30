// @ts-nocheck
import React from "react";
import { useT } from "../../../lib/i18n";
import {
  Button,
  Callout,
  ConfirmDialog,
  FormField,
  Icon,
  Input,
  Card,
  SearchInput,
  SegmentedControl,
  SelectMenu,
  SkeletonList,
  Badge,
} from "@ironclaw/ui";
import { useAdminUsers } from "../hooks/useAdminUsers";
import {
  formatRelativeTime,
  formatCost,
  truncateId,
  statusTone,
  roleTone,
  formatUserRole,
  formatUserStatus,
  filterUsers,
  buildRoleOptions,
  adminUserActionErrorMessage,
} from "../lib/admin-presenters";

function buildFilters(t) {
  return [
    { value: "all", label: t("admin.users.filter.all") },
    { value: "active", label: t("admin.users.filter.active") },
    { value: "suspended", label: t("admin.users.filter.suspended") },
    { value: "admin", label: t("admin.users.filter.admins") },
  ];
}

function TokenBanner({ token, onDismiss }) {
  const t = useT();
  const [copied, setCopied] = React.useState(false);

  const handleCopy = () => {
    if (navigator.clipboard) {
      navigator.clipboard.writeText(token);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  return (
    <Callout
      tone="info"
      title={t("admin.users.tokenCreated")}
      onDismiss={onDismiss}
      dismissLabel={
        <>
          <span className="sr-only">{t("common.dismiss")}</span>
          <Icon name="close" className="h-4 w-4" aria-hidden="true" />
        </>
      }
    >
      {t("admin.users.tokenCreatedDesc")}
      <span className="mt-3 flex items-center gap-2">
        <code className="min-w-0 flex-1 truncate rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2 font-mono text-xs text-[var(--v2-text-strong)]">
          {token}
        </code>
        <Button variant="secondary" onClick={handleCopy}>
          {copied ? t("admin.users.copied") : t("admin.users.copy")}
        </Button>
      </span>
    </Callout>
  );
}

function CreateUserForm({ onCreate, isCreating, error, resetError }) {
  const t = useT();
  const [name, setName] = React.useState("");
  const [email, setEmail] = React.useState("");
  const [role, setRole] = React.useState("member");
  const [isOpen, setIsOpen] = React.useState(false);
  const roleOptions = React.useMemo(() => buildRoleOptions(t), [t]);

  const handleSubmit = async (e) => {
    e.preventDefault();
    if (!name.trim()) return;
    resetError?.();
    try {
      await onCreate({ display_name: name.trim(), email: email.trim() || undefined, role });
      setName("");
      setEmail("");
      setIsOpen(false);
    } catch (_) {
      // Keep the form open; the mutation exposes its sanitized error below.
    }
  };

  if (!isOpen) {
    return (
      <Button variant="secondary" onClick={() => setIsOpen(true)}>
        <Icon name="plus" className="mr-2 h-4 w-4" />
        {t("admin.users.newUser")}
      </Button>
    );
  }

  return (
    <Card className="p-5 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">{t("admin.users.createUser")}</h3>
      <form onSubmit={handleSubmit} className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-3">
          <FormField label={t("admin.users.displayName")} htmlFor="admin-user-display-name">
            <Input
              id="admin-user-display-name"
              size="sm"
              type="text"
              value={name}
              onChange={(e) => setName(e.currentTarget.value)}
              required
              placeholder={t("admin.users.displayNamePlaceholder")}
            />
          </FormField>
          <FormField label={t("admin.users.email")} htmlFor="admin-user-email">
            <Input
              id="admin-user-email"
              size="sm"
              type="email"
              value={email}
              onChange={(e) => setEmail(e.currentTarget.value)}
              placeholder={t("admin.users.emailPlaceholder")}
            />
          </FormField>
          <div>
            <label className="mb-1 block text-xs text-iron-300">{t("admin.users.role")}</label>
            <SelectMenu
              value={role}
              options={roleOptions}
              onChange={setRole}
              ariaLabel={t("admin.users.role")}
              className="w-full"
              buttonClassName="h-9 rounded-md border-iron-700 bg-iron-800/70 px-3 font-sans text-sm text-iron-100"
            />
          </div>
        </div>
        {error && (<p className="text-sm text-[var(--v2-danger-text)]">{error.message}</p>)}
        <div className="flex gap-2">
          <Button type="submit" disabled={isCreating}>
            {isCreating ? t("admin.users.creating") : t("admin.users.createUser")}
          </Button>
          <Button variant="ghost" type="button" onClick={() => setIsOpen(false)}>{t("admin.users.cancel")}</Button>
        </div>
      </form>
    </Card>
  );
}

export function UserRow({
  user,
  onSelect,
  onSuspend,
  onActivate,
  onChangeRole,
  isActionPending,
  isSuspending,
  isActivating,
  isUpdating,
}) {
  const t = useT();
  return (
    <div className="flex items-center justify-between gap-4 border-t border-iron-700 py-3.5 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick={() => onSelect(user.id)}
            className="text-sm font-medium text-signal hover:underline"
          >
            {user.display_name || user.id}
          </button>
          <Badge tone={roleTone(user.role)} label={formatUserRole(user.role, t)} />
          <Badge tone={statusTone(user.status)} label={formatUserStatus(user.status, t)} />
        </div>
        <div className="mt-0.5 flex flex-wrap gap-x-4 gap-y-0.5">
          {user.email && (<span className="font-mono text-xs text-iron-300">{user.email}</span>)}
          <span className="font-mono text-xs text-iron-700">{truncateId(user.id)}</span>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-2">
        <span className="hidden font-mono text-xs text-iron-300 sm:inline">
          {user.job_count != null ? t("admin.users.jobsCount", { count: user.job_count }) : ""}
          {user.total_cost != null ? ` · ${formatCost(user.total_cost)}` : ""}
        </span>
        <span className="hidden text-xs text-iron-700 lg:inline">{formatRelativeTime(user.last_active_at, t)}</span>
        <div className="flex gap-1">
          {user.status === "active"
            ? (
                <Button
                  variant="secondary"
                  size="sm"
                  data-testid="admin-user-suspend"
                  disabled={isActionPending}
                  loading={isSuspending}
                  onClick={() => onSuspend(user.id)}
                >
                  {isSuspending ? t("common.loading") : t("admin.users.suspend")}
                </Button>
              )
            : (
                <Button
                  variant="secondary"
                  size="sm"
                  data-testid="admin-user-activate"
                  disabled={isActionPending}
                  loading={isActivating}
                  onClick={() => onActivate(user.id)}
                >
                  {isActivating ? t("common.loading") : t("admin.users.activate")}
                </Button>
              )}
          <Button
            variant="secondary"
            size="sm"
            data-testid="admin-user-role"
            disabled={isActionPending}
            loading={isUpdating}
            onClick={() => onChangeRole(user.id, user.role === "admin" ? "member" : "admin")}
          >
            {isUpdating
              ? t("common.saving")
              : user.role === "admin" ? t("admin.users.demote") : t("admin.users.promote")}
          </Button>
        </div>
      </div>
    </div>
  );
}

export function AdminUsersTab({ onSelectUser }) {
  const adminState = useAdminUsers();
  return (
    <AdminUsersTabView
      onSelectUser={onSelectUser}
      adminState={adminState}
    />
  );
}

export function AdminUsersTabView({ onSelectUser, adminState }) {
  const t = useT();
  const {
    users, query, isForbidden, createUser, isCreating, createError,
    resetCreate,
    updateUser, suspendUser, activateUser,
    isUpdating, updateError, updatingUserId,
    isSuspending, suspendError, suspendingUserId, resetSuspend,
    isActivating, activateError, activatingUserId,
    resetActionErrors,
    newToken, clearToken,
  } = adminState;

  const [search, setSearch] = React.useState("");
  const [filter, setFilter] = React.useState("all");
  const [confirm, setConfirm] = React.useState(null);

  const filtered = filterUsers(users, { search, filter });
  const FILTERS = buildFilters(t);

  const isActionPending = isUpdating || isSuspending || isActivating;
  const actionError = activateError || updateError;

  const handleSuspend = (id) => {
    resetSuspend?.();
    setConfirm({
      userId: id,
      title: t("admin.users.suspendTitle"),
      message: t("admin.users.suspendDesc"),
      confirmLabel: t("admin.users.suspend"),
    });
  };

  const confirmSuspend = async () => {
    if (!confirm?.userId || isActionPending) return;
    resetActionErrors?.();
    try {
      await suspendUser(confirm.userId);
      setConfirm(null);
    } catch (_) {
      // Keep the confirmation open so the administrator can retry.
    }
  };

  const handleActivate = async (id) => {
    if (isActionPending) return;
    resetActionErrors?.();
    try {
      await activateUser(id);
    } catch (_) {
      // The mutation exposes its sanitized error in the list panel.
    }
  };

  const handleChangeRole = async (id, role) => {
    if (isActionPending) return;
    resetActionErrors?.();
    try {
      await updateUser(id, { role });
    } catch (_) {
      // The mutation exposes its sanitized error in the list panel.
    }
  };

  const closeConfirm = () => {
    if (isSuspending) return;
    setConfirm(null);
    resetSuspend?.();
  };

  if (query.isLoading) {
    return (
      <Card className="p-5 sm:p-6">
        <SkeletonList
          label={t("admin.users.loading")}
          itemClassName="h-10 rounded"
          className="space-y-3"
        />
      </Card>
    );
  }

  if (isForbidden) {
    return (
      <Card className="p-6 sm:p-8">
        <div className="flex items-center gap-3">
          <Icon name="lock" className="h-5 w-5 text-iron-700" />
          <h3 className="text-lg font-semibold text-iron-100">{t("users.adminRequired")}</h3>
        </div>
        <p className="mt-2 max-w-md text-sm leading-6 text-iron-300">
          {t("users.adminRequiredDesc")}
        </p>
      </Card>
    );
  }

  return (
    <div className="space-y-5">
      {newToken && (
        <TokenBanner
          token={newToken.token || newToken.plaintext_token}
          onDismiss={clearToken}
        />
      )}

      <CreateUserForm
        onCreate={createUser}
        isCreating={isCreating}
        error={createError}
        resetError={resetCreate}
      />

      <Card className="p-5 sm:p-6">
        <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
            {t("admin.users.title", { count: filtered.length, total: users.length })}
          </h3>
          <div className="flex items-center gap-2">
            <SearchInput
              label={t("admin.users.searchLabel")}
              placeholder={t("admin.users.searchPlaceholder")}
              value={search}
              onChange={(e) => setSearch(e.currentTarget.value)}
              onClear={() => setSearch("")}
              clearLabel={t("admin.users.clearSearch")}
              className="w-48"
            />
            <SegmentedControl
              label={t("admin.users.filterLabel")}
              options={FILTERS}
              value={filter}
              onChange={setFilter}
            />
          </div>
        </div>

        {actionError && (
          <p className="mb-4 text-sm text-[var(--v2-danger-text)]" role="alert" data-testid="admin-user-action-error">
            {adminUserActionErrorMessage(actionError, t)}
          </p>
        )}

        {filtered.length === 0
          ? (<p className="py-4 text-sm text-iron-300">{t("admin.users.noMatch")}</p>)
          : filtered.map(
              (user) => (
                <UserRow
                  key={user.id}
                  user={user}
                  onSelect={onSelectUser}
                  onSuspend={handleSuspend}
                  onActivate={handleActivate}
                  onChangeRole={handleChangeRole}
                  isActionPending={isActionPending}
                  isSuspending={isSuspending && suspendingUserId === user.id}
                  isActivating={isActivating && activatingUserId === user.id}
                  isUpdating={isUpdating && updatingUserId === user.id}
                />
              )
            )}
      </Card>

      {confirm && (
        <ConfirmDialog
          open
          title={confirm.title}
          description={
            <>
              {confirm.message}
              {suspendError && (
                <span
                  className="mt-2 block text-[var(--v2-danger-text)]"
                  role="alert"
                  data-testid="admin-user-confirm-error"
                >
                  {adminUserActionErrorMessage(suspendError, t)}
                </span>
              )}
            </>
          }
          confirmLabel={confirm.confirmLabel}
          cancelLabel={t("admin.users.cancel")}
          isConfirming={isSuspending}
          onConfirm={confirmSuspend}
          onCancel={closeConfirm}
        />
      )}
    </div>
  );
}
