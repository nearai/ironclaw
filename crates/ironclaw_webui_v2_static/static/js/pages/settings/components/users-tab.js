import { Button } from "../../../design-system/button.js";
import { Badge } from "../../../design-system/badge.js";
import { Card } from "../../../design-system/card.js";
import { Icon } from "../../../design-system/icons.js";
import { Input, FormField, Label } from "../../../design-system/input.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { useUsers } from "../hooks/useUsers.js";
import { matchesSearch } from "../lib/settings-search.js";

function CreateUserForm({ onCreate, isCreating, error }) {
  const t = useT();
  const [name, setName] = React.useState("");
  const [email, setEmail] = React.useState("");
  const [role, setRole] = React.useState("member");
  const [isOpen, setIsOpen] = React.useState(false);

  const handleSubmit = (e) => {
    e.preventDefault();
    if (!name.trim()) return;
    onCreate(
      { display_name: name.trim(), email: email.trim() || undefined, role },
      {
        onSuccess: () => {
          setName("");
          setEmail("");
          setIsOpen(false);
        },
      }
    );
  };

  if (!isOpen) {
    return html`
      <${Button} variant="secondary" onClick=${() => setIsOpen(true)}>
        <${Icon} name="plus" className="mr-2 h-4 w-4" />
        ${t("users.addUser")}
      <//>
    `;
  }

  return html`
    <${Card} padding="md">
      <h3
        className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
      >
        ${t("users.newUser")}
      </h3>
      <form onSubmit=${handleSubmit} className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-2">
          <${FormField} label=${t("users.displayName")} htmlFor="user-name">
            <${Input}
              id="user-name"
              type="text"
              value=${name}
              onChange=${(e) => setName(e.target.value)}
              required
            />
          <//>
          <${FormField} label=${t("users.email")} htmlFor="user-email">
            <${Input}
              id="user-email"
              type="email"
              value=${email}
              onChange=${(e) => setEmail(e.target.value)}
            />
          <//>
        </div>
        <${FormField} label=${t("users.role")} htmlFor="user-role">
          <select
            id="user-role"
            value=${role}
            onChange=${(e) => setRole(e.target.value)}
            className="v2-select h-9 rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 text-sm text-[var(--v2-text-strong)] outline-none focus:border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))]"
          >
            <option value="member">${t("users.member")}</option>
            <option value="admin">${t("users.admin")}</option>
          </select>
        <//>
        ${error &&
        html` <p className="text-sm text-[var(--v2-danger-text)]">${error.message}</p> `}
        <div className="flex gap-2">
          <${Button} type="submit" disabled=${isCreating}>
            ${isCreating ? t("users.creating") : t("users.createUser")}
          <//>
          <${Button}
            variant="ghost"
            type="button"
            onClick=${() => setIsOpen(false)}
            >${t("users.cancel")}<//
          >
        </div>
      </form>
    <//>
  `;
}

function UserRow({ user }) {
  const t = useT();
  const statusTone = user.status === "active" ? "positive" : "danger";
  const roleTone = user.role === "admin" ? "accent" : "muted";

  return html`
    <div
      className="flex items-center justify-between gap-4 border-t border-[var(--v2-panel-border)] py-3.5 first:border-0 first:pt-0"
    >
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--v2-text)]"
            >${user.display_name || user.id}</span
          >
          <${Badge}
            tone=${roleTone}
            label=${user.role === "admin"
              ? t("users.admin")
              : t("users.member")}
            size="sm"
          />
          <${Badge} tone=${statusTone} label=${user.status || "active"} size="sm" />
        </div>
        ${user.email &&
        html`
          <div className="mt-0.5 font-mono text-xs text-[var(--v2-text-muted)]">
            ${user.email}
          </div>
        `}
      </div>
      <div
        className="flex shrink-0 items-center gap-4 font-mono text-[11px] text-[var(--v2-text-faint)]"
      >
        ${user.last_active &&
        html`<span>${new Date(user.last_active).toLocaleDateString()}</span>`}
      </div>
    </div>
  `;
}

export function UsersTab({ searchQuery = "" }) {
  const t = useT();
  const { users, query, isForbidden, createUser, createError, isCreating } =
    useUsers();

  if (query.isLoading) {
    return html`
      <${Card} padding="md">
        <div className="mb-4 h-3 w-24 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
        ${[1, 2, 3].map(
          (i) => html`
            <div
              key=${i}
              className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-3.5 first:border-0"
            >
              <div className="h-4 w-32 animate-pulse rounded bg-[var(--v2-surface-muted)]" />
              <div className="h-6 w-20 animate-pulse rounded-full bg-[var(--v2-surface-muted)]" />
            </div>
          `
        )}
      <//>
    `;
  }

  if (isForbidden) {
    return html`
      <${Card} padding="lg">
        <div className="flex items-center gap-3">
          <${Icon} name="lock" className="h-5 w-5 text-[var(--v2-text-faint)]" />
          <h3 className="text-lg font-semibold text-[var(--v2-text-strong)]">
            ${t("users.adminRequired")}
          </h3>
        </div>
        <p className="mt-2 max-w-md text-sm leading-6 text-[var(--v2-text-muted)]">
          ${t("users.adminRequiredDesc")}
        </p>
      <//>
    `;
  }

  if (query.error) {
    return html`
      <${Card} padding="md">
        <p className="text-sm text-[var(--v2-danger-text)]">
          ${t("users.failedLoad", { message: query.error.message })}
        </p>
      <//>
    `;
  }

  const filteredUsers = users.filter((user) =>
    matchesSearch(searchQuery, [
      user.id,
      user.display_name,
      user.email,
      user.role,
      user.status,
      user.last_active,
    ])
  );

  return html`
    <div className="space-y-5">
      <${CreateUserForm}
        onCreate=${createUser}
        isCreating=${isCreating}
        error=${createError}
      />

      <${Card} padding="md">
        <h3
          className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
        >
          ${t("users.title", { count: filteredUsers.length })}
        </h3>
        ${users.length === 0
          ? html`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("users.noUsers")}
            </p>`
          : filteredUsers.length === 0
          ? html`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("settings.noMatchingSettings", { query: searchQuery })}
            </p>`
          : filteredUsers.map(
              (user) => html`<${UserRow} key=${user.id} user=${user} />`
            )}
      <//>
    </div>
  `;
}
