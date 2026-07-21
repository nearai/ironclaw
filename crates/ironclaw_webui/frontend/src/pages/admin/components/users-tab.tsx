// @ts-nocheck
import React from "react";
import { useT } from "../../../lib/i18n";
import { Panel, StatusPill, EmptyPanel } from "@ironclaw/design-system";
import { Button } from "@ironclaw/design-system";
import { Icon } from "@ironclaw/design-system";
import { SelectMenu } from "@ironclaw/design-system";
import { Input, FormField, ConfirmDialog } from "@ironclaw/design-system";
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
    <div className="rounded-xl border border-[color-mix(in_srgb,var(--v2-accent)_30%,transparent)] bg-[var(--v2-accent-soft)] p-4 sm:p-5">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium text-[var(--v2-text-strong)]">{t("admin.users.tokenCreated")}</p>
          <p className="mt-1 text-xs text-[var(--v2-text-muted)]">{t("admin.users.tokenCreatedDesc")}</p>
          <div className="mt-3 flex items-center gap-2">
            <code className="min-w-0 flex-1 truncate rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-code-bg)] px-3 py-2 font-mono text-xs text-[var(--v2-text-strong)]">
              {token}
            </code>
            <Button variant="secondary" onClick={handleCopy}>
              {copied ? t("admin.users.copied") : t("admin.users.copy")}
            </Button>
          </div>
        </div>
        <Button variant="ghost" size="icon-sm" onClick={onDismiss} aria-label={t("common.close")}>
          <Icon name="close" className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}

function CreateUserForm({ onCreate, isCreating, error }) {
  const t = useT();
  const [name, setName] = React.useState("");
  const [email, setEmail] = React.useState("");
  const [role, setRole] = React.useState("member");
  const [isOpen, setIsOpen] = React.useState(false);
  const roleOptions = React.useMemo(() => buildRoleOptions(t), [t]);

  const handleSubmit = async (e) => {
    e.preventDefault();
    if (!name.trim()) return;
    await onCreate({ display_name: name.trim(), email: email.trim() || undefined, role });
    setName("");
    setEmail("");
    setIsOpen(false);
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
    <Panel className="p-5 sm:p-6">
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">{t("admin.users.createUser")}</h3>
      <form onSubmit={handleSubmit} className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-3">
          <FormField label={t("admin.users.displayName")}>
            <Input
              type="text"
              value={name}
              onChange={(e) => setName(e.currentTarget.value)}
              required
              placeholder={t("admin.users.displayNamePlaceholder")}
            />
          </FormField>
          <FormField label={t("admin.users.email")}>
            <Input
              type="email"
              value={email}
              onChange={(e) => setEmail(e.currentTarget.value)}
              placeholder={t("admin.users.emailPlaceholder")}
            />
          </FormField>
          <FormField label={t("admin.users.role")}>
            <SelectMenu
              value={role}
              options={roleOptions}
              onChange={setRole}
              ariaLabel={t("admin.users.role")}
              className="w-full"
              buttonClassName="h-[var(--v2-control-h-md)] rounded-[var(--v2-radius-md)] border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] px-3 font-sans text-sm text-[var(--v2-text-strong)]"
            />
          </FormField>
        </div>
        {error && (<p className="text-sm text-[var(--v2-danger-text)]">{error.message}</p>)}
        <div className="flex gap-2">
          <Button type="submit" disabled={isCreating}>
            {isCreating ? t("admin.users.creating") : t("admin.users.createUser")}
          </Button>
          <Button variant="ghost" type="button" onClick={() => setIsOpen(false)}>{t("admin.users.cancel")}</Button>
        </div>
      </form>
    </Panel>
  );
}

function UserRow({ user, onSelect, onSuspend, onActivate, onChangeRole }) {
  const t = useT();
  return (
    <div className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <button
            onClick={() => onSelect(user.id)}
            className="text-sm font-medium text-[var(--v2-accent-text)] hover:underline"
          >
            {user.display_name || user.id}
          </button>
          <StatusPill tone={roleTone(user.role)} label={formatUserRole(user.role, t)} />
          <StatusPill tone={statusTone(user.status)} label={formatUserStatus(user.status, t)} />
        </div>
        <div className="mt-0.5 flex flex-wrap gap-x-4 gap-y-0.5">
          {user.email && (<span className="font-mono text-xs text-[var(--v2-text-muted)]">{user.email}</span>)}
          <span className="font-mono text-xs text-[var(--v2-text-faint)]">{truncateId(user.id)}</span>
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-2">
        <span className="hidden font-mono text-xs text-[var(--v2-text-muted)] sm:inline">
          {user.job_count != null ? t("admin.users.jobsCount", { count: user.job_count }) : ""}
          {user.total_cost != null ? ` · ${formatCost(user.total_cost)}` : ""}
        </span>
        <span className="hidden text-xs text-[var(--v2-text-faint)] lg:inline">{formatRelativeTime(user.last_active_at, t)}</span>
        <div className="flex gap-1">
          {user.status === "active"
            ? (<button onClick={() => onSuspend(user.id)} className="rounded-md border border-[var(--v2-panel-border)] px-2.5 py-1.5 text-[11px] font-medium text-[var(--v2-text-muted)] hover:border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] hover:text-[var(--v2-danger-text)]">{t("admin.users.suspend")}</button>)
            : (<button onClick={() => onActivate(user.id)} className="rounded-md border border-[var(--v2-panel-border)] px-2.5 py-1.5 text-[11px] font-medium text-[var(--v2-text-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))] hover:text-[var(--v2-accent-text)]">{t("admin.users.activate")}</button>)}
          <button
            onClick={() => onChangeRole(user.id, user.role === "admin" ? "member" : "admin")}
            className="rounded-md border border-[var(--v2-panel-border)] px-2.5 py-1.5 text-[11px] font-medium text-[var(--v2-text-muted)] hover:border-[var(--v2-panel-border)] hover:text-[var(--v2-text-strong)]"
          >
            {user.role === "admin" ? t("admin.users.demote") : t("admin.users.promote")}
          </button>
        </div>
      </div>
    </div>
  );
}

export function AdminUsersTab({ selectedUserId, onSelectUser }) {
  const t = useT();
  const {
    users, query, isForbidden, createUser, isCreating, createError,
    updateUser, deleteUser, suspendUser, activateUser,
    newToken, clearToken,
  } = useAdminUsers();

  const [search, setSearch] = React.useState("");
  const [filter, setFilter] = React.useState("all");
  const [confirm, setConfirm] = React.useState(null);

  const filtered = filterUsers(users, { search, filter });
  const FILTERS = buildFilters(t);

  const handleSuspend = (id) => {
    setConfirm({
      title: t("admin.users.suspendTitle"),
      message: t("admin.users.suspendDesc"),
      confirmLabel: t("admin.users.suspend"),
      onConfirm: () => { suspendUser(id); setConfirm(null); },
    });
  };

  if (query.isLoading) {
    return (
      <Panel className="p-5 sm:p-6">
        <div className="v2-skeleton mb-4 h-3 w-24 rounded" />
        {[1, 2, 3].map((i) => (
          <div key={i} className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-3.5 first:border-0">
            <div className="v2-skeleton h-4 w-32 rounded" />
            <div className="v2-skeleton h-6 w-20 rounded-full" />
          </div>
        ))}
      </Panel>
    );
  }

  if (isForbidden) {
    return (
      <Panel className="p-6 sm:p-8">
        <div className="flex items-center gap-3">
          <Icon name="lock" className="h-5 w-5 text-[var(--v2-text-faint)]" />
          <h3 className="text-lg font-medium text-[var(--v2-text-strong)]">{t("users.adminRequired")}</h3>
        </div>
        <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
          {t("users.adminRequiredDesc")}
        </p>
      </Panel>
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

      <CreateUserForm onCreate={createUser} isCreating={isCreating} error={createError} />

      <Panel className="p-5 sm:p-6">
        <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
            {t("admin.users.title", { count: filtered.length, total: users.length })}
          </h3>
          <div className="flex items-center gap-2">
            <div className="w-48">
              <Input
                type="text"
                size="md"
                placeholder={t("admin.users.searchPlaceholder")}
                value={search}
                onChange={(e) => setSearch(e.currentTarget.value)}
              />
            </div>
            <div className="flex gap-1">
              {FILTERS.map(
                (f) => (
                  <button
                    key={f.value}
                    onClick={() => setFilter(f.value)}
                    className={[
                      "rounded-md px-2.5 py-1.5 text-[11px] font-medium",
                      filter === f.value
                        ? "border border-[color-mix(in_srgb,var(--v2-accent)_35%,transparent)] bg-[var(--v2-accent-soft)] text-[var(--v2-text-strong)]"
                        : "border border-transparent text-[var(--v2-text-muted)] hover:text-[var(--v2-text-strong)]",
                    ].join(" ")}
                  >
                    {f.label}
                  </button>
                )
              )}
            </div>
          </div>
        </div>

        {filtered.length === 0
          ? (<p className="py-4 text-sm text-[var(--v2-text-muted)]">{t("admin.users.noMatch")}</p>)
          : filtered.map(
              (user) => (
                <UserRow
                  key={user.id}
                  user={user}
                  onSelect={onSelectUser}
                  onSuspend={handleSuspend}
                  onActivate={activateUser}
                  onChangeRole={(id, role) => updateUser(id, { role })}
                />
              )
            )}
      </Panel>

      {confirm && (
        <ConfirmDialog
          open
          title={confirm.title}
          description={confirm.message}
          confirmLabel={confirm.confirmLabel}
          cancelLabel={t("admin.users.cancel")}
          onConfirm={confirm.onConfirm}
          onCancel={() => setConfirm(null)}
        />
      )}
    </div>
  );
}
