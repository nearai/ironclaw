import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { createFileRoute, Link, useRouter } from "@tanstack/react-router";
import {
  Cloud,
  Edit2,
  Key,
  Loader2,
  LogOut,
  Mail,
  Save,
  Terminal,
  Trash2,
  Users,
} from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import {
  type Organization,
  type SessionData,
  sessionQueryOptions,
  useApiClient,
  useAuthClient,
} from "@/app";
import {
  ApiKeyForm,
  type ApiKeyFormValues,
  ApiKeyReveal,
  type ApiKeyRevealProps,
  Button,
  Card,
  Input,
  InvitationCard,
  Label,
  MemberCard,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "@/components";

type AuthClientType = import("@/app").AuthClient;

type MembersResponse = Awaited<ReturnType<AuthClientType["organization"]["listMembers"]>>;
type MemberItem = NonNullable<MembersResponse["data"]>["members"][number];

type InvitationsResponse = Awaited<ReturnType<AuthClientType["organization"]["listInvitations"]>>;
type InvitationItem = NonNullable<InvitationsResponse["data"]>[number];

type ApiKeyItem = {
  id: string;
  name: string | null;
  prefix: string | null;
  start: string | null;
  createdAt: string | Date;
  expiresAt?: string | Date | null;
  metadata?: Record<string, unknown> | null;
};

type CreatedApiKey = ApiKeyRevealProps["apiKey"];

const orgMembersQueryKey = (orgId: string) => ["org-members", orgId] as const;
const orgInvitationsQueryKey = (orgId: string) => ["org-invitations", orgId] as const;
const orgApiKeysQueryKey = (orgId: string) => ["org-api-keys", orgId] as const;

export const Route = createFileRoute("/_layout/_authenticated/organizations/$slug")({
  head: () => ({
    title: "Organization | auth.everything.dev",
    meta: [{ name: "description", content: "Manage organization details and members." }],
  }),
  loader: async ({ context }) => {
    await context.queryClient.ensureQueryData(
      sessionQueryOptions(context.authClient, context.session),
    );
    await context.queryClient.ensureQueryData({
      queryKey: ["organizations"],
      queryFn: async () => {
        const { data } = await context.authClient.organization.list();
        return (data || []) as Organization[];
      },
      staleTime: 30 * 1000,
    });
  },
  component: OrganizationDetail,
});

function OrganizationDetail() {
  const queryClient = useQueryClient();
  const router = useRouter();
  const { slug: orgSlug } = Route.useParams();
  const auth = useAuthClient();
  const apiClient = useApiClient();

  const { data: session } = useQuery<SessionData | null>({
    queryKey: ["session"],
    queryFn: async () => {
      const { data } = await auth.getSession();
      return data ?? null;
    },
    staleTime: 60 * 1000,
  });

  const { data: organizations = [], isLoading: isLoadingOrgs } = useQuery({
    queryKey: ["organizations"],
    queryFn: async () => {
      const { data } = await auth.organization.list();
      return (data || []) as Organization[];
    },
    staleTime: 30 * 1000,
  });

  const org = organizations.find((o: Organization) => o.slug === orgSlug);
  const orgId = org?.id ?? "";
  const activeOrgId = session?.session?.activeOrganizationId;
  const isActive = orgId === activeOrgId;

  const members =
    useQuery({
      queryKey: orgMembersQueryKey(orgId),
      queryFn: async (): Promise<MemberItem[]> => {
        const { data, error } = await auth.organization.listMembers({
          query: { organizationId: orgId },
        });
        if (error) throw new Error(error.message);
        return (data?.members ?? []) as MemberItem[];
      },
      enabled: !!orgId,
    }).data ?? [];

  const invitations =
    useQuery({
      queryKey: orgInvitationsQueryKey(orgId),
      queryFn: async (): Promise<InvitationItem[]> => {
        const { data, error } = await auth.organization.listInvitations({
          query: { organizationId: orgId },
        });
        if (error) throw new Error(error.message);
        return (data ?? []) as InvitationItem[];
      },
      enabled: !!orgId,
    }).data ?? [];

  const apiKeys =
    useQuery({
      queryKey: orgApiKeysQueryKey(orgId),
      queryFn: async (): Promise<ApiKeyItem[]> => {
        const { data, error } = await auth.apiKey.list({
          query: { configId: "org-keys", organizationId: orgId },
        });
        if (error) throw new Error(error.message);
        return (data?.apiKeys ?? []) as ApiKeyItem[];
      },
      enabled: !!orgId,
    }).data ?? [];

  const myMembership = members.find((m) => m.userId === session?.user?.id);
  const canManageMembers = myMembership?.role === "owner" || myMembership?.role === "admin";
  const isOwner = myMembership?.role === "owner";

  const pendingInvitationsCount = invitations.filter((i) => i.status === "pending").length;

  const [inviteEmail, setInviteEmail] = useState("");
  const [inviteRole, setInviteRole] = useState<"admin" | "member">("member");
  const [createdApiKey, setCreatedApiKey] = useState<CreatedApiKey | null>(null);

  const [baseUrl, setBaseUrl] = useState("");
  const [apiToken, setApiToken] = useState("");
  const [tokenConfigured, setTokenConfigured] = useState(false);
  const [icLoading, setIcLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [disconnecting, setDisconnecting] = useState(false);
  const [hasSettings, setHasSettings] = useState(false);

  useEffect(() => {
    apiClient.ironclaw.settings
      .get({ scope: "organization" })
      .then((res) => {
        setBaseUrl(res.baseUrl);
        setTokenConfigured(res.hasToken ?? false);
        setHasSettings(true);
      })
      .catch(() => {
        setHasSettings(false);
      })
      .finally(() => setIcLoading(false));
  }, [apiClient]);

  const handleCopyApiKey = async (value: string, message = "API key copied") => {
    try {
      await navigator.clipboard.writeText(value);
      toast.success(message);
    } catch {
      toast.error("Failed to copy API key");
    }
  };

  const switchOrgMutation = useMutation({
    mutationFn: async () => {
      const { error } = await auth.organization.setActive({ organizationId: orgId });
      if (error) throw new Error(error.message);
    },
    onSuccess: async () => {
      toast.success("Switched to this organization");
      await queryClient.invalidateQueries({ queryKey: ["session"] });
    },
    onError: (error: Error) => toast.error(error.message || "Failed to switch organization"),
  });

  const inviteMutation = useMutation({
    mutationFn: async () => {
      const { error } = await auth.organization.inviteMember({
        organizationId: orgId,
        email: inviteEmail,
        role: inviteRole,
      });
      if (error) throw new Error(error.message);
    },
    onSuccess: async () => {
      toast.success(`Invitation sent to ${inviteEmail}`);
      setInviteEmail("");
      await queryClient.invalidateQueries({ queryKey: orgInvitationsQueryKey(orgId) });
    },
    onError: (error: Error) => {
      toast.error(error.message || "Failed to send invitation");
    },
  });

  const cancelInvitationMutation = useMutation({
    mutationFn: async (invitationId: string) => {
      const { error } = await auth.organization.cancelInvitation({ invitationId });
      if (error) throw new Error(error.message);
    },
    onSuccess: async () => {
      toast.success("Invitation cancelled");
      await queryClient.invalidateQueries({ queryKey: orgInvitationsQueryKey(orgId) });
    },
    onError: (error: Error) => {
      toast.error(error.message || "Failed to cancel invitation");
    },
  });

  const resendInvitationMutation = useMutation({
    mutationFn: async (invitation: InvitationItem) => {
      const { error } = await auth.organization.inviteMember({
        organizationId: orgId,
        email: invitation.email,
        role: invitation.role as "admin" | "member" | "owner",
        resend: true,
      });
      if (error) throw new Error(error.message);
    },
    onSuccess: async () => {
      toast.success("Invitation resent");
      await queryClient.invalidateQueries({ queryKey: orgInvitationsQueryKey(orgId) });
    },
    onError: (error: Error) => {
      toast.error(error.message || "Failed to resend invitation");
    },
  });

  const createApiKeyMutation = useMutation({
    mutationFn: async (values: ApiKeyFormValues) => {
      const { data, error } = await auth.apiKey.create({
        configId: "org-keys",
        organizationId: orgId,
        name: values.name,
        ...(values.expiresIn !== undefined ? { expiresIn: values.expiresIn } : {}),
      });
      if (error) throw new Error(error.message);
      return data;
    },
    onSuccess: async (data) => {
      if (data) setCreatedApiKey(data as CreatedApiKey);
      toast.success("API key created");
      await queryClient.invalidateQueries({ queryKey: orgApiKeysQueryKey(orgId) });
    },
    onError: (error: Error) => {
      toast.error(error.message || "Failed to create API key");
    },
  });

  const deleteApiKeyMutation = useMutation({
    mutationFn: async (keyId: string) => {
      const { error } = await auth.apiKey.delete({ keyId, configId: "org-keys" });
      if (error) throw new Error(error.message);
    },
    onMutate: async (keyId) => {
      await queryClient.cancelQueries({ queryKey: orgApiKeysQueryKey(orgId) });
      const previousKeys = queryClient.getQueryData<ApiKeyItem[]>(orgApiKeysQueryKey(orgId));
      queryClient.setQueryData<ApiKeyItem[]>(orgApiKeysQueryKey(orgId), (current) => {
        if (!current) return current;
        return current.filter((key) => key.id !== keyId);
      });
      return { previousKeys };
    },
    onSuccess: async () => {
      toast.success("API key deleted");
      await queryClient.invalidateQueries({ queryKey: orgApiKeysQueryKey(orgId) });
    },
    onError: (error: Error, _keyId, context) => {
      if (context?.previousKeys) {
        queryClient.setQueryData(orgApiKeysQueryKey(orgId), context.previousKeys);
      }
      toast.error(error.message || "Failed to delete API key");
    },
  });

  const removeMemberMutation = useMutation({
    mutationFn: async (member: MemberItem) => {
      const memberIdOrEmail = member.user?.email ?? member.userId;
      const { error } = await auth.organization.removeMember({
        memberIdOrEmail,
        organizationId: orgId,
      });
      if (error) throw new Error(error.message);
    },
    onSuccess: async () => {
      toast.success("Member removed");
      await queryClient.invalidateQueries({ queryKey: orgMembersQueryKey(orgId) });
    },
    onError: (error: Error) => {
      toast.error(error.message || "Failed to remove member");
    },
  });

  const updateOrgMutation = useMutation({
    mutationFn: async ({ name, slug }: { name: string; slug: string }) => {
      const { error } = await auth.organization.update({
        organizationId: orgId,
        data: { name, slug },
      });
      if (error) throw new Error(error.message);
    },
    onSuccess: async () => {
      toast.success("Organization updated");
      await queryClient.invalidateQueries({ queryKey: ["organizations"] });
      await queryClient.invalidateQueries({ queryKey: orgMembersQueryKey(orgId) });
      setIsEditing(false);
    },
    onError: (error: Error) => {
      toast.error(error.message || "Failed to update organization");
    },
  });

  const isPersonal = session?.user
    ? org?.slug === session.user.id ||
      (org?.metadata as { isPersonal?: boolean } | null | undefined)?.isPersonal === true
    : false;

  const leaveOrgMutation = useMutation({
    mutationFn: async () => {
      const { error } = await auth.organization.leave({ organizationId: orgId });
      if (error) throw new Error(error.message);
    },
    onSuccess: async () => {
      toast.success("You have left the organization");
      await queryClient.invalidateQueries({ queryKey: ["organizations"] });
      await router.navigate({ to: "/organizations" });
    },
    onError: (error: Error) => toast.error(error.message || "Failed to leave organization"),
  });

  const deleteOrgMutation = useMutation({
    mutationFn: async () => {
      const { error } = await auth.organization.delete({ organizationId: orgId });
      if (error) throw new Error(error.message);
    },
    onSuccess: async () => {
      toast.success("Organization deleted");
      await queryClient.invalidateQueries({ queryKey: ["organizations"] });
      await router.navigate({ to: "/organizations" });
    },
    onError: (error: Error) => toast.error(error.message || "Failed to delete organization"),
  });

  const [isEditing, setIsEditing] = useState(false);
  const [editName, setEditName] = useState("");
  const [editSlug, setEditSlug] = useState("");

  const handleIronclawSave = async (e: React.FormEvent) => {
    e.preventDefault();
    setSaving(true);
    try {
      await apiClient.ironclaw.settings.update({
        baseUrl,
        ...(apiToken ? { apiToken } : {}),
        scope: "organization",
      });
      setHasSettings(true);
      if (apiToken) setTokenConfigured(true);
      toast.success("IronClaw settings saved");
    } catch (err: any) {
      toast.error(err.message ?? "Failed to save settings");
    } finally {
      setSaving(false);
    }
  };

  const handleIronclawDisconnect = async () => {
    setDisconnecting(true);
    try {
      await apiClient.ironclaw.settings.delete({ scope: "organization" });
      setBaseUrl("");
      setApiToken("");
      setTokenConfigured(false);
      setHasSettings(false);
      toast.success("Disconnected from tunnel");
    } catch (err: any) {
      toast.error(err.message ?? "Failed to disconnect");
    } finally {
      setDisconnecting(false);
    }
  };

  if (isLoadingOrgs) {
    return (
      <div className="flex h-full flex-col overflow-hidden">
        <div className="flex shrink-0 items-center justify-between gap-3 border-b border-border bg-card px-4 py-2.5 sm:px-6 sm:py-3">
          <div className="h-6 w-32 animate-pulse rounded-[6px] bg-muted" />
        </div>
        <div className="flex-1 flex items-center justify-center">
          <p className="text-sm text-muted-foreground">Loading organization...</p>
        </div>
      </div>
    );
  }

  if (!org) {
    return (
      <div className="flex h-full flex-col overflow-hidden">
        <div className="flex shrink-0 items-center justify-between gap-3 border-b border-border bg-card px-4 py-2.5 sm:px-6 sm:py-3">
          <h1 className="text-xl font-semibold text-foreground">Organization</h1>
          <Button asChild variant="outline" size="sm">
            <Link to="/organizations">back</Link>
          </Button>
        </div>
        <div className="flex-1 flex items-center justify-center p-8">
          <div className="rounded-[12px] border border-border bg-card p-8 text-center space-y-3 max-w-sm w-full">
            <p className="text-sm text-foreground">
              This organization does not exist or you do not have access.
            </p>
            <Button asChild variant="outline" size="sm">
              <Link to="/organizations">back to organizations</Link>
            </Button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <div className="flex shrink-0 items-center justify-between gap-3 border-b border-border bg-card px-4 py-2.5 sm:px-6 sm:py-3">
        <div className="flex items-center gap-2 text-xs font-mono text-muted-foreground min-w-0">
          <Link to="/organizations" className="hover:text-foreground transition-colors shrink-0">
            organizations
          </Link>
          <span>/</span>
          <span className="text-foreground truncate">{org.slug}</span>
        </div>
        <Button asChild variant="outline" size="sm">
          <Link to="/organizations">back</Link>
        </Button>
      </div>

      <div className="flex-1 overflow-y-auto px-4 py-6 sm:px-6">
        <div className="mx-auto max-w-3xl space-y-6">
          <div className="rounded-[12px] border border-border bg-card p-6">
            <div className="flex flex-wrap items-center gap-2 mb-4">
              <Chip>organization</Chip>
              {isActive && <Chip accent>active</Chip>}
              {isPersonal && <Chip>personal</Chip>}
            </div>
            <h2 className="text-foreground text-2xl font-semibold mb-1">{org.name}</h2>
            <p className="text-muted-foreground text-sm font-mono mb-1">@{org.slug}</p>
            <p className="text-muted-foreground text-sm leading-relaxed mb-4">
              Manage membership, invitations, and organization-scoped API access.
            </p>
            <div className="flex flex-col gap-2 mb-4">
              <InfoRow label="members" value={String(members.length)} />
              <InfoRow label="invites" value={String(pendingInvitationsCount)} />
              <InfoRow label="api keys" value={String(apiKeys.length)} />
              {org.createdAt && (
                <InfoRow label="created" value={new Date(org.createdAt).toLocaleDateString()} />
              )}
            </div>
            <div className="flex flex-wrap gap-2">
              {!isActive && (
                <Button
                  onClick={() => switchOrgMutation.mutate()}
                  disabled={switchOrgMutation.isPending}
                  size="sm"
                >
                  {switchOrgMutation.isPending ? "switching..." : "switch to org"}
                </Button>
              )}
              {isOwner && !isPersonal && (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    setEditName(org.name);
                    setEditSlug(org.slug);
                    setIsEditing(true);
                  }}
                >
                  <Edit2 className="h-3.5 w-3.5" />
                  edit
                </Button>
              )}
              {!isPersonal && !isOwner && (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    if (confirm(`Leave "${org.name}"?`)) {
                      leaveOrgMutation.mutate();
                    }
                  }}
                  disabled={leaveOrgMutation.isPending}
                >
                  <LogOut className="h-3.5 w-3.5" />
                  {leaveOrgMutation.isPending ? "leaving..." : "leave"}
                </Button>
              )}
              {isOwner && !isPersonal && (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    if (confirm(`Delete "${org.name}"? This cannot be undone.`)) {
                      deleteOrgMutation.mutate();
                    }
                  }}
                  disabled={deleteOrgMutation.isPending}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                  {deleteOrgMutation.isPending ? "deleting..." : "delete org"}
                </Button>
              )}
            </div>
          </div>

          {isEditing && isOwner && (
            <div className="rounded-[12px] border border-border bg-card p-6 space-y-4">
              <div className="text-[11px] font-bold uppercase tracking-wider text-muted-foreground">
                Edit Organization
              </div>
              <div className="grid gap-4 md:grid-cols-2">
                <Input
                  type="text"
                  value={editName}
                  onChange={(e) => setEditName(e.target.value)}
                  placeholder="Organization name"
                />
                <div className="flex items-center gap-2">
                  <span className="text-muted-foreground text-sm">@</span>
                  <Input
                    type="text"
                    value={editSlug}
                    onChange={(e) => setEditSlug(e.target.value.replace(/[^a-z0-9-]/g, ""))}
                    placeholder="slug"
                    pattern="[a-z0-9-]+"
                  />
                </div>
              </div>
              <div className="flex gap-2">
                <Button
                  onClick={() => updateOrgMutation.mutate({ name: editName, slug: editSlug })}
                  disabled={updateOrgMutation.isPending || !editName || !editSlug}
                  size="sm"
                >
                  {updateOrgMutation.isPending ? "saving..." : "save"}
                </Button>
                <Button onClick={() => setIsEditing(false)} variant="outline" size="sm">
                  cancel
                </Button>
              </div>
            </div>
          )}

          <Tabs defaultValue="members" className="w-full min-w-0">
            <TabsList className="w-full justify-start overflow-x-auto">
              <TabsTrigger value="members" className="shrink-0">
                <Users className="h-4 w-4 mr-1.5" />
                Members ({members.length})
              </TabsTrigger>
              <TabsTrigger value="invitations" className="shrink-0">
                <Mail className="h-4 w-4 mr-1.5" />
                Invitations ({pendingInvitationsCount})
              </TabsTrigger>
              <TabsTrigger value="apikeys" className="shrink-0">
                <Key className="h-4 w-4 mr-1.5" />
                API Keys ({apiKeys.length})
              </TabsTrigger>
              <TabsTrigger value="ironclaw" className="shrink-0">
                <Terminal className="h-4 w-4 mr-1.5" />
                IronClaw
              </TabsTrigger>
            </TabsList>

            <TabsContent value="members" className="space-y-6 pt-4">
              {members.length > 0 ? (
                <div className="grid gap-4 md:grid-cols-2">
                  {members.map((member) => (
                    <MemberCard
                      key={member.id}
                      member={member as never}
                      canManage={canManageMembers && member.userId !== session?.user?.id}
                      onRemove={() => removeMemberMutation.mutate(member)}
                      isRemoving={removeMemberMutation.isPending}
                    />
                  ))}
                </div>
              ) : (
                <EmptyState label="No members found" />
              )}
            </TabsContent>

            <TabsContent value="invitations" className="space-y-6 pt-4">
              {canManageMembers && !isPersonal && (
                <div className="rounded-[12px] border border-border bg-card p-6 space-y-4">
                  <div className="text-[11px] font-bold uppercase tracking-wider text-muted-foreground">
                    Invite member
                  </div>
                  <div className="grid gap-4 md:grid-cols-[1fr_180px]">
                    <Input
                      type="email"
                      value={inviteEmail}
                      onChange={(e) => setInviteEmail(e.target.value)}
                      placeholder="email@example.com"
                    />
                    <select
                      value={inviteRole}
                      onChange={(e) => setInviteRole(e.target.value as "admin" | "member")}
                      className="w-full px-3 py-2 text-sm bg-card text-foreground border-2 border-inset border-border-strong rounded-[8px] outline-none focus:ring-2 focus:ring-ring"
                    >
                      <option value="member">Member</option>
                      <option value="admin">Admin</option>
                    </select>
                  </div>
                  <Button
                    onClick={() => inviteMutation.mutate()}
                    disabled={inviteMutation.isPending || !inviteEmail}
                    variant="outline"
                    size="sm"
                  >
                    {inviteMutation.isPending ? "sending..." : "send invitation"}
                  </Button>
                </div>
              )}

              {(() => {
                const pendingInvitations = invitations.filter((i) => i.status === "pending");
                return pendingInvitations.length > 0 ? (
                  <div className="grid gap-4 md:grid-cols-2">
                    {pendingInvitations.map((invitation) => (
                      <InvitationCard
                        key={invitation.id}
                        invitation={invitation as never}
                        onCancel={
                          canManageMembers
                            ? () => cancelInvitationMutation.mutate(invitation.id)
                            : undefined
                        }
                        onResend={
                          canManageMembers
                            ? () => resendInvitationMutation.mutate(invitation)
                            : undefined
                        }
                        isCancelling={cancelInvitationMutation.isPending}
                        isResending={resendInvitationMutation.isPending}
                      />
                    ))}
                  </div>
                ) : (
                  <EmptyState label="No pending invitations" />
                );
              })()}
            </TabsContent>

            <TabsContent value="apikeys" className="space-y-6 pt-4">
              {canManageMembers && (
                <div className="rounded-[12px] border border-border bg-card p-6">
                  <ApiKeyForm
                    onCreate={(values: ApiKeyFormValues) => createApiKeyMutation.mutate(values)}
                    isPending={createApiKeyMutation.isPending}
                  />
                </div>
              )}

              {createdApiKey && (
                <ApiKeyReveal apiKey={createdApiKey} onDismiss={() => setCreatedApiKey(null)} />
              )}

              {apiKeys.length > 0 ? (
                <div className="grid gap-4 md:grid-cols-2">
                  {apiKeys.map((key) => (
                    <div
                      key={key.id}
                      className="rounded-[12px] border border-border bg-card p-5 space-y-3"
                    >
                      <div className="space-y-1 min-w-0">
                        <div className="font-medium text-foreground break-all">
                          {key.name ?? "unnamed"}
                        </div>
                        <div className="text-xs text-muted-foreground font-mono">
                          {key.prefix ?? "api_"}...{key.start ?? ""}
                        </div>
                      </div>
                      <div className="flex flex-col gap-1 text-xs text-muted-foreground">
                        <div>created {new Date(key.createdAt).toLocaleString()}</div>
                        {key.expiresAt && (
                          <div>expires {new Date(key.expiresAt).toLocaleString()}</div>
                        )}
                      </div>
                      <div className="flex gap-2">
                        <Button
                          onClick={() => handleCopyApiKey(key.start || "", "Key prefix copied")}
                          variant="outline"
                          size="sm"
                        >
                          copy id
                        </Button>
                        {canManageMembers && (
                          <Button
                            onClick={() => deleteApiKeyMutation.mutate(key.id)}
                            disabled={deleteApiKeyMutation.isPending}
                            variant="outline"
                            size="sm"
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                            delete
                          </Button>
                        )}
                      </div>
                    </div>
                  ))}
                </div>
              ) : (
                <EmptyState label="No API keys" />
              )}
            </TabsContent>

            <TabsContent value="ironclaw" className="space-y-6 pt-4">
              {icLoading ? (
                <div className="flex items-center justify-center py-12">
                  <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
                </div>
              ) : canManageMembers ? (
                <form onSubmit={handleIronclawSave} className="space-y-4">
                  <Card className="space-y-4 p-5">
                    <div className="space-y-2">
                      <Label htmlFor="org-tunnelUrl" className="flex items-center gap-1.5">
                        <Terminal size={14} />
                        Tunnel URL
                      </Label>
                      <Input
                        id="org-tunnelUrl"
                        value={baseUrl}
                        onChange={(e) => setBaseUrl(e.target.value)}
                        placeholder="https://your-tunnel.ngrok.io"
                        required
                      />
                      <p className="text-xs text-muted-foreground">
                        Public URL pointing to your ironclaw reborn binary. This tunnel applies to
                        all org members.
                      </p>
                    </div>

                    <div className="space-y-2">
                      <Label htmlFor="org-apiToken" className="flex items-center gap-1.5">
                        <Key size={14} />
                        API Token
                      </Label>
                      <Input
                        id="org-apiToken"
                        type="password"
                        value={apiToken}
                        onChange={(e) => setApiToken(e.target.value)}
                        placeholder={
                          tokenConfigured
                            ? "Token is configured"
                            : "The bearer token your binary expects"
                        }
                        required={!tokenConfigured}
                      />
                      <p className="text-xs text-muted-foreground">
                        {tokenConfigured
                          ? "Token is already configured. Leave empty to keep the existing token."
                          : "Must match the bearer token configured on your Reborn binary."}
                      </p>
                    </div>
                  </Card>

                  <div className="flex items-center justify-between gap-4">
                    {!hasSettings && (
                      <p className="text-xs text-muted-foreground">
                        No settings configured yet. Add your tunnel URL and API token to connect.
                      </p>
                    )}
                    <div className="flex items-center gap-2 ml-auto">
                      {hasSettings && (
                        <Button
                          type="button"
                          variant="outline"
                          disabled={disconnecting}
                          onClick={handleIronclawDisconnect}
                          className="text-destructive hover:text-destructive"
                        >
                          {disconnecting ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : (
                            <Cloud size={14} />
                          )}
                          {disconnecting ? "Disconnecting..." : "Disconnect"}
                        </Button>
                      )}
                      <Button type="submit" disabled={saving || !baseUrl}>
                        {saving ? <Loader2 className="h-4 w-4 animate-spin" /> : <Save size={14} />}
                        {saving ? "Saving..." : "Save settings"}
                      </Button>
                    </div>
                  </div>
                </form>
              ) : (
                <Card className="p-6 space-y-3">
                  <div className="flex items-start gap-3">
                    <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-secondary">
                      <Terminal size={14} className="text-muted-foreground" />
                    </div>
                    <div className="space-y-2">
                      <p className="text-sm font-semibold text-foreground">
                        Organization IronClaw Tunnel
                      </p>
                      <p className="text-sm text-muted-foreground">
                        Your organization has an IronClaw tunnel configured by an admin.
                      </p>
                      {baseUrl && (
                        <p className="text-xs text-muted-foreground truncate">
                          Tunnel: {baseUrl.replace(/\/\/([^/]+)/, "//***")}
                        </p>
                      )}
                    </div>
                  </div>
                </Card>
              )}
            </TabsContent>
          </Tabs>
        </div>
      </div>
    </div>
  );
}

function InfoRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid grid-cols-[100px_1fr] gap-4 rounded-[8px] border border-border bg-muted px-3.5 py-2.5 items-center">
      <span className="text-muted-foreground text-[11px] font-bold uppercase tracking-wider">
        {label}
      </span>
      <span className="text-foreground text-[13px] break-all">{value}</span>
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

function EmptyState({ label }: { label: string }) {
  return (
    <div className="rounded-[12px] border border-border bg-card p-8 text-center text-sm text-muted-foreground">
      {label}
    </div>
  );
}
