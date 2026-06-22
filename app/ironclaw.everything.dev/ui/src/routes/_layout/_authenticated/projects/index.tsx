import { createFileRoute, Link } from "@tanstack/react-router";
import {
  FolderGit2,
  Loader2,
  Plus,
  RefreshCw,
  Trash2,
  Users,
  XCircle,
} from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import {
  projectsQueryOptions,
  useCreateProject,
  useDeleteProject,
  useProjects,
} from "@/hooks/use-projects";

export const Route = createFileRoute("/_layout/_authenticated/projects/")({
  loader: async ({ context }) => {
    await context.queryClient.ensureQueryData(projectsQueryOptions(context.apiClient));
  },
  component: ProjectsPage,
});

function ProjectSkeleton() {
  return (
    <Card className="p-5 space-y-3">
      <Skeleton className="h-5 w-40" />
      <Skeleton className="h-3 w-full" />
      <Skeleton className="h-3 w-3/4" />
      <div className="flex items-center gap-3 pt-2">
        <Skeleton className="h-4 w-16" />
        <Skeleton className="h-4 w-24" />
      </div>
      <Skeleton className="h-8 w-20 rounded-md" />
    </Card>
  );
}

function CreateProjectDialog() {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const create = useCreateProject();

  const handleSubmit = async () => {
    const trimmed = name.trim();
    if (!trimmed) return;
    create.mutate(
      { name: trimmed, description: description.trim() || undefined },
      {
        onSuccess: () => {
          toast.success(`Project created`);
          setOpen(false);
          setName("");
          setDescription("");
        },
        onError: (error: Error) => {
          toast.error(error.message ?? "Failed to create project");
        },
      },
    );
  };

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button size="sm">
          <Plus size={14} className="mr-1.5" />
          New Project
        </Button>
      </DialogTrigger>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Create project</DialogTitle>
          <DialogDescription>Projects group threads, files, and members.</DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="project-name">Name</Label>
            <Input
              id="project-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="My Project"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="project-desc">Description (optional)</Label>
            <Textarea
              id="project-desc"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="What is this project about?"
              rows={3}
            />
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" size="sm" onClick={() => setOpen(false)}>
            Cancel
          </Button>
          <Button size="sm" onClick={handleSubmit} disabled={!name.trim() || create.isPending}>
            {create.isPending ? (
              <Loader2 size={14} className="mr-1.5 animate-spin" />
            ) : (
              <Plus size={14} className="mr-1.5" />
            )}
            Create
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function ProjectsPage() {
  const { data: projects, isLoading, isError, refetch } = useProjects();
  const deleteProject = useDeleteProject();
  const [deletingId, setDeletingId] = useState<string | null>(null);

  const handleDelete = (projectId: string) => {
    setDeletingId(projectId);
    deleteProject.mutate(projectId, {
      onSuccess: () => {
        toast.success("Project deleted");
        setDeletingId(null);
      },
      onError: (error: Error) => {
        toast.error(error.message ?? "Failed to delete project");
        setDeletingId(null);
      },
    });
  };

  return (
    <div className="space-y-6 p-6 max-w-5xl mx-auto">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <div className="flex h-10 w-10 items-center justify-center rounded-full bg-primary/10">
            <FolderGit2 className="h-5 w-5 text-primary" />
          </div>
          <div className="space-y-0.5">
            <h1 className="text-lg font-semibold text-foreground">Projects</h1>
            <p className="text-sm text-muted-foreground">
              Organize threads, files, and team members into scoped workspaces.
            </p>
          </div>
        </div>
        <CreateProjectDialog />
      </div>

      {isLoading ? (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          <ProjectSkeleton />
          <ProjectSkeleton />
          <ProjectSkeleton />
          <ProjectSkeleton />
        </div>
      ) : isError ? (
        <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-6 text-center space-y-3">
          <XCircle className="mx-auto h-6 w-6 text-destructive" />
          <p className="text-sm text-destructive">Failed to load projects</p>
          <Button variant="outline" size="sm" onClick={() => refetch()}>
            <RefreshCw size={14} className="mr-1.5" />
            Retry
          </Button>
        </div>
      ) : !projects || projects.length === 0 ? (
        <div className="rounded-lg border border-border p-12 text-center space-y-3">
          <div className="flex h-12 w-12 items-center justify-center rounded-full bg-muted mx-auto">
            <FolderGit2 className="h-6 w-6 text-muted-foreground" />
          </div>
          <div className="space-y-1">
            <p className="text-sm font-medium text-foreground">No projects yet</p>
            <p className="text-xs text-muted-foreground">
              Create a project to organize threads and team members.
            </p>
          </div>
          <CreateProjectDialog />
        </div>
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {projects.map((project) => (
            <Card key={project.projectId} className="p-5 space-y-3">
              <Link
                to="/projects/$projectId"
                params={{ projectId: project.projectId }}
                className="block"
              >
                <h2 className="font-medium text-foreground hover:text-primary transition-colors">
                  {project.name}
                </h2>
              </Link>
              {project.description && (
                <p className="text-sm text-muted-foreground line-clamp-2">
                  {project.description}
                </p>
              )}
              <div className="flex items-center gap-3 text-xs text-muted-foreground">
                {project.memberCount !== undefined && (
                  <span className="inline-flex items-center gap-1">
                    <Users size={12} />
                    {project.memberCount} member{project.memberCount !== 1 ? "s" : ""}
                  </span>
                )}
              </div>
              {project.createdAt && (
                <p className="text-xs text-muted-foreground">
                  Created {new Date(project.createdAt).toLocaleDateString()}
                </p>
              )}
              <div className="flex gap-2 pt-1">
                <Button size="sm" asChild variant="secondary">
                  <Link
                    to="/projects/$projectId"
                    params={{ projectId: project.projectId }}
                  >
                    View
                  </Link>
                </Button>
                <Button
                  size="sm"
                  variant="destructive"
                  onClick={() => handleDelete(project.projectId)}
                  disabled={deletingId === project.projectId}
                >
                  {deletingId === project.projectId ? (
                    <Loader2 size={14} className="mr-1 animate-spin" />
                  ) : (
                    <Trash2 size={14} className="mr-1" />
                  )}
                  Delete
                </Button>
              </div>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
