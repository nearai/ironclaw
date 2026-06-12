import { useQuery } from "@tanstack/react-query";
import { createFileRoute, Link } from "@tanstack/react-router";
import { useMemo } from "react";
import { type Passkey, type SessionData, sessionQueryOptions, useAuthClient } from "@/app";

export const Route = createFileRoute("/_layout/_authenticated/home")({
  head: () => ({
    meta: [
      { title: "Workspace | app" },
      { name: "description", content: "Your workspace center." },
    ],
  }),
  component: Home,
});

function Home() {
  const auth = useAuthClient();
  const { data: session } = useQuery<SessionData | null>(sessionQueryOptions(auth, undefined));
  const { data: passkeys = [] } = useQuery({
    queryKey: ["passkeys"],
    queryFn: async () => {
      const { data } = await auth.passkey.listUserPasskeys();
      return (data || []) as Passkey[];
    },
    staleTime: 60 * 1000,
  });
  const user = session?.user;
  const nearAccountId = auth.near.getAccountId();

  const profile = useMemo(() => {
    if (!user)
      return {
        isAnonymous: false,
        hasEmail: false,
        hasNear: false,
        hasPasskeys: false,
        isAdmin: false,
      };
    return {
      isAnonymous: user.isAnonymous || false,
      hasEmail: Boolean(user.email),
      hasNear: Boolean(nearAccountId),
      hasPasskeys: passkeys.length > 0,
      isAdmin: user.role === "admin",
    };
  }, [user, nearAccountId, passkeys.length]);

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <div className="flex shrink-0 items-center justify-between gap-3 border-b border-border bg-card px-4 py-2.5 sm:px-6 sm:py-3">
        <h1 className="text-xl font-semibold text-foreground">Workspace</h1>
        <Link
          to="/settings"
          preload="intent"
          className="h-9 rounded-[12px] bg-primary px-4 text-sm font-bold text-primary-foreground inline-flex items-center no-underline transition-colors duration-150 hover:opacity-90"
        >
          Settings
        </Link>
      </div>

      <div className="flex-1 overflow-y-auto px-6 py-6">
        <div className="mx-auto max-w-3xl space-y-6">
          {!user ? (
            <div className="text-muted-foreground text-center py-12 text-sm">Loading…</div>
          ) : (
            <>
              <div className="rounded-[12px] border border-border bg-card p-6">
                <div className="flex flex-wrap items-center gap-2 mb-4">
                  <Chip>workspace</Chip>
                  {profile.isAnonymous && <Chip>anonymous</Chip>}
                  {profile.isAdmin && <Chip accent>admin</Chip>}
                </div>
                <h2 className="text-foreground text-2xl font-semibold mb-1">
                  {user.name || user.email || user.id.slice(0, 8)}
                </h2>
                <p className="text-muted-foreground text-sm leading-relaxed">
                  Manage your identity and connected accounts.
                </p>
              </div>

              <div className="rounded-[12px] border border-border bg-card p-6">
                <div className="text-muted-foreground text-[11px] font-bold uppercase tracking-wider mb-4">
                  Identity Status
                </div>
                <div className="flex flex-col gap-2">
                  <InfoRow
                    label="email"
                    value={profile.hasEmail ? (user.email ?? "linked") : "not linked"}
                  />
                  <InfoRow
                    label="near"
                    value={profile.hasNear ? (nearAccountId ?? "linked") : "not linked"}
                    mono
                  />
                  <InfoRow
                    label="passkeys"
                    value={profile.hasPasskeys ? `${passkeys.length} registered` : "not linked"}
                  />
                  <InfoRow
                    label="profile"
                    value={profile.isAnonymous ? "anonymous session" : "persistent account"}
                  />
                </div>

                {profile.isAnonymous && (
                  <div className="mt-4 rounded-[8px] bg-brand-accent-light border border-brand-accent-border text-foreground text-[13px] leading-relaxed px-4 py-3">
                    Link an email or NEAR wallet before signing out to keep your data.
                  </div>
                )}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function InfoRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="grid grid-cols-[100px_1fr] gap-4 rounded-[8px] border border-border bg-muted px-3.5 py-2.5 items-center">
      <span className="text-muted-foreground text-[11px] font-bold uppercase tracking-wider">
        {label}
      </span>
      <span className={`text-foreground text-[13px] break-all ${mono ? "font-mono text-xs" : ""}`}>
        {value}
      </span>
    </div>
  );
}

function Chip({ children, accent }: { children: React.ReactNode; accent?: boolean }) {
  return (
    <span
      className={`inline-flex items-center rounded-[6px] px-2.5 py-0.5 text-[11px] font-semibold border ${accent ? "bg-brand-accent-light border-brand-accent-border" : "bg-secondary border-border"} text-foreground`}
    >
      {children}
    </span>
  );
}
