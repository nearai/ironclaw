// @ts-nocheck
import React from "react";
import { useT } from "../../../lib/i18n";
import { Panel, StatusPill, EmptyPanel } from "../../../design-system/primitives";
import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { SelectMenu } from "../../../design-system/select-menu";
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
    <div className="rounded-xl border border-signal/30 bg-signal/10 p-4 sm:p-5">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <p className="text-sm font-semibold text-iron-100">{t("admin.users.tokenCreated")}</p>
          <p className="mt-1 text-xs text-iron-300">{t("admin.users.tokenCreatedDesc")}</p>
          <div className="mt-3 flex items-center gap-2">
            <code className="min-w-0 flex-1 truncate rounded-md border border-iron-700 bg-iron-800/70 px-3 py-2 font-mono text-xs text-iron-100">
              {token}
            </code>
            <Button variant="secondary" onClick={handleCopy}>
              {copied ? t("admin.users.copied") : t("admin.users.copy")}
            </Button>
          </div>
        </div>
        <button onClick={onDismiss} className="text-iron-300 hover:text-iron-100">
          <Icon name="close" className="h-4 w-4" />
        </button>
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
      <h3 className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-signal">{t("admin.users.createUser")}</h3>
      <form onSubmit={handleSubmit} className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-3">
          <div>
            <label className="mb-1 block text-xs text-iron-300">{t("admin.users.displayName")}</label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.currentTarget.value)}
              required
              className="h-9 w-full rounded-md border border-iron-700 bg-iron-800/70 px-3 text-sm text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/45"
              placeholder={t("admin.users.displayNamePlaceholder")}
            />
          </div>
          <div>
            <label className="mb-1 block text-xs text-iron-300">{t("admin.users.email")}</label>
            <input
              type="email"
              value={email}
              onChange={(e) => setEmail(e.currentTarget.value)}
              className="h-9 w-full rounded-md border border-iron-700 bg-iron-800/70 px-3 text-sm text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/45"
              placeholder={t("admin.users.emailPlaceholder")}
            />
          </div>
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
    </Panel>
  );
}

function ConfirmModal({ title, message, confirmLabel, onConfirm, onCancel }) {
  const t = useT();
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" onClick={onCancel}>
      <div className="w-full max-w-md rounded-xl border border-iron-700 bg-iron-900 p-6" onClick={(e) => e.stopPropagation()}>
        <h3 className="text-lg font-semibold text-iron-100">{title}</h3>
        <p className="mt-2 text-sm text-iron-300">{message}</p>
        <div className="mt-5 flex justify-end gap-2">
          <Button variant="ghost" onClick={onCancel}>{t("admin.users.cancel")}</Button>
          <button
            onClick={onConfirm}
            className="v2-button inline-flex h-10 items-center justify-center rounded-md bg-[var(--v2-danger-soft)] px-4 text-sm font-semibold text-[var(--v2-danger-text)] hover:bg-[color-mix(in_srgb,var(--v2-danger-soft)_65%,var(--v2-danger-text))]"
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

function UserRow({ user, onSelect, onSuspend, onActivate, onChangeRole }) {
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
          <StatusPill tone={roleTone(user.role)} label={formatUserRole(user.role, t)} />
          <StatusPill tone={statusTone(user.status)} label={formatUserStatus(user.status, t)} />
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
            ? (<button onClick={() => onSuspend(user.id)} className="rounded-md border border-iron-700 px-2.5 py-1.5 text-[11px] font-medium text-iron-300 hover:border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] hover:text-[var(--v2-danger-text)]">{t("admin.users.suspend")}</button>)
            : (<button onClick={() => onActivate(user.id)} className="rounded-md border border-iron-700 px-2.5 py-1.5 text-[11px] font-medium text-iron-300 hover:border-signal/30 hover:text-signal">{t("admin.users.activate")}</button>)}
          <button
            onClick={() => onChangeRole(user.id, user.role === "admin" ? "member" : "admin")}
            className="rounded-md border border-iron-700 px-2.5 py-1.5 text-[11px] font-medium text-iron-300 hover:border-iron-700 hover:text-iron-100"
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
          <div key={i} className="flex items-center justify-between border-t border-iron-700 py-3.5 first:border-0">
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
          <Icon name="lock" className="h-5 w-5 text-iron-700" />
          <h3 className="text-lg font-semibold text-iron-100">{t("users.adminRequired")}</h3>
        </div>
        <p className="mt-2 max-w-md text-sm leading-6 text-iron-300">
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
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            {t("admin.users.title", { count: filtered.length, total: users.length })}
          </h3>
          <div className="flex items-center gap-2">
            <input
              type="text"
              placeholder={t("admin.users.searchPlaceholder")}
              value={search}
              onChange={(e) => setSearch(e.currentTarget.value)}
              className="h-8 w-48 rounded-md border border-iron-700 bg-iron-800/70 px-3 text-xs text-iron-100 outline-none placeholder:text-iron-400 focus:border-signal/45"
            />
            <div className="flex gap-1">
              {FILTERS.map(
                (f) => (
                  <button
                    key={f.value}
                    onClick={() => setFilter(f.value)}
                    className={[
                      "rounded-md px-2.5 py-1.5 text-[11px] font-medium",
                      filter === f.value
                        ? "border border-signal/35 bg-signal/10 text-iron-100"
                        : "border border-transparent text-iron-300 hover:text-iron-100",
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
          ? (<p className="py-4 text-sm text-iron-300">{t("admin.users.noMatch")}</p>)
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
        <ConfirmModal
          title={confirm.title}
          message={confirm.message}
          confirmLabel={confirm.confirmLabel}
          onConfirm={confirm.onConfirm}
          onCancel={() => setConfirm(null)}
        />
      )}
    </div>
  );
}
