import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { createFileRoute, useNavigate, useRouter } from "@tanstack/react-router";
import { useState } from "react";
import { toast } from "sonner";
import { type SessionData, sessionQueryOptions, useAuthClient } from "@/app";
import { Button } from "@/components";
import { Input } from "@/components/ui/input";

export const Route = createFileRoute("/_layout/_authenticated/settings/security")({
  component: SecuritySettings,
});

function SecuritySettings() {
  const auth = useAuthClient();
  const { data: session } = useQuery<SessionData | null>(sessionQueryOptions(auth));
  const user = session?.user;

  if (!user) return null;

  return <SecurityTab user={user} />;
}

function SecurityTab({ user }: { user: { email?: string; isAnonymous?: boolean | null } }) {
  const auth = useAuthClient();
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const router = useRouter();
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");

  const changePasswordMutation = useMutation({
    mutationFn: () => {
      if (newPassword !== confirmPassword) throw new Error("Passwords do not match");
      if (newPassword.length < 8) throw new Error("Password must be at least 8 characters");
      return (async () => {
        const { error } = await auth.changePassword({ currentPassword, newPassword });
        if (error) throw new Error(error.message);
      })();
    },
    onSuccess: () => {
      toast.success("Password changed");
      setCurrentPassword("");
      setNewPassword("");
      setConfirmPassword("");
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const revokeSessionsMutation = useMutation({
    mutationFn: async () => {
      const { error } = await auth.revokeSessions();
      if (error) throw new Error(error.message);
    },
    onSuccess: () => toast.success("Other sessions revoked"),
    onError: (err: Error) => toast.error(err.message),
  });

  const signOutMutation = useMutation({
    mutationFn: async () => {
      const { error } = await auth.signOut();
      if (error) throw new Error(error.message || "Failed to sign out");
      await auth.near.disconnect().catch(() => {});
    },
    onSuccess: async () => {
      queryClient.setQueryData(["session"], null);
      queryClient.removeQueries({ queryKey: ["passkeys"] });
      queryClient.removeQueries({ queryKey: ["organizations"] });
      await queryClient.invalidateQueries({ queryKey: ["session"] });
      await router.invalidate();
      await navigate({ to: "/chat", replace: true });
    },
    onError: (err: Error) => toast.error(err.message),
  });

  return (
    <div className="space-y-4">
      {user.email ? (
        <div className="rounded-[12px] border border-border bg-card p-6 space-y-4">
          <div className="text-[11px] font-bold uppercase tracking-wider text-muted-foreground">
            Change password
          </div>
          <div className="grid gap-4 md:grid-cols-3">
            <Field label="current">
              <Input
                type="password"
                value={currentPassword}
                onChange={(e) => setCurrentPassword(e.target.value)}
                placeholder="Current password"
              />
            </Field>
            <Field label="new">
              <Input
                type="password"
                value={newPassword}
                onChange={(e) => setNewPassword(e.target.value)}
                placeholder="New password"
              />
            </Field>
            <Field label="confirm">
              <Input
                type="password"
                value={confirmPassword}
                onChange={(e) => setConfirmPassword(e.target.value)}
                placeholder="Confirm password"
              />
            </Field>
          </div>
          <Button
            onClick={() => changePasswordMutation.mutate()}
            disabled={
              changePasswordMutation.isPending ||
              !currentPassword ||
              !newPassword ||
              !confirmPassword
            }
            variant="outline"
            size="sm"
          >
            {changePasswordMutation.isPending ? "changing..." : "change password"}
          </Button>
        </div>
      ) : (
        <div className="rounded-[12px] border border-border bg-card p-6 text-sm text-muted-foreground">
          Password management appears once an email-based login is attached to this account.
        </div>
      )}

      <div className="grid gap-4 sm:grid-cols-2">
        <ActionCard
          title="revoke other sessions"
          body="End every other active session while keeping this one open."
          actionLabel={revokeSessionsMutation.isPending ? "revoking..." : "revoke sessions"}
          onClick={() => revokeSessionsMutation.mutate()}
          disabled={revokeSessionsMutation.isPending}
        />
        <ActionCard
          title="sign out"
          body="Disconnect this session and return to the public landing page."
          actionLabel={signOutMutation.isPending ? "signing out..." : "sign out"}
          onClick={() => signOutMutation.mutate()}
          disabled={signOutMutation.isPending}
        />
      </div>
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="space-y-2">
      <div className="text-[11px] font-bold uppercase tracking-wider text-muted-foreground">
        {label}
      </div>
      {children}
    </div>
  );
}

function ActionCard({
  title,
  body,
  actionLabel,
  onClick,
  disabled,
}: {
  title: string;
  body: string;
  actionLabel: string;
  onClick: () => void;
  disabled: boolean;
}) {
  return (
    <div className="rounded-[12px] border border-border bg-card p-5 space-y-3">
      <div className="space-y-1">
        <div className="font-medium text-foreground">{title}</div>
        <p className="text-sm text-muted-foreground leading-relaxed">{body}</p>
      </div>
      <Button onClick={onClick} disabled={disabled} variant="outline" size="sm">
        {actionLabel}
      </Button>
    </div>
  );
}
