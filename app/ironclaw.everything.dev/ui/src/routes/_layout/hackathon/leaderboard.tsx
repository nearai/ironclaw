import { useQuery } from "@tanstack/react-query";
import { createFileRoute } from "@tanstack/react-router";
import { Trophy } from "lucide-react";
import { useApiClient } from "@/app";

export const Route = createFileRoute("/_layout/hackathon/leaderboard")({
  component: LeaderboardPage,
});

function LeaderboardPage() {
  const apiClient = useApiClient();

  const { data, isLoading, error } = useQuery({
    queryKey: ["hackathon", "leaderboard"],
    queryFn: () => apiClient.hackathon.leaderboard(),
    refetchInterval: 30_000,
  });

  const entries = data?.entries ?? [];

  return (
    <div className="mx-auto max-w-3xl px-4 py-10">
      <div className="mb-6 flex items-center gap-3">
        <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-amber-500/10 text-amber-500">
          <Trophy size={18} />
        </div>
        <div>
          <h1 className="text-lg font-semibold">Leaderboard</h1>
          <p className="text-sm text-muted-foreground">Submissions ranked by submission time</p>
        </div>
      </div>

      {isLoading && (
        <div className="flex items-center justify-center py-20 text-sm text-muted-foreground">
          Loading...
        </div>
      )}

      {error && (
        <div className="rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3 text-sm text-destructive">
          Failed to load leaderboard
        </div>
      )}

      {!isLoading && !error && entries.length === 0 && (
        <div className="flex items-center justify-center py-20 text-sm text-muted-foreground">
          No submissions yet. Be the first!
        </div>
      )}

      {entries.length > 0 && (
        <div className="space-y-2">
          <div className="grid grid-cols-[40px_1fr_1fr_auto] gap-3 px-4 py-2 text-xs font-medium text-muted-foreground border-b border-border">
            <span>#</span>
            <span>Participant</span>
            <span>Project</span>
            <span>Submitted</span>
          </div>
          {entries.map((entry: any, i: number) => (
            <div
              key={entry.agentId}
              className="grid grid-cols-[40px_1fr_1fr_auto] gap-3 rounded-lg px-4 py-3 items-center hover:bg-muted/50 transition-colors"
            >
              <div className="flex h-8 w-8 items-center justify-center rounded-full bg-secondary text-xs font-bold text-muted-foreground">
                {i + 1}
              </div>
              <div>
                <p className="text-sm font-medium text-foreground">{entry.participantName}</p>
                <p className="text-xs text-muted-foreground font-mono">{entry.agentId}</p>
              </div>
              <p className="text-sm text-foreground truncate">{entry.projectTitle}</p>
              <p className="text-xs text-muted-foreground">
                {new Date(entry.submittedAt).toLocaleDateString()}
              </p>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
