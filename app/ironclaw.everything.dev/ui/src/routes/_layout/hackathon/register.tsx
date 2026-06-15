import { createFileRoute } from "@tanstack/react-router";
import { Loader2, UserPlus } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { sessionQueryOptions, useApiClient } from "@/app";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export const Route = createFileRoute("/_layout/hackathon/register")({
  beforeLoad: async ({ context }) => {
    const { queryClient, authClient } = context;
    const session = await queryClient.ensureQueryData(
      sessionQueryOptions(authClient, context.session),
    );
    if (!session?.user) {
      throw new Error("You must be logged in to register");
    }
  },
  component: RegisterPage,
});

function RegisterPage() {
  const apiClient = useApiClient();
  const [agentId, setAgentId] = useState("");
  const [participantName, setParticipantName] = useState("");
  const [novaAccountId, setNovaAccountId] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [result, setResult] = useState<{ message: string } | null>(null);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setSubmitting(true);
    try {
      const res = await apiClient.hackathon.register({
        agentId,
        participantName,
        novaAccountId,
      });
      setResult(res);
      toast.success("Registration successful!");
    } catch (err: any) {
      toast.error(err.message ?? "Registration failed");
    } finally {
      setSubmitting(false);
    }
  };

  if (result) {
    return (
      <div className="mx-auto max-w-lg px-4 py-10">
        <Card className="space-y-4 p-8 text-center">
          <div className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-green-500/10 text-green-500">
            <UserPlus size={22} />
          </div>
          <h2 className="text-lg font-semibold">Registered!</h2>
          <p className="text-sm text-muted-foreground whitespace-pre-wrap">{result.message}</p>
          <Button onClick={() => (window.location.href = "/hackathon")} variant="outline">
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
          <h1 className="text-lg font-semibold">Register for the Hackathon</h1>
          <p className="text-sm text-muted-foreground">Record your intent to compete.</p>
        </div>

        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="agentId">Agent ID</Label>
            <Input
              id="agentId"
              value={agentId}
              onChange={(e) => setAgentId(e.target.value)}
              placeholder="my-agent"
              required
              pattern={"[^\\s/\"'\\\\]+"}
              title="No spaces, slashes, or quotes"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="participantName">Participant Name</Label>
            <Input
              id="participantName"
              value={participantName}
              onChange={(e) => setParticipantName(e.target.value)}
              placeholder="@handle or display name"
              required
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="novaAccountId">NOVA Account ID</Label>
            <Input
              id="novaAccountId"
              value={novaAccountId}
              onChange={(e) => setNovaAccountId(e.target.value)}
              placeholder="alice.nova-sdk.near"
              required
            />
          </div>
          <Button type="submit" className="w-full" disabled={submitting}>
            {submitting ? <Loader2 className="h-4 w-4 animate-spin" /> : <UserPlus size={14} />}
            {submitting ? "Registering..." : "Register"}
          </Button>
        </form>
      </Card>
    </div>
  );
}
