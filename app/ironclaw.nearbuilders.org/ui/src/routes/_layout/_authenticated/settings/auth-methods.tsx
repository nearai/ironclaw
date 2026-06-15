import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { createFileRoute } from "@tanstack/react-router";
import { KeyRound, Mail, Smartphone, Wallet } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { type SessionData, sessionQueryOptions, useAuthClient } from "@/app";
import { Button, ConfirmDialog } from "@/components";
import { useUserPasskeys } from "@/components/settings-sections";
import { Input } from "@/components/ui/input";

export const Route = createFileRoute("/_layout/_authenticated/settings/auth-methods")({
  loader: async ({ context }) => {
    await context.queryClient.ensureQueryData({
      queryKey: ["passkeys"],
      queryFn: async () => {
        const { data } = await context.authClient.passkey.listUserPasskeys();
        return (data || []) as any[];
      },
      staleTime: 60 * 1000,
    });
  },
  component: AuthMethodsSettings,
});

function AuthMethodsSettings() {
  const auth = useAuthClient();
  const { data: session } = useQuery<SessionData | null>(sessionQueryOptions(auth));
  const user = session?.user;
  const { data: passkeys = [] } = useUserPasskeys(!!user);
  const nearAccountId = auth.near.getAccountId();

  if (!user) return null;

  return (
    <div className="space-y-4">
      <EmailMethod user={user} />
      <NearMethod nearAccountId={nearAccountId} />
      <PhoneMethod user={user} />
      <PasskeysMethod passkeys={passkeys} />
    </div>
  );
}

function EmailMethod({ user }: { user: { email?: string; isAnonymous?: boolean | null } }) {
  return (
    <div className="rounded-[12px] border border-border bg-card p-6 space-y-4">
      <div className="flex items-start gap-4">
        <div className="w-10 h-10 rounded-[10px] border border-border bg-muted flex items-center justify-center shrink-0">
          <Mail className="h-4 w-4 text-muted-foreground" />
        </div>
        <div className="min-w-0 flex-1 space-y-1">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="text-base font-semibold text-foreground">Email</span>
            <StatusChip linked={!!user.email} />
          </div>
          <p className="text-sm text-muted-foreground">
            {user.email ?? "Email login has not been linked for this account yet."}
          </p>
        </div>
      </div>
    </div>
  );
}

function NearMethod({ nearAccountId }: { nearAccountId: string | null }) {
  const auth = useAuthClient();
  const queryClient = useQueryClient();

  const linkNearMutation = useMutation({
    mutationFn: async () => {
      const result: unknown = await auth.signIn.near();
      if (
        result &&
        typeof result === "object" &&
        "error" in result &&
        result.error &&
        typeof result.error === "object" &&
        "message" in result.error
      ) {
        throw new Error(String(result.error.message));
      }
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["session"] });
      queryClient.invalidateQueries({ queryKey: ["near-accounts"] });
    },
    onError: (err: Error) => toast.error(err.message),
  });

  return (
    <div className="rounded-[12px] border border-border bg-card p-6 space-y-4">
      <div className="flex items-start gap-4">
        <div className="w-10 h-10 rounded-[10px] border border-border bg-muted flex items-center justify-center shrink-0">
          <Wallet className="h-4 w-4 text-muted-foreground" />
        </div>
        <div className="min-w-0 flex-1 space-y-1">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="text-base font-semibold text-foreground">NEAR Wallet</span>
            <StatusChip linked={!!nearAccountId} />
          </div>
          {nearAccountId ? (
            <div className="rounded-[8px] border border-border bg-muted px-3 py-2 font-mono text-xs break-all text-foreground">
              {nearAccountId}
            </div>
          ) : (
            <div className="flex items-center gap-3 pt-1">
              <button
                type="button"
                onClick={() => linkNearMutation.mutate()}
                disabled={linkNearMutation.isPending}
                className="h-9 px-4 inline-flex items-center justify-center gap-2 text-sm font-medium border-2 border-outset border-border-strong bg-card text-foreground shadow-sm hover:shadow-md hover:bg-muted active:border-inset active:shadow-none transition-all duration-200 ease-out disabled:pointer-events-none disabled:opacity-50 rounded-[12px]"
              >
                {linkNearMutation.isPending ? "connecting..." : "connect NEAR wallet"}
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function PhoneMethod({
  user,
}: {
  user: { phoneNumber?: string | null; phoneNumberVerified?: boolean | null };
}) {
  const phoneNumber = user.phoneNumber ?? null;

  return (
    <div className="rounded-[12px] border border-border bg-card p-6 space-y-4">
      <div className="flex items-start gap-4">
        <div className="w-10 h-10 rounded-[10px] border border-border bg-muted flex items-center justify-center shrink-0">
          <Smartphone className="h-4 w-4 text-muted-foreground" />
        </div>
        <div className="min-w-0 flex-1 space-y-1">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="text-base font-semibold text-foreground">Phone</span>
            <StatusChip
              linked={!!phoneNumber}
              label={phoneNumber && !user.phoneNumberVerified ? "unverified" : undefined}
            />
          </div>
          <p className="text-sm text-muted-foreground">
            {phoneNumber ?? "Phone OTP sign-in is available from the login screen."}
          </p>
        </div>
      </div>
    </div>
  );
}

function PasskeysMethod({ passkeys }: { passkeys: Array<{ id: string; name?: string }> }) {
  const auth = useAuthClient();
  const queryClient = useQueryClient();
  const [passkeyName, setPasskeyName] = useState("");
  const [passkeyToDelete, setPasskeyToDelete] = useState<{ id: string; name?: string } | null>(
    null,
  );
  const passkeyQueryKey = ["passkeys"] as const;

  const addPasskeyMutation = useMutation({
    mutationFn: async () => {
      const name = passkeyName.trim();
      const { error } = name
        ? await auth.passkey.addPasskey({ name })
        : await auth.passkey.addPasskey();
      if (error) throw new Error(error.message);
    },
    onSuccess: () => {
      setPasskeyName("");
      toast.success("Passkey added");
      queryClient.invalidateQueries({ queryKey: passkeyQueryKey });
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const removePasskeyMutation = useMutation({
    mutationFn: async (passkeyId: string) => {
      const { error } = await auth.passkey.deletePasskey({ id: passkeyId });
      if (error) throw new Error(error.message);
    },
    onSuccess: () => {
      setPasskeyToDelete(null);
      toast.success("Passkey removed");
      queryClient.invalidateQueries({ queryKey: passkeyQueryKey });
    },
    onError: (err: Error) => toast.error(err.message),
  });

  return (
    <>
      <div className="rounded-[12px] border border-border bg-card p-6 space-y-4">
        <div className="flex items-start gap-4">
          <div className="w-10 h-10 rounded-[10px] border border-border bg-muted flex items-center justify-center shrink-0">
            <KeyRound className="h-4 w-4 text-muted-foreground" />
          </div>
          <div className="min-w-0 flex-1 space-y-3">
            <div className="flex items-center gap-2 flex-wrap">
              <span className="text-base font-semibold text-foreground">Passkeys</span>
              <StatusChip
                linked={passkeys.length > 0}
                label={passkeys.length > 0 ? `${passkeys.length} registered` : undefined}
              />
            </div>

            {passkeys.length > 0 && (
              <div className="flex flex-col gap-2">
                {passkeys.map((passkey) => (
                  <div
                    key={passkey.id}
                    className="flex items-center justify-between gap-3 rounded-[8px] border border-border bg-muted px-3.5 py-2.5"
                  >
                    <span className="text-sm text-foreground truncate min-w-0 flex-1">
                      {passkey.name || "Passkey"}
                    </span>
                    <Button
                      onClick={() => setPasskeyToDelete(passkey)}
                      disabled={removePasskeyMutation.isPending}
                      variant="outline"
                      size="sm"
                    >
                      remove
                    </Button>
                  </div>
                ))}
              </div>
            )}

            <div className="flex gap-2">
              <Input
                type="text"
                value={passkeyName}
                onChange={(e) => setPasskeyName(e.target.value)}
                placeholder="Passkey name, e.g. Work laptop"
                className="max-w-xs"
              />
              <Button
                onClick={() => addPasskeyMutation.mutate()}
                disabled={addPasskeyMutation.isPending}
                variant="outline"
                size="sm"
              >
                {addPasskeyMutation.isPending ? "adding..." : "add passkey"}
              </Button>
            </div>
          </div>
        </div>
      </div>

      <ConfirmDialog
        open={!!passkeyToDelete}
        onOpenChange={(open: boolean) => {
          if (!open) setPasskeyToDelete(null);
        }}
        title="Remove passkey"
        description={`Remove ${passkeyToDelete?.name || "this passkey"} from your account? You will no longer be able to use it to sign in.`}
        confirmLabel="remove"
        variant="destructive"
        onConfirm={() => {
          if (passkeyToDelete) removePasskeyMutation.mutate(passkeyToDelete.id);
        }}
        isPending={removePasskeyMutation.isPending}
      />
    </>
  );
}

function StatusChip({ linked, label }: { linked: boolean; label?: string }) {
  return (
    <span
      className={`inline-flex items-center rounded-[6px] px-2 py-0.5 text-[10px] font-semibold border ${
        linked
          ? "bg-secondary border-border text-foreground"
          : "bg-muted border-border text-muted-foreground"
      }`}
    >
      {label ?? (linked ? "linked" : "not linked")}
    </span>
  );
}
