import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { createFileRoute, Link, useRouter } from "@tanstack/react-router";
import { CheckCircle, XCircle } from "lucide-react";
import { toast } from "sonner";
import { getAppName, useAuthClient } from "@/app";
import { Badge, Button, Card, CardContent } from "@/components";

export const Route = createFileRoute("/_layout/_authenticated/accept-invitation/$id")({
  head: () => ({
    meta: [{ title: `Accept Invitation | ${getAppName()}` }],
  }),
  loader: async ({ context, params }) => {
    await context.queryClient.ensureQueryData({
      queryKey: ["invitation", params.id],
      queryFn: async () => {
        const { data, error } = await context.authClient.organization.getInvitation({
          query: { id: params.id },
        });
        if (error) {
          if (error.status === 400 || error.status === 404) return null;
          throw new Error(error.message || "Failed to load invitation");
        }
        return data;
      },
      staleTime: 30 * 1000,
      retry: false,
    });
  },
  component: AcceptInvitation,
});

function AcceptInvitation() {
  const { id } = Route.useParams();
  const router = useRouter();
  const auth = useAuthClient();
  const queryClient = useQueryClient();

  const { data: invitation, isLoading } = useQuery({
    queryKey: ["invitation", id],
    queryFn: async () => {
      const { data, error } = await auth.organization.getInvitation({ query: { id } });
      if (error) {
        if (error.status === 400 || error.status === 404) return null;
        throw new Error(error.message || "Failed to load invitation");
      }
      return data;
    },
    staleTime: 30 * 1000,
    retry: false,
  });

  const acceptMutation = useMutation({
    mutationFn: async () => {
      const { error } = await auth.organization.acceptInvitation({ invitationId: id });
      if (error) throw new Error(error.message);
    },
    onSuccess: async () => {
      toast.success("Invitation accepted");
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["organizations"] }),
        queryClient.invalidateQueries({ queryKey: ["session"] }),
        queryClient.invalidateQueries({ queryKey: ["user-invitations"] }),
      ]);
      await queryClient.refetchQueries({ queryKey: ["organizations"] });
      await router.navigate({
        to: "/organizations/$slug",
        params: { slug: invitation?.organizationSlug ?? "" },
      });
    },
    onError: (error: Error) => toast.error(error.message || "Failed to accept invitation"),
  });

  const rejectMutation = useMutation({
    mutationFn: async () => {
      const { error } = await auth.organization.rejectInvitation({ invitationId: id });
      if (error) throw new Error(error.message);
    },
    onSuccess: async () => {
      toast.success("Invitation declined");
      await queryClient.invalidateQueries({ queryKey: ["user-invitations"] });
      await router.navigate({ to: "/organizations" });
    },
    onError: (error: Error) => toast.error(error.message || "Failed to decline invitation"),
  });

  if (isLoading) {
    return (
      <div className="flex items-center justify-center min-h-[40vh]">
        <p className="text-sm text-muted-foreground">Loading invitation...</p>
      </div>
    );
  }

  if (!invitation) {
    return (
      <Card className="max-w-md mx-auto mt-12">
        <CardContent className="p-8 text-center space-y-4">
          <XCircle className="h-8 w-8 mx-auto text-muted-foreground" />
          <p className="text-sm">
            This invitation does not exist, has expired, or is not addressed to your account.
          </p>
          <Button asChild variant="outline" size="sm">
            <Link to="/organizations">go to organizations</Link>
          </Button>
        </CardContent>
      </Card>
    );
  }

  const isPending_ = acceptMutation.isPending || rejectMutation.isPending;

  return (
    <div className="max-w-lg mx-auto mt-12 space-y-6">
      <Card>
        <CardContent className="p-8 space-y-6">
          <div className="space-y-2 text-center">
            <div className="flex justify-center">
              <CheckCircle className="h-8 w-8 text-muted-foreground" />
            </div>
            <h1 className="text-xl font-semibold tracking-tight">You've been invited</h1>
            <p className="text-sm text-muted-foreground">
              You have been invited to join{" "}
              <span className="font-medium text-foreground">
                {invitation.organizationName ?? invitation.organizationSlug}
              </span>{" "}
              as <span className="font-mono">{invitation.role ?? "member"}</span>.
            </p>
          </div>

          <div className="border-2 border-outset border-border bg-muted/10 p-4 space-y-2 text-xs font-mono">
            <div className="flex justify-between gap-4">
              <span className="text-muted-foreground">organization</span>
              <span className="text-right break-all">
                {invitation.organizationName ?? invitation.organizationSlug}
              </span>
            </div>
            <div className="flex justify-between gap-4">
              <span className="text-muted-foreground">role</span>
              <span>{invitation.role ?? "member"}</span>
            </div>
            <div className="flex justify-between gap-4">
              <span className="text-muted-foreground">expires</span>
              <span>{new Date(invitation.expiresAt).toLocaleDateString()}</span>
            </div>
            <div className="flex justify-between gap-4">
              <span className="text-muted-foreground">status</span>
              <Badge variant="outline">{invitation.status}</Badge>
            </div>
          </div>

          <div className="flex gap-3 justify-center">
            <Button onClick={() => acceptMutation.mutate()} disabled={isPending_} size="sm">
              {acceptMutation.isPending ? "accepting..." : "accept"}
            </Button>
            <Button
              onClick={() => rejectMutation.mutate()}
              disabled={isPending_}
              variant="outline"
              size="sm"
            >
              {rejectMutation.isPending ? "declining..." : "decline"}
            </Button>
          </div>
        </CardContent>
      </Card>

      <div className="text-center">
        <Button asChild variant="ghost" size="sm">
          <Link to="/organizations">back to organizations</Link>
        </Button>
      </div>
    </div>
  );
}
