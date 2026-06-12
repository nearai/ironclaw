import { createFileRoute } from "@tanstack/react-router";
import { Loader2, Zap } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { sessionQueryOptions, useApiClient } from "@/app";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";

export const Route = createFileRoute("/_layout/hackathon/submit")({
  beforeLoad: async ({ context }) => {
    const { queryClient, authClient } = context;
    const session = await queryClient.ensureQueryData(
      sessionQueryOptions(authClient, context.session),
    );
    if (!session?.user) {
      throw new Error("You must be logged in to submit");
    }
  },
  component: SubmitPage,
});

function SubmitPage() {
  const apiClient = useApiClient();
  const [form, setForm] = useState({
    agentId: "",
    novaAccountId: "",
    novaApiKey: "",
    projectTitle: "",
    description: "",
    demoUrl: "",
    githubUrl: "",
    skillsList: "",
    demoNotes: "",
  });
  const [submitting, setSubmitting] = useState(false);
  const [result, setResult] = useState<{ cid: string; message: string } | null>(null);

  const handleChange = (field: string) => (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
    setForm((prev) => ({ ...prev, [field]: e.target.value }));
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setSubmitting(true);
    try {
      const res = await apiClient.hackathon.submit({
        agentId: form.agentId,
        novaAccountId: form.novaAccountId,
        novaApiKey: form.novaApiKey,
        projectTitle: form.projectTitle,
        description: form.description,
        demoUrl: form.demoUrl,
        githubUrl: form.githubUrl || undefined,
        skillsList: form.skillsList || undefined,
        demoNotes: form.demoNotes || undefined,
      });
      setResult(res);
      toast.success("Submission recorded!");
    } catch (err: any) {
      toast.error(err.message ?? "Submission failed");
    } finally {
      setSubmitting(false);
    }
  };

  if (result) {
    return (
      <div className="mx-auto max-w-lg px-4 py-10">
        <Card className="space-y-4 p-8 text-center">
          <div className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-green-500/10 text-green-500">
            <Zap size={22} />
          </div>
          <h2 className="text-lg font-semibold">Submitted!</h2>
          <p className="text-sm text-muted-foreground">{result.message}</p>
          <p className="text-xs text-muted-foreground">
            CID: {result.cid}
          </p>
          <Button onClick={() => { window.location.href = "/hackathon"; }} variant="outline">
            Back to guide
          </Button>
        </Card>
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-lg px-4 py-10">
      <Card className="space-y-6 p-8">
        <div className="space-y-1">
          <h1 className="text-lg font-semibold">Submit Your Final Entry</h1>
          <p className="text-sm text-muted-foreground">
            Encrypt and upload via NOVA.
          </p>
        </div>

        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="agentId">Agent ID</Label>
            <Input id="agentId" value={form.agentId} onChange={handleChange("agentId")} required />
          </div>
          <div className="space-y-2">
            <Label htmlFor="novaAccountId">NOVA Account ID</Label>
            <Input id="novaAccountId" value={form.novaAccountId} onChange={handleChange("novaAccountId")} required />
          </div>
          <div className="space-y-2">
            <Label htmlFor="novaApiKey">NOVA API Key</Label>
            <Input id="novaApiKey" type="password" value={form.novaApiKey} onChange={handleChange("novaApiKey")} required />
          </div>
          <div className="space-y-2">
            <Label htmlFor="projectTitle">Project Title</Label>
            <Input id="projectTitle" value={form.projectTitle} onChange={handleChange("projectTitle")} required />
          </div>
          <div className="space-y-2">
            <Label htmlFor="description">Description (280 chars max)</Label>
            <Textarea id="description" value={form.description} onChange={handleChange("description")} maxLength={280} required />
          </div>
          <div className="space-y-2">
            <Label htmlFor="demoUrl">Demo URL (video)</Label>
            <Input id="demoUrl" type="url" value={form.demoUrl} onChange={handleChange("demoUrl")} required />
          </div>
          <div className="space-y-2">
            <Label htmlFor="githubUrl">GitHub Repo (optional)</Label>
            <Input id="githubUrl" type="url" value={form.githubUrl} onChange={handleChange("githubUrl")} />
          </div>
          <div className="space-y-2">
            <Label htmlFor="skillsList">Skills List (optional)</Label>
            <Input id="skillsList" value={form.skillsList} onChange={handleChange("skillsList")} placeholder="comma-separated" />
          </div>
          <div className="space-y-2">
            <Label htmlFor="demoNotes">Demo Notes (optional)</Label>
            <Textarea id="demoNotes" value={form.demoNotes} onChange={handleChange("demoNotes")} />
          </div>
          <Button type="submit" className="w-full" disabled={submitting}>
            {submitting ? <Loader2 className="h-4 w-4 animate-spin" /> : <Zap size={14} />}
            {submitting ? "Submitting..." : "Submit Entry"}
          </Button>
        </form>
      </Card>
    </div>
  );
}
