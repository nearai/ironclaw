import { useMutation, useQueryClient } from "@tanstack/react-query";
import { createFileRoute, Link, useRouter } from "@tanstack/react-router";
import { useState } from "react";
import { toast } from "sonner";
import { useAuthClient } from "@/app";
import { Button, Card, CardContent, Input } from "@/components";

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
    <div className="space-y-6">
      <div className="flex items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">New Organization</h1>
          <p className="text-sm text-muted-foreground">
            Create a workspace for team members and organization API keys.
          </p>
        </div>
        <Button asChild variant="outline" size="sm">
          <Link to="/organizations">back to organizations</Link>
        </Button>
      </div>

      <form
        onSubmit={(e) => {
          e.preventDefault();
          createMutation.mutate();
        }}
        className="space-y-6"
      >
        <Card>
          <CardContent className="p-6 space-y-4">
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
          </CardContent>
        </Card>

        <div className="flex gap-2">
          <Button asChild variant="outline" size="sm">
            <Link to="/organizations">cancel</Link>
          </Button>
          <Button
            type="submit"
            disabled={createMutation.isPending || !name || !slug}
            variant="outline"
            size="sm"
          >
            {createMutation.isPending ? "creating..." : "create"}
          </Button>
        </div>
      </form>

      <section className="space-y-4">
        <h2 className="text-sm font-medium uppercase tracking-wide text-muted-foreground">
          What Happens Next
        </h2>
        <Card>
          <CardContent className="p-4">
            <ul className="space-y-2 text-xs text-muted-foreground">
              <li>• Your organization will be created immediately</li>
              <li>• You'll be the owner with full permissions</li>
              <li>• You can invite team members from the organization settings</li>
              <li>• You can switch between organizations anytime</li>
            </ul>
          </CardContent>
        </Card>
      </section>
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
      <label htmlFor={htmlFor} className="text-xs uppercase tracking-wide text-muted-foreground">
        {label}
      </label>
      {children}
    </div>
  );
}
