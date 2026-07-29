import { Button, Icon } from "@ironclaw/design-system";
import { LazyPopover } from "./lazy-popover";
import { useT } from "../lib/i18n";

function profileName(profile) {
  return profile?.display_name || profile?.email || profile?.id || "IronClaw";
}

function profileInitial(profile) {
  return profileName(profile).trim().charAt(0).toUpperCase() || "I";
}

export function SidebarFooter({ theme, toggleTheme, profile, onSignOut }) {
  const t = useT();
  const name = profileName(profile);
  const detail = profile?.email || profile?.role || t("common.gatewaySession");

  return (
    <div
      className="relative flex items-center gap-2 border-t border-[var(--v2-panel-border)] px-3 py-3"
    >
      <LazyPopover
        side="top"
        align="start"
        contentClassName="w-60 rounded-[10px] bg-[var(--v2-surface)] p-3"
        triggerProps={{
          className: "flex min-w-0 flex-1 items-center gap-2 rounded-[8px] text-left",
          title: name,
        }}
        trigger={
          <>
            <div
              className="grid h-8 w-8 shrink-0 overflow-hidden rounded-full bg-[var(--v2-accent-soft)] text-[11px] font-semibold text-[var(--v2-accent-text)]"
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
          </>
        }
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
      </LazyPopover>
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        onClick={toggleTheme}
        className="shrink-0 hover:bg-[var(--v2-surface-muted)]"
        title={theme === "dark" ? t("theme.light") : t("theme.dark")}
        aria-label={theme === "dark" ? t("theme.light") : t("theme.dark")}
      >
        <Icon name={theme === "dark" ? "sun" : "moon"} className="h-4 w-4" />
      </Button>
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        onClick={onSignOut}
        className="shrink-0 hover:bg-[var(--v2-surface-muted)]"
        title={t("header.signOut")}
        aria-label={t("header.signOut")}
      >
        <Icon name="logout" className="h-4 w-4" />
      </Button>
    </div>
  );
}
