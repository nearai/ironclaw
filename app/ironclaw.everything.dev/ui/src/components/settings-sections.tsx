import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { toast } from "sonner";
import { type Passkey, useAuthClient } from "@/app";
import {
  ApiKeyForm,
  type ApiKeyFormValues,
  ApiKeyReveal,
  type ApiKeyRevealProps,
  Badge,
  Button,
  Card,
  CardContent,
  ConfirmDialog,
} from "@/components";
import { Input } from "@/components/ui/input";

type CreatedApiKey = ApiKeyRevealProps["apiKey"];

type ApiKeyItem = {
  id: string;
  name: string | null;
  prefix: string | null;
  start: string | null;
  enabled?: boolean;
  createdAt: string | Date;
  expiresAt?: string | Date | null;
  lastRequest?: string | Date | null;
};

export function useUserPasskeys(enabled = true) {
  const auth = useAuthClient();
  const passkeyQueryKey = ["passkeys"] as const;

  return useQuery({
    queryKey: passkeyQueryKey,
    queryFn: async () => {
      const { data } = await auth.passkey.listUserPasskeys();
      return (data || []) as Passkey[];
    },
    enabled,
    staleTime: 60 * 1000,
  });
}

export function AuthMethodsPanel({
  user,
  passkeys,
  nearAccountId,
}: {
  user: {
    id: string;
    email?: string;
    phoneNumber?: string | null;
    phoneNumberVerified?: boolean | null;
    isAnonymous?: boolean | null;
  };
  passkeys: Array<{ id: string; name?: string }>;
  nearAccountId: string | null;
}) {
  const auth = useAuthClient();
  const queryClient = useQueryClient();
  const [passkeyName, setPasskeyName] = useState("");
  const [passkeyToDelete, setPasskeyToDelete] = useState<{ id: string; name?: string } | null>(
    null,
  );
  const passkeyQueryKey = ["passkeys"] as const;
  const phoneNumber = user.phoneNumber ?? null;

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
    <>
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <MethodCard title="email" status={user.email ? "linked" : "missing"}>
          <p className={`text-sm text-muted-foreground${user.email ? " break-all" : ""}`}>
            {user.email ?? "Email login has not been linked for this account yet."}
          </p>
        </MethodCard>

        <MethodCard
          title="phone"
          status={phoneNumber ? (user.phoneNumberVerified ? "verified" : "linked") : "available"}
        >
          <p className="text-sm text-muted-foreground">
            {phoneNumber ? phoneNumber : "Phone OTP sign-in is available from the login screen."}
          </p>
        </MethodCard>

        <MethodCard title="near" status={nearAccountId ? "linked" : "missing"}>
          {nearAccountId ? (
            <div className="border-2 border-outset border-[rgb(51,51,51)] dark:border-[rgb(100,100,100)] bg-muted/30 p-3 font-mono text-xs break-all">
              {nearAccountId}
            </div>
          ) : (
            <Button
              onClick={() => linkNearMutation.mutate()}
              disabled={linkNearMutation.isPending}
              variant="outline"
              size="sm"
            >
              {linkNearMutation.isPending ? "redirecting..." : "link NEAR wallet"}
            </Button>
          )}
        </MethodCard>

        <MethodCard
          title="passkeys"
          status={passkeys.length > 0 ? `${passkeys.length} linked` : "missing"}
        >
          <div className="space-y-2">
            {passkeys.length > 0 ? (
              passkeys.map((passkey) => (
                <div
                  key={passkey.id}
                  className="border-2 border-outset border-[rgb(51,51,51)] dark:border-[rgb(100,100,100)] bg-muted/30 p-3 flex items-center justify-between gap-3"
                >
                  <span className="text-sm truncate min-w-0 flex-1">
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
              ))
            ) : (
              <p className="text-sm text-muted-foreground">No passkeys registered yet.</p>
            )}
            <Input
              type="text"
              value={passkeyName}
              onChange={(event) => setPasskeyName(event.target.value)}
              placeholder="Passkey name, e.g. Work laptop"
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
        </MethodCard>
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
          if (passkeyToDelete) {
            removePasskeyMutation.mutate(passkeyToDelete.id);
          }
        }}
        isPending={removePasskeyMutation.isPending}
      />
    </>
  );
}

export function UserApiKeysPanel() {
  const auth = useAuthClient();
  const queryClient = useQueryClient();
  const [createdApiKey, setCreatedApiKey] = useState<CreatedApiKey | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [confirmConfig, setConfirmConfig] = useState<{
    title: string;
    description: string;
    onConfirm: () => void;
  }>({ title: "", description: "", onConfirm: () => {} });

  const { data: apiKeys = [], isLoading } = useQuery({
    queryKey: ["user-api-keys"],
    queryFn: async (): Promise<ApiKeyItem[]> => {
      const { data, error } = await auth.apiKey.list({
        query: { configId: "user-keys" },
      });
      if (error) throw new Error(error.message);
      return (data?.apiKeys ?? []) as ApiKeyItem[];
    },
  });

  const createMutation = useMutation({
    mutationFn: async (values: ApiKeyFormValues) => {
      const { data, error } = await auth.apiKey.create({
        configId: "user-keys",
        name: values.name,
        ...(values.expiresIn !== undefined ? { expiresIn: values.expiresIn } : {}),
      });
      if (error) throw new Error(error.message);
      return data;
    },
    onSuccess: (data) => {
      if (data) setCreatedApiKey(data as CreatedApiKey);
      toast.success("API key created");
      queryClient.invalidateQueries({ queryKey: ["user-api-keys"] });
    },
    onError: (error: Error) => toast.error(error.message || "Failed to create API key"),
  });

  const deleteMutation = useMutation({
    mutationFn: async (keyId: string) => {
      const { error } = await auth.apiKey.delete({ keyId, configId: "user-keys" });
      if (error) throw new Error(error.message);
    },
    onSuccess: () => {
      setConfirmOpen(false);
      toast.success("API key deleted");
      queryClient.invalidateQueries({ queryKey: ["user-api-keys"] });
    },
    onError: (error: Error) => toast.error(error.message || "Failed to delete API key"),
  });

  const handleDelete = (keyId: string, keyName: string | null) => {
    setConfirmConfig({
      title: "Delete API key",
      description: `Permanently revoke ${keyName ?? "this key"}. Any service using it will stop working.`,
      onConfirm: () => {
        deleteMutation.mutate(keyId);
      },
    });
    setConfirmOpen(true);
  };

  return (
    <div className="space-y-6">
      <Card>
        <CardContent className="p-6">
          <ApiKeyForm
            onCreate={(values: ApiKeyFormValues) => createMutation.mutate(values)}
            isPending={createMutation.isPending}
          />
        </CardContent>
      </Card>

      {createdApiKey && (
        <ApiKeyReveal apiKey={createdApiKey} onDismiss={() => setCreatedApiKey(null)} />
      )}

      {isLoading ? (
        <Card>
          <CardContent className="p-8 text-center text-sm text-muted-foreground">
            Loading API keys...
          </CardContent>
        </Card>
      ) : apiKeys.length > 0 ? (
        <div className="grid gap-4 md:grid-cols-2">
          {apiKeys.map((key) => (
            <Card key={key.id}>
              <CardContent className="p-5 space-y-3">
                <div className="flex items-start justify-between gap-3">
                  <div className="space-y-1 min-w-0">
                    <div className="font-medium break-all">{key.name ?? "unnamed"}</div>
                    <div className="text-xs text-muted-foreground font-mono break-all">
                      {key.prefix ?? "api_"}...{key.start ?? ""}
                    </div>
                  </div>
                  {key.enabled === false && <Badge variant="outline">disabled</Badge>}
                </div>
                <div className="grid gap-1 text-xs text-muted-foreground">
                  <div>created {new Date(key.createdAt).toLocaleString()}</div>
                  {key.expiresAt && <div>expires {new Date(key.expiresAt).toLocaleString()}</div>}
                  {key.lastRequest && (
                    <div>last used {new Date(key.lastRequest).toLocaleString()}</div>
                  )}
                </div>
                <Button
                  onClick={() => handleDelete(key.id, key.name)}
                  disabled={deleteMutation.isPending}
                  variant="outline"
                  size="sm"
                  className="text-destructive hover:text-destructive"
                >
                  delete
                </Button>
              </CardContent>
            </Card>
          ))}
        </div>
      ) : (
        <Card>
          <CardContent className="p-8 text-center text-sm text-muted-foreground">
            No personal API keys
          </CardContent>
        </Card>
      )}

      <ConfirmDialog
        open={confirmOpen}
        onOpenChange={setConfirmOpen}
        title={confirmConfig.title}
        description={confirmConfig.description}
        variant="destructive"
        onConfirm={confirmConfig.onConfirm}
        isPending={deleteMutation.isPending}
      />
    </div>
  );
}

function MethodCard({
  title,
  status,
  children,
}: {
  title: string;
  status: string;
  children: React.ReactNode;
}) {
  return (
    <Card className="min-w-0 overflow-hidden">
      <CardContent className="p-5 space-y-4">
        <div className="flex items-center justify-between gap-3">
          <div className="font-medium">{title}</div>
          <Badge variant="outline">{status}</Badge>
        </div>
        {children}
      </CardContent>
    </Card>
  );
}
