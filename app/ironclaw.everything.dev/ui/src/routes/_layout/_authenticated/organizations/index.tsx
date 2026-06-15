import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { createFileRoute, Link, useRouter } from "@tanstack/react-router";
import { Building2, Mail, Plus, RefreshCw } from "lucide-react";
import { toast } from "sonner";
import { type Organization, type SessionData, sessionQueryOptions, useAuthClient } from "@/app";
import { Button } from "@/components";

type AuthClientType = import("@/app").AuthClient;
type UserInvitationsResponse = Awaited<
  ReturnType<AuthClientType["organization"]["listUserInvitations"]>
>;
type UserInvitationItem = NonNullable<UserInvitationsResponse["data"]>[number];

export const Route = createFileRoute("/_layout/_authenticated/organizations/")({
  head: () => ({
    title: "Organizations | auth.everything.dev",
    meta: [{ name: "description", content: "Manage your organizations and teams." }],
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
    await context.queryClient.ensureQueryData({
      queryKey: ["user-invitations"],
      queryFn: async (): Promise<UserInvitationItem[]> => {
        const { data, error } = await context.authClient.organization.listUserInvitations();
        if (error) throw new Error(error.message);
        return (data ?? []) as UserInvitationItem[];
      },
      staleTime: 30 * 1000,
    });
  },
  component: OrganizationsList,
});

function OrganizationsList() {
  const auth = useAuthClient();
  const router = useRouter();
  const queryClient = useQueryClient();
  const { data: session } = useQuery<SessionData | null>({
    queryKey: ["session"],
    queryFn: async () => {
      const { data } = await auth.getSession();
      return data ?? null;
    },
    staleTime: 60 * 1000,
  });
  const { data: organizations, isLoading } = useQuery({
    queryKey: ["organizations"],
    queryFn: async () => {
      const { data } = await auth.organization.list();
      return (data || []) as Organization[];
    },
    staleTime: 30 * 1000,
  });

  const { data: userInvitations = [] } = useQuery({
    queryKey: ["user-invitations"],
    queryFn: async (): Promise<UserInvitationItem[]> => {
      const { data, error } = await auth.organization.listUserInvitations();
      if (error) throw new Error(error.message);
      return (data ?? []) as UserInvitationItem[];
    },
    staleTime: 30 * 1000,
  });

  const pendingInvitations = userInvitations.filter((i) => i.status === "pending");

  const acceptInvitationMutation = useMutation({
    mutationFn: async (invitation: UserInvitationItem) => {
      const { error } = await auth.organization.acceptInvitation({
        invitationId: invitation.id,
      });
      if (error) throw new Error(error.message);
      return invitation;
    },
    onSuccess: async (invitation) => {
      toast.success(`Joined ${invitation.organizationName ?? "organization"}`);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["organizations"] }),
        queryClient.invalidateQueries({ queryKey: ["session"] }),
        queryClient.invalidateQueries({ queryKey: ["user-invitations"] }),
      ]);
      await queryClient.refetchQueries({ queryKey: ["organizations"] });
      if (invitation.organizationSlug) {
        await router.navigate({
          to: "/organizations/$slug",
          params: { slug: invitation.organizationSlug },
        });
      }
    },
    onError: (error: Error) => toast.error(error.message || "Failed to accept invitation"),
  });

  const rejectInvitationMutation = useMutation({
    mutationFn: async (invitationId: string) => {
      const { error } = await auth.organization.rejectInvitation({ invitationId });
      if (error) throw new Error(error.message);
    },
    onSuccess: async () => {
      toast.success("Invitation declined");
      await queryClient.invalidateQueries({ queryKey: ["user-invitations"] });
    },
    onError: (error: Error) => toast.error(error.message || "Failed to decline invitation"),
  });

  const user = session?.user;
  const activeOrgId = session?.session?.activeOrganizationId;

  const switchOrgMutation = useMutation({
    mutationFn: async (orgId: string) => {
      const { error } = await auth.organization.setActive({ organizationId: orgId });
      if (error) throw new Error(error.message);
    },
    onSuccess: async () => {
      toast.success("Switched organization");
      await queryClient.invalidateQueries({ queryKey: ["session"] });
    },
    onError: (error: Error) => toast.error(error.message || "Failed to switch organization"),
  });

  const orgs = organizations || [];

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <div className="flex shrink-0 items-center justify-between gap-3 border-b border-border bg-card px-4 py-2.5 sm:px-6 sm:py-3">
        <h1 className="text-xl font-semibold text-foreground">Organizations</h1>
        <Link
          to="/organizations/new"
          className="h-9 rounded-[12px] bg-primary px-4 text-sm font-bold text-primary-foreground inline-flex items-center gap-1.5 no-underline transition-colors duration-150 hover:opacity-90"
        >
          <Plus size={14} />
          New
        </Link>
      </div>

      <div className="flex-1 overflow-y-auto px-4 py-6 sm:px-6">
        <div className="space-y-6">
          {pendingInvitations.length > 0 && (
            <section className="space-y-3">
              <div className="text-[11px] font-bold uppercase tracking-wider text-muted-foreground">
                Pending Invitations ({pendingInvitations.length})
              </div>
              <div className="grid gap-4 md:grid-cols-2">
                {pendingInvitations.map((invitation) => (
                  <div
                    key={invitation.id}
                    className="rounded-[12px] border border-border bg-card p-6 space-y-4"
                  >
                    <div className="flex items-start gap-4">
                      <div className="w-12 h-12 rounded-[10px] border border-border bg-muted flex items-center justify-center shrink-0">
                        <Mail className="h-5 w-5 text-muted-foreground" />
                      </div>
                      <div className="space-y-1 min-w-0 flex-1">
                        <div className="text-base font-semibold text-foreground break-all">
                          {invitation.organizationName ?? invitation.organizationSlug}
                        </div>
                        <div className="text-sm text-muted-foreground font-mono">
                          invited as {invitation.role ?? "member"}
                        </div>
                        <div className="text-sm text-muted-foreground">
                          expires {new Date(invitation.expiresAt).toLocaleDateString()}
                        </div>
                      </div>
                    </div>
                    <div className="flex gap-2">
                      <Button
                        onClick={() => acceptInvitationMutation.mutate(invitation)}
                        disabled={
                          acceptInvitationMutation.isPending || rejectInvitationMutation.isPending
                        }
                        size="sm"
                      >
                        {acceptInvitationMutation.isPending &&
                        acceptInvitationMutation.variables?.id === invitation.id
                          ? "accepting..."
                          : "accept"}
                      </Button>
                      <Button
                        onClick={() => rejectInvitationMutation.mutate(invitation.id)}
                        disabled={
                          acceptInvitationMutation.isPending || rejectInvitationMutation.isPending
                        }
                        variant="outline"
                        size="sm"
                      >
                        {rejectInvitationMutation.isPending &&
                        rejectInvitationMutation.variables === invitation.id
                          ? "declining..."
                          : "decline"}
                      </Button>
                    </div>
                  </div>
                ))}
              </div>
            </section>
          )}

          {isLoading ? (
            <div className="grid gap-6 md:grid-cols-2">
              {[1, 2].map((n) => (
                <div key={n} className="rounded-[12px] border border-border bg-card p-6 space-y-5">
                  <div className="flex items-start gap-4">
                    <div className="h-14 w-14 rounded-[10px] animate-pulse bg-muted shrink-0" />
                    <div className="space-y-2 flex-1 pt-1">
                      <div className="h-5 w-3/4 rounded-[4px] animate-pulse bg-muted" />
                      <div className="h-4 w-1/2 rounded-[4px] animate-pulse bg-muted" />
                    </div>
                  </div>
                  <div className="h-10 w-full rounded-[8px] animate-pulse bg-muted" />
                  <div className="flex gap-2">
                    <div className="h-10 w-24 rounded-[12px] animate-pulse bg-muted" />
                    <div className="h-10 w-24 rounded-[12px] animate-pulse bg-muted" />
                  </div>
                </div>
              ))}
            </div>
          ) : orgs.length === 0 ? (
            <div className="rounded-[12px] border border-border bg-card p-10 text-center space-y-4">
              <Building2 className="h-10 w-10 mx-auto text-muted-foreground" />
              <p className="text-base font-semibold text-foreground">No organizations yet.</p>
              <Link
                to="/organizations/new"
                className="h-9 rounded-[12px] bg-primary px-4 text-sm font-bold text-primary-foreground inline-flex items-center no-underline transition-colors duration-150 hover:opacity-90"
              >
                create your first org
              </Link>
            </div>
          ) : (
            <div className="grid gap-6 md:grid-cols-2">
              {orgs.map((org: Organization) => {
                const isActive = org.id === activeOrgId;
                const isPersonal = user
                  ? org.slug === user.id || org.metadata?.isPersonal === true
                  : false;

                return (
                  <div
                    key={org.id}
                    className="rounded-[12px] border border-border bg-card p-6 space-y-5"
                  >
                    <div className="flex items-start gap-4">
                      {org.logo ? (
                        <img
                          src={org.logo}
                          alt=""
                          className="w-14 h-14 rounded-[10px] border border-border object-cover shrink-0"
                        />
                      ) : (
                        <div className="w-14 h-14 rounded-[10px] border border-border bg-muted flex items-center justify-center text-xl font-bold text-foreground shrink-0">
                          {org.name.charAt(0).toUpperCase()}
                        </div>
                      )}
                      <div className="min-w-0 space-y-1.5 flex-1 pt-0.5">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="text-base font-semibold text-foreground break-all leading-tight">
                            {org.name}
                          </span>
                          {isActive && <Chip>active</Chip>}
                          {isPersonal && <Chip>personal</Chip>}
                        </div>
                        <div className="text-sm font-mono text-muted-foreground">@{org.slug}</div>
                      </div>
                    </div>

                    <div className="rounded-[8px] border border-border bg-muted px-3.5 py-2.5 text-sm text-muted-foreground">
                      {org.createdAt
                        ? `created ${new Date(org.createdAt).toLocaleDateString()}`
                        : "organization record"}
                    </div>

                    <div className="flex flex-wrap gap-2">
                      <Button asChild>
                        <Link to="/organizations/$slug" params={{ slug: org.slug }}>
                          open org
                        </Link>
                      </Button>
                      {!isActive && (
                        <Button
                          onClick={() => switchOrgMutation.mutate(org.id)}
                          disabled={switchOrgMutation.isPending}
                          variant="outline"
                        >
                          <RefreshCw className="h-4 w-4" />
                          switch
                        </Button>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          )}

          <div className="rounded-[12px] border border-border bg-card p-5 text-sm text-muted-foreground leading-relaxed">
            Each user gets a personal organization automatically. Additional organizations give
            teams their own members, invitations, and API key scope.
          </div>
        </div>
      </div>
    </div>
  );
}

function Chip({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex items-center rounded-[6px] px-2 py-0.5 text-[10px] font-semibold border bg-secondary border-border text-foreground">
      {children}
    </span>
  );
}
