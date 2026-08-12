import { Icon } from "../design-system/icons";
import React from "react";
import { useT } from "../lib/i18n";
import { cn } from "../utils/cn";

function profileName(profile) {
  return profile?.display_name || profile?.email || profile?.id || "IronClaw";
}

function profileInitial(profile) {
  return profileName(profile).trim().charAt(0).toUpperCase() || "I";
}

function useAccountPopover() {
  const [open, setOpen] = React.useState(false);
  const toggle = React.useCallback(() => {
    setOpen((next) => !next);
  }, []);

  return { open, toggle };
}

export function SidebarFooter({ theme, toggleTheme, profile, onSignOut }) {
  const t = useT();
  const accountPopover = useAccountPopover();
  const name = profileName(profile);
  const detail = profile?.email || profile?.role || t("common.gatewaySession");

  return (
    <div
      className="relative flex items-center gap-2 border-t border-[var(--v2-panel-border)] px-3 py-3"
    >
      {accountPopover.open &&
      (
        <div
          className={cn(
            "absolute bottom-full left-3 right-3 mb-2 rounded-[10px] border p-3 shadow-lg",
            "border-[var(--v2-panel-border)] bg-[var(--v2-surface)]"
          )}
        >
          <div className="truncate text-sm font-medium text-[var(--v2-text-strong)]">
            {name}
          </div>
          {profile?.email &&
          (<div className="mt-1 truncate text-xs text-[var(--v2-text-muted)]">
            {profile.email}
          </div>)}
          {profile?.role &&
          (<div className="mt-2 text-[11px] uppercase text-[var(--v2-text-faint)]">
            {profile.role}
          </div>)}
        </div>
      )}

      <button
        type="button"
        onClick={accountPopover.toggle}
        className="flex min-w-0 flex-1 items-center gap-2 rounded-[8px] text-left"
        title={name}
      >
        <div
          className="grid h-8 w-8 shrink-0 overflow-hidden rounded-full bg-[var(--v2-accent-soft)] text-[11px] font-bold text-[var(--v2-accent-text)]"
        >
          {profile?.avatar_url
          ? (<img
              src={profile.avatar_url}
              alt=""
              referrerPolicy="no-referrer"
              className="h-full w-full object-cover"
            />)
          : (<span className="place-self-center">{profileInitial(profile)}</span>)}
        </div>
        <span className="min-w-0">
          <span className="block truncate text-[13px] font-medium text-[var(--v2-text-strong)]">
            {name}
          </span>
          <span className="block truncate text-[11px] text-[var(--v2-text-faint)]">
            {detail}
          </span>
        </span>
      </button>
      <button
        onClick={toggleTheme}
        className="grid h-8 w-8 shrink-0 place-items-center rounded-[8px] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
        title={theme === "dark" ? t("theme.light") : t("theme.dark")}
      >
        <Icon name={theme === "dark" ? "sun" : "moon"} className="h-4 w-4" />
      </button>
      <button
        onClick={onSignOut}
        className="grid h-8 w-8 shrink-0 place-items-center rounded-[8px] text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]"
        title={t("header.signOut")}
      >
        <Icon name="logout" className="h-4 w-4" />
      </button>
    </div>
  );
}
