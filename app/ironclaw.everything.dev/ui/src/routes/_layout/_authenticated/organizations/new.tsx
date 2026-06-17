import { useMutation, useQueryClient } from "@tanstack/react-query";
import { createFileRoute, Link, useRouter } from "@tanstack/react-router";
import { ArrowLeft, Building2 } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { useAuthClient } from "@/app";
import { Button, Input } from "@/components";

export const Route = createFileRoute("/_layout/_authenticated/organizations/new")({
  head: () => ({
    title: "New Organization | auth.everything.dev",
    meta: [{ name: "description", content: "Create a new organization." }],
  }),
  component: NewOrganization,
});

function NewOrganization() {
  const router = useRouter();
  const auth = useAuthClient();
  const queryClient = useQueryClient();
  const [name, setName] = useState("");
  const [slug, setSlug] = useState("");

  const createMutation = useMutation({
    mutationFn: async () => {
      const { data, error } = await auth.organization.create({
        name,
        slug,
      });
      if (error) throw new Error(error.message);
      return data;
    },
    onSuccess: async (data) => {
      toast.success(`Organization "${data?.name}" created`);
      await queryClient.invalidateQueries({ queryKey: ["organizations"] });
      await queryClient.refetchQueries({ queryKey: ["organizations"] });
      if (data?.slug) {
        await router.navigate({
          to: "/organizations/$slug",
          params: { slug: data.slug },
        });
      }
    },
    onError: (error: Error) => {
      toast.error(error.message || "Failed to create organization");
    },
  });

  const generateSlug = (value: string) => {
    return value
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "");
  };

  const handleNameChange = (value: string) => {
    setName(value);
    if (!slug || slug === generateSlug(name.slice(0, -1))) {
      setSlug(generateSlug(value));
    }
  };

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <div className="flex shrink-0 items-center justify-between gap-3 border-b border-border bg-card px-4 py-2.5 sm:px-6 sm:py-3">
        <div className="flex items-center gap-3">
          <Link
            to="/organizations"
            className="inline-flex items-center justify-center h-8 w-8 rounded-[10px] border border-border bg-muted text-muted-foreground hover:text-foreground transition-colors duration-150"
          >
            <ArrowLeft size={16} />
          </Link>
          <h1 className="text-xl font-semibold text-foreground">New Organization</h1>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto px-4 py-6 sm:px-6">
        <div className="space-y-6">
          <form
            onSubmit={(e) => {
              e.preventDefault();
              createMutation.mutate();
            }}
            className="space-y-6"
          >
            <div className="rounded-[12px] border border-border bg-card p-6 space-y-4">
              <Field label="name" htmlFor="organization-name">
                <Input
                  id="organization-name"
                  type="text"
                  value={name}
                  onChange={(e) => handleNameChange(e.target.value)}
                  placeholder="My Team"
                  required
                />
              </Field>
              <Field label="slug" htmlFor="organization-slug">
                <div className="flex items-center gap-2">
                  <span className="text-muted-foreground">@</span>
                  <Input
                    id="organization-slug"
                    type="text"
                    value={slug}
                    onChange={(e) => setSlug(e.target.value.replace(/[^a-z0-9-]/g, ""))}
                    placeholder="my-team"
                    pattern="[a-z0-9-]+"
                    required
                  />
                </div>
                <p className="text-xs text-muted-foreground mt-2">
                  Only lowercase letters, numbers, and hyphens.
                </p>
              </Field>
            </div>

            <div className="flex gap-2">
              <Link
                to="/organizations"
                className="h-9 rounded-[12px] border border-border bg-card px-4 text-sm font-medium text-foreground inline-flex items-center no-underline transition-colors duration-150 hover:bg-muted"
              >
                cancel
              </Link>
              <Button
                type="submit"
                disabled={createMutation.isPending || !name || !slug}
                size="sm"
              >
                {createMutation.isPending ? "creating..." : "create"}
              </Button>
            </div>
          </form>

          <section className="space-y-3">
            <div className="text-[11px] font-bold uppercase tracking-wider text-muted-foreground">
              What Happens Next
            </div>
            <div className="rounded-[12px] border border-border bg-card p-6 space-y-3">
              <div className="flex items-start gap-3">
                <div className="w-10 h-10 rounded-[10px] border border-border bg-muted flex items-center justify-center shrink-0">
                  <Building2 className="h-5 w-5 text-muted-foreground" />
                </div>
                <div className="space-y-1 pt-0.5">
                  <p className="text-sm font-medium text-foreground">Organization created immediately</p>
                  <p className="text-sm text-muted-foreground">You'll be the owner with full permissions.</p>
                </div>
              </div>
              <div className="flex items-start gap-3">
                <div className="w-10 h-10 rounded-[10px] border border-border bg-muted flex items-center justify-center shrink-0">
                  <Building2 className="h-5 w-5 text-muted-foreground" />
                </div>
                <div className="space-y-1 pt-0.5">
                  <p className="text-sm font-medium text-foreground">Invite team members</p>
                  <p className="text-sm text-muted-foreground">From the organization settings page.</p>
                </div>
              </div>
              <div className="flex items-start gap-3">
                <div className="w-10 h-10 rounded-[10px] border border-border bg-muted flex items-center justify-center shrink-0">
                  <Building2 className="h-5 w-5 text-muted-foreground" />
                </div>
                <div className="space-y-1 pt-0.5">
                  <p className="text-sm font-medium text-foreground">Switch anytime</p>
                  <p className="text-sm text-muted-foreground">Use the org switcher in the header to flip between workspaces.</p>
                </div>
              </div>
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}

function Field({
  label,
  htmlFor,
  children,
}: {
  label: string;
  htmlFor: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-2">
      <label htmlFor={htmlFor} className="text-[11px] font-bold uppercase tracking-wider text-muted-foreground">
        {label}
      </label>
      {children}
    </div>
  );
}
