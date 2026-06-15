import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate, useRouter } from "@tanstack/react-router";
import { Check, Cloud, Terminal } from "lucide-react";
import { toast } from "sonner";
import { useMemo } from "react";
import type { Organization } from "@/app";
import { ironclawStatusQueryKey } from "@/hooks/use-ironclaw-status";
import { sessionQueryOptions, useApiClient, useAuthClient } from "@/app";
import { OrgSwitcher } from "@/components";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useConnectionMode } from "@/hooks/use-connection-mode";

export function UserNav() {
  const auth = useAuthClient();
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const router = useRouter();
  const { data: session } = useQuery(sessionQueryOptions(auth));
  const user = session?.user;
  const { data: organizations } = useQuery({
    queryKey: ["organizations"],
    queryFn: async () => {
      const { data } = await auth.organization.list();
      return (data || []) as Organization[];
    },
    staleTime: 30 * 1000,
    enabled: !!user,
  });
  const activeOrgId = session?.session?.activeOrganizationId;

  const activeOrg = useMemo(() => {
    return organizations?.find((org) => org.id === activeOrgId);
  }, [organizations, activeOrgId]);

  const nearAccountId = auth.near.getAccountId();

  const { data: nearProfile } = useQuery({
    queryKey: ["near-profile", nearAccountId],
    queryFn: async () => {
      const res = await auth.near.getProfile(nearAccountId ?? "");
      return (res as any)?.data || null;
    },
    enabled: !!nearAccountId,
    staleTime: 5 * 60 * 1000,
  });

  const nearAvatarUrl =
    nearProfile?.image?.url ??
    (nearProfile?.image?.ipfs_cid
      ? `https://ipfs.near.social/ipfs/${nearProfile.image.ipfs_cid}`
      : null);

  const { connectionMode, switchMode } = useConnectionMode();
  const apiClient = useApiClient();

  const disconnectTunnelMutation = useMutation({
    mutationFn: async () => {
      await apiClient.ironclaw.settings.delete();
    },
    onSuccess: () => {
      switchMode("hosted");
      queryClient.invalidateQueries({ queryKey: ironclawStatusQueryKey });
      toast.success("Tunnel disconnected");
    },
    onError: (err: any) => {
      toast.error(err.message ?? "Failed to disconnect");
    },
  });

  const signOutMutation = useMutation({
    mutationFn: async () => {
      const { error } = await auth.signOut();
      if (error) {
        throw new Error(error.message || "Failed to sign out");
      }
      await auth.near.disconnect().catch(() => {});
    },
    onSuccess: async () => {
      queryClient.setQueryData(["session"], null);
      queryClient.removeQueries({ queryKey: ["organizations"] });
      await queryClient.invalidateQueries({ queryKey: ["session"] });
      await router.invalidate();
      await navigate({ to: "/", replace: true });
    },
    onError: (error: Error) => {
      console.error("Sign out error:", error);
    },
  });

  if (!user) {
    return (
      <div className="flex items-center gap-2">
        <Link
          to="/login"
          className="h-9 px-4 inline-flex items-center justify-center text-sm font-medium border-2 border-outset border-border-strong bg-card text-foreground shadow-sm hover:shadow-md hover:bg-muted active:border-inset active:shadow-none transition-all duration-200 ease-out cursor-pointer"
        >
          connect
        </Link>
        <DotControl />
      </div>
    );
  }

  const handleOrgSwitch = async () => {
    await queryClient.invalidateQueries({ queryKey: ["session"] });
    await queryClient.invalidateQueries({ queryKey: ["organizations"] });
  };

  return (
    <div className="flex items-center gap-2">
      {organizations && organizations.length > 0 && (
        <OrgSwitcher
          organizations={organizations}
          activeOrgId={activeOrgId}
          onSwitch={handleOrgSwitch}
        />
      )}

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            className="w-6 h-6 rounded-full! transition-all duration-200 ease-out hover:shadow-lg hover:scale-110 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
            title={nearProfile?.name ?? nearAccountId ?? "menu"}
          >
            <Avatar className="w-6 h-6">
              {nearAvatarUrl ? (
                <AvatarImage
                  src={nearAvatarUrl}
                  alt={nearProfile?.name ?? nearAccountId ?? "User"}
                />
              ) : null}
              <AvatarFallback className="bg-foreground text-background text-[10px] font-medium">
                {(nearProfile?.name ?? user.email ?? user.id).charAt(0).toUpperCase()}
              </AvatarFallback>
            </Avatar>
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-56">
          <DropdownMenuLabel>
            <div className="space-y-1">
              <p className="text-xs text-muted-foreground">signed in as</p>
              {nearProfile?.name && (
                <p className="truncate text-sm font-medium">{nearProfile.name}</p>
              )}
              <p className="truncate text-sm font-normal">{user.email || user.id}</p>
            </div>
          </DropdownMenuLabel>
          <DropdownMenuSeparator />
          <DropdownMenuItem asChild>
            <Link to="/home">workspace</Link>
          </DropdownMenuItem>
          {activeOrg && (
            <DropdownMenuItem asChild>
              <Link to="/organizations/$slug" params={{ slug: activeOrg.slug }}>
                {activeOrg.name}
              </Link>
            </DropdownMenuItem>
          )}
          <DropdownMenuSeparator />
          <DropdownMenuLabel className="text-xs text-muted-foreground">
            connection
          </DropdownMenuLabel>
          <DropdownMenuItem
            onClick={() => switchMode("hosted")}
            className="gap-2 text-xs cursor-pointer"
          >
            <div className="flex h-4 w-4 items-center justify-center">
              {connectionMode === "hosted" && (
                <Check size={12} className="text-[color:var(--near-green)]" />
              )}
            </div>
            <Cloud size={12} className="shrink-0" />
            Using hosted agent
          </DropdownMenuItem>
          <DropdownMenuItem
            onClick={() => switchMode("local")}
            className="gap-2 text-xs cursor-pointer"
          >
            <div className="flex h-4 w-4 items-center justify-center">
              {connectionMode === "local" && (
                <Check size={12} className="text-[color:var(--near-green)]" />
              )}
            </div>
            <Terminal size={12} className="shrink-0" />
            Using my own binary
          </DropdownMenuItem>
          <DropdownMenuItem
            onClick={() => disconnectTunnelMutation.mutate()}
            disabled={disconnectTunnelMutation.isPending}
            className="gap-2 text-xs cursor-pointer text-destructive focus:text-destructive"
          >
            <Terminal size={12} className="shrink-0" />
            {disconnectTunnelMutation.isPending ? "Disconnecting..." : "Disconnect from tunnel"}
          </DropdownMenuItem>
          <DropdownMenuItem asChild>
            <Link to="/settings">settings</Link>
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            variant="destructive"
            onSelect={(event) => {
              event.preventDefault();
              signOutMutation.mutate();
            }}
            disabled={signOutMutation.isPending}
          >
            {signOutMutation.isPending ? "signing out..." : "sign out"}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}

function DotControl() {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className="w-6 h-6 rounded-full! bg-foreground transition-all duration-200 ease-out hover:shadow-lg hover:scale-110 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
          title="actions"
        />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-48">
        <DropdownMenuLabel className="text-xs text-muted-foreground">navigate</DropdownMenuLabel>
        <DropdownMenuItem asChild>
          <Link to="/login">connect</Link>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
