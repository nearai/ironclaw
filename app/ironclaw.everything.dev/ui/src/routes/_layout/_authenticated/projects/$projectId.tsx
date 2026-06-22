import { createFileRoute, Link } from "@tanstack/react-router";
import {
  ArrowLeft,
  Loader2,
  Plus,
  RefreshCw,
  Shield,
  Trash2,
  User,
  XCircle,
} from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import {
  projectMembersQueryOptions,
  projectQueryOptions,
  useAddProjectMember,
  useProject,
  useProjectMembers,
  useRemoveProjectMember,
  useUpdateProjectMember,
} from "@/hooks/use-projects";

export const Route = createFileRoute("/_layout/_authenticated/projects/$projectId")({
  loader: async ({ context, params }) => {
    const { projectId } = params;
    await context.queryClient.ensureQueryData(projectQueryOptions(context.apiClient, projectId));
    await context.queryClient.ensureQueryData(
      projectMembersQueryOptions(context.apiClient, projectId),
    );
  },
  component: ProjectDetailPage,
});

function ProjectDetailPage() {
  const { projectId } = Route.useParams();
  const { data: project, isLoading: projectLoading, isError: projectError } = useProject(projectId);
  const { data: members, isLoading: membersLoading, isError: membersError, refetch: refetchMembers } =
    useProjectMembers(projectId);
  const addMember = useAddProjectMember(projectId);
  const updateMember = useUpdateProjectMember(projectId);
  const removeMember = useRemoveProjectMember(projectId);
  const [removingUserId, setRemovingUserId] = useState<string | null>(null);

  const [showAddForm, setShowAddForm] = useState(false);
  const [newUserId, setNewUserId] = useState("");
  const [newUserRole, setNewUserRole] = useState("member");

  const handleAddMember = () => {
    const trimmed = newUserId.trim();
    if (!trimmed) return;
    addMember.mutate(
      { userId: trimmed, role: newUserRole },
      {
        onSuccess: () => {
          toast.success("Member added");
          setNewUserId("");
          setNewUserRole("member");
          setShowAddForm(false);
        },
        onError: (error: Error) => {
          toast.error(error.message ?? "Failed to add member");
        },
      },
    );
  };

  const handleUpdateRole = (userId: string, role: string) => {
    updateMember.mutate(
      { userId, role },
      {
        onSuccess: () => toast.success("Role updated"),
        onError: (error: Error) => toast.error(error.message ?? "Failed to update role"),
      },
    );
  };

  const handleRemoveMember = (userId: string) => {
    setRemovingUserId(userId);
    removeMember.mutate(userId, {
      onSuccess: () => {
        toast.success("Member removed");
        setRemovingUserId(null);
      },
      onError: (error: Error) => {
        toast.error(error.message ?? "Failed to remove member");
        setRemovingUserId(null);
      },
    });
  };

  if (projectLoading) {
    return (
      <div className="p-6 space-y-4 max-w-3xl mx-auto">
        <Skeleton className="h-8 w-48" />
        <Skeleton className="h-4 w-96" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (projectError || !project) {
    return (
      <div className="flex items-center justify-center h-full p-6">
        <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-6 text-center space-y-3">
          <XCircle className="mx-auto h-6 w-6 text-destructive" />
          <p className="text-sm text-destructive">Failed to load project</p>
          <Button variant="outline" size="sm" asChild>
            <Link to="/projects">Back to projects</Link>
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-6 p-6 max-w-3xl mx-auto">
      <div className="flex items-center gap-2">
        <Button variant="ghost" size="icon" className="h-8 w-8 shrink-0" asChild>
          <Link to="/projects">
            <ArrowLeft size={16} />
          </Link>
        </Button>
        <div className="space-y-0.5">
          <h1 className="text-lg font-semibold text-foreground">{project.name}</h1>
          {project.description && (
            <p className="text-sm text-muted-foreground">{project.description}</p>
          )}
        </div>
      </div>

      <Card className="p-5 space-y-4">
        <div className="flex items-center justify-between">
          <h2 className="text-sm font-semibold text-foreground">
            Members
            {members && members.length > 0 && (
              <span className="ml-2 text-xs font-normal text-muted-foreground">
                ({members.length})
              </span>
            )}
          </h2>
          {!showAddForm && (
            <Button size="sm" variant="outline" onClick={() => setShowAddForm(true)}>
              <Plus size={14} className="mr-1.5" />
              Add member
            </Button>
          )}
        </div>

        {showAddForm && (
          <div className="rounded-lg border border-border bg-card p-4 space-y-3">
            <div className="grid grid-cols-[1fr_140px_auto] gap-3 items-end">
              <div className="space-y-1.5">
                <Label htmlFor="new-member-id" className="text-xs">
                  User ID
                </Label>
                <Input
                  id="new-member-id"
                  value={newUserId}
                  onChange={(e) => setNewUserId(e.target.value)}
                  placeholder="user-abc123"
                  className="h-8 text-sm"
                  onKeyDown={(e) => {
                    if (e.key === "Enter") handleAddMember();
                  }}
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="new-member-role" className="text-xs">
                  Role
                </Label>
                <Select value={newUserRole} onValueChange={setNewUserRole}>
                  <SelectTrigger id="new-member-role" className="h-8 text-xs">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="member">Member</SelectItem>
                    <SelectItem value="owner">Owner</SelectItem>
                    <SelectItem value="viewer">Viewer</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="flex gap-2">
                <Button
                  size="sm"
                  onClick={handleAddMember}
                  disabled={!newUserId.trim() || addMember.isPending}
                >
                  {addMember.isPending ? (
                    <Loader2 size={14} className="animate-spin" />
                  ) : (
                    "Add"
                  )}
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => setShowAddForm(false)}
                >
                  Cancel
                </Button>
              </div>
            </div>
          </div>
        )}

        {membersLoading ? (
          <div className="space-y-2">
            {Array.from({ length: 3 }).map((_, i) => (
              <Skeleton key={i} className="h-12 w-full" />
            ))}
          </div>
        ) : membersError ? (
          <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-4 text-center space-y-2">
            <XCircle className="mx-auto h-5 w-5 text-destructive" />
            <p className="text-xs text-destructive">Failed to load members</p>
            <Button variant="outline" size="sm" onClick={() => refetchMembers()}>
              <RefreshCw size={12} className="mr-1" />
              Retry
            </Button>
          </div>
        ) : !members || members.length === 0 ? (
          <div className="rounded-lg border border-border p-6 text-center space-y-1">
            <User className="mx-auto h-5 w-5 text-muted-foreground" />
            <p className="text-sm text-muted-foreground">No members yet</p>
          </div>
        ) : (
          <div className="divide-y divide-border -mx-0">
            {members.map((member) => (
              <div
                key={member.userId}
                className="flex items-center justify-between py-3 first:pt-0 last:pb-0"
              >
                <div className="flex items-center gap-3 min-w-0">
                  <div className="flex h-8 w-8 items-center justify-center rounded-full bg-muted shrink-0">
                    <User size={14} className="text-muted-foreground" />
                  </div>
                  <div className="min-w-0">
                    <p className="text-sm font-medium text-foreground truncate">
                      {member.displayName ?? member.userId}
                    </p>
                    {member.displayName && (
                      <p className="text-xs text-muted-foreground truncate">{member.userId}</p>
                    )}
                  </div>
                </div>
                <div className="flex items-center gap-2 shrink-0">
                  <Select
                    value={member.role}
                    onValueChange={(role) => handleUpdateRole(member.userId, role)}
                  >
                    <SelectTrigger className="h-7 w-[100px] text-xs">
                      <Shield size={12} className="mr-1" />
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="owner">Owner</SelectItem>
                      <SelectItem value="member">Member</SelectItem>
                      <SelectItem value="viewer">Viewer</SelectItem>
                    </SelectContent>
                  </Select>
                  <Button
                    size="icon"
                    variant="ghost"
                    className="h-7 w-7 text-destructive hover:text-destructive"
                    onClick={() => handleRemoveMember(member.userId)}
                    disabled={removingUserId === member.userId}
                  >
                    {removingUserId === member.userId ? (
                      <Loader2 size={14} className="animate-spin" />
                    ) : (
                      <Trash2 size={14} />
                    )}
                  </Button>
                </div>
              </div>
            ))}
          </div>
        )}
      </Card>
    </div>
  );
}
