import { createFileRoute } from "@tanstack/react-router";
import { Activity, AlertCircle, ArrowRight, Cable, CalendarClock, CheckCircle, Clock, Loader2, RefreshCw, XCircle } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { useApiClient } from "@/app";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";

export const Route = createFileRoute("/_layout/automations")({
  component: AutomationsPage,
});

type Automation = NonNullable<Awaited<ReturnType<ReturnType<typeof useApiClient>["ironclaw"]["automations"]["list"]>>["data"]>[number];
type OutboundTarget = NonNullable<Awaited<ReturnType<ReturnType<typeof useApiClient>["ironclaw"]["outbound"]["listTargets"]>>["data"]>[number];
type OutboundPrefs = Awaited<ReturnType<ReturnType<typeof useApiClient>["ironclaw"]["outbound"]["getPreferences"]>>;

function statusBadgeVariant(status: string | undefined): "default" | "secondary" | "destructive" {
  if (status === "active" || status === "success") return "default";
  if (status === "inactive" || status === "disabled" || status === "pending") return "secondary";
  if (status === "error" || status === "failed") return "destructive";
  return "secondary";
}

function StatusIcon({ status }: { status?: string }) {
  if (status === "active" || status === "success") return <CheckCircle className="size-3 text-[color:var(--near-green)]" />;
  if (status === "inactive" || status === "disabled") return <XCircle className="size-3 text-muted-foreground" />;
  if (status === "error" || status === "failed") return <AlertCircle className="size-3 text-destructive" />;
  if (status === "pending") return <Loader2 className="size-3 animate-spin text-muted-foreground" />;
  return <Activity className="size-3 text-muted-foreground" />;
}

function formatDateTime(iso?: string) {
  if (!iso) return null;
  try {
    const d = new Date(iso);
    return d.toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
  } catch {
    return iso;
  }
}

function AutomationCard({ automation }: { automation: Automation }) {
  const lastRun = automation.recentRuns?.[0];

  return (
    <Card className="space-y-3 p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1 space-y-1">
          <div className="flex items-center gap-2">
            <h3 className="truncate text-sm font-semibold text-foreground">{automation.name}</h3>
            <Badge variant={automation.isActive ? "default" : "secondary"}>
              <StatusIcon status={automation.isActive ? "active" : "inactive"} />
              {automation.isActive ? "Active" : "Disabled"}
            </Badge>
          </div>
          {automation.lastStatus && (
            <Badge variant={statusBadgeVariant(automation.lastStatus)}>
              <StatusIcon status={automation.lastStatus} />
              {automation.lastStatus}
            </Badge>
          )}
        </div>
      </div>

      <div className="grid grid-cols-1 gap-2 text-xs sm:grid-cols-2">
        {automation.source?.type === "schedule" && (
          <>
            <div className="flex items-center gap-1.5 text-muted-foreground">
              <Clock className="size-3 shrink-0" />
              <span className="font-mono">{automation.source.cron}</span>
            </div>
            <div className="flex items-center gap-1.5 text-muted-foreground">
              <CalendarClock className="size-3 shrink-0" />
              <span>{automation.source.timezone}</span>
            </div>
          </>
        )}
      </div>

      <div className="grid grid-cols-1 gap-2 text-xs sm:grid-cols-2">
        <div className="space-y-1">
          {automation.nextRunAt && (
            <div className="flex items-center gap-1.5 text-muted-foreground">
              <ArrowRight className="size-3 shrink-0" />
              <span>Next: {formatDateTime(automation.nextRunAt)}</span>
            </div>
          )}
        </div>
        <div className="space-y-1">
          {lastRun && (
            <div className="flex items-center gap-1.5 text-muted-foreground">
              <Activity className="size-3 shrink-0" />
              <span>
                Last: {formatDateTime(lastRun.submittedAt)}
                {lastRun.status && (
                  <Badge variant={statusBadgeVariant(lastRun.status)} className="ml-1.5">{lastRun.status}</Badge>
                )}
              </span>
            </div>
          )}
        </div>
      </div>
    </Card>
  );
}

function AutomationSkeleton() {
  return (
    <Card className="space-y-3 p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="flex-1 space-y-2">
          <Skeleton className="h-4 w-48" />
          <Skeleton className="h-5 w-16" />
        </div>
      </div>
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        <Skeleton className="h-3 w-32" />
        <Skeleton className="h-3 w-28" />
      </div>
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        <Skeleton className="h-3 w-36" />
        <Skeleton className="h-3 w-40" />
      </div>
    </Card>
  );
}

function OutboundPanel({
  prefs,
  targets,
  loading,
  onSave,
}: {
  prefs: OutboundPrefs | null;
  targets: OutboundTarget[];
  loading: boolean;
  onSave: (targetId: string) => Promise<void>;
}) {
  const [selectedTargetId, setSelectedTargetId] = useState(prefs?.finalReplyTarget?.targetId ?? "");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (prefs?.finalReplyTarget?.targetId) {
      setSelectedTargetId(prefs.finalReplyTarget.targetId);
    }
  }, [prefs]);

  const handleSave = useCallback(async () => {
    if (!selectedTargetId) return;
    setSaving(true);
    try {
      await onSave(selectedTargetId);
      toast.success("Delivery target updated");
    } catch {
      toast.error("Failed to update delivery target");
    } finally {
      setSaving(false);
    }
  }, [selectedTargetId, onSave]);

  const currentTarget = targets.find((t) => t.target.targetId === selectedTargetId);

  return (
    <Card className="space-y-4 p-5">
      <div className="flex items-center gap-2">
        <Cable className="size-4 text-muted-foreground" />
        <h2 className="text-sm font-semibold text-foreground">Outbound Delivery</h2>
      </div>

      <p className="text-xs text-muted-foreground">
        Select where final replies and prompts are delivered.
      </p>

      {prefs?.finalReplyTarget && (
        <div className="rounded-lg border border-border bg-muted/50 px-3 py-2 space-y-1">
          <p className="text-xs font-medium text-foreground">Current target</p>
          <p className="text-xs text-muted-foreground">
            {prefs.finalReplyTarget.displayName ?? prefs.finalReplyTarget.channel}{" "}
            {prefs.finalReplyTarget.description && (
              <span className="text-muted-foreground/60">— {prefs.finalReplyTarget.description}</span>
            )}
          </p>
          {prefs.status && (
            <div className="flex items-center gap-1.5 pt-1">
              <Badge variant={statusBadgeVariant(prefs.status)}>
                <StatusIcon status={prefs.status} />
                {prefs.status}
              </Badge>
              {prefs.modality && (
                <span className="text-xs text-muted-foreground">{prefs.modality}</span>
              )}
            </div>
          )}
        </div>
      )}

      <div className="flex flex-col gap-3 sm:flex-row sm:items-end">
        <div className="flex-1 space-y-1.5">
          <Select
            value={selectedTargetId}
            onValueChange={setSelectedTargetId}
            disabled={loading || targets.length === 0}
          >
            <SelectTrigger className="w-full">
              <SelectValue placeholder={loading ? "Loading targets..." : "Select a target"} />
            </SelectTrigger>
            <SelectContent>
              <SelectGroup>
                {targets.length === 0 && !loading && (
                  <div className="px-2 py-1.5 text-xs text-muted-foreground">No targets available</div>
                )}
                {targets.map((t) => (
                  <SelectItem key={t.target.targetId} value={t.target.targetId}>
                    <span className="flex items-center gap-2">
                      <span>{t.target.displayName}</span>
                      <span className="text-muted-foreground text-xs">({t.target.channel})</span>
                      {t.target.description && (
                        <span className="text-muted-foreground text-xs">— {t.target.description}</span>
                      )}
                    </span>
                  </SelectItem>
                ))}
              </SelectGroup>
            </SelectContent>
          </Select>
        </div>

        <Button
          onClick={handleSave}
          disabled={saving || !selectedTargetId || selectedTargetId === prefs?.finalReplyTarget?.targetId}
        >
          {saving ? <Loader2 className="size-3 animate-spin" /> : null}
          {saving ? "Saving..." : currentTarget ? "Switch target" : "Set target"}
        </Button>
      </div>
    </Card>
  );
}

function AutomationsPage() {
  const apiClient = useApiClient();

  const [automations, setAutomations] = useState<Automation[]>([]);
  const [automationsLoading, setAutomationsLoading] = useState(true);
  const [automationsError, setAutomationsError] = useState<boolean>(false);

  const [prefs, setPrefs] = useState<OutboundPrefs | null>(null);
  const [targets, setTargets] = useState<OutboundTarget[]>([]);
  const [outboundLoading, setOutboundLoading] = useState(true);

  const loadAutomations = useCallback(async () => {
    setAutomationsLoading(true);
    setAutomationsError(false);
    try {
      const result = await apiClient.ironclaw.automations.list({ limit: 50, runLimit: 5 });
      setAutomations(result.data);
    } catch {
      setAutomationsError(true);
    } finally {
      setAutomationsLoading(false);
    }
  }, [apiClient]);

  const loadOutbound = useCallback(async () => {
    setOutboundLoading(true);
    try {
      const [prefsResult, targetsResult] = await Promise.all([
        apiClient.ironclaw.outbound.getPreferences(),
        apiClient.ironclaw.outbound.listTargets(),
      ]);
      setPrefs(prefsResult);
      setTargets(targetsResult.data);
    } catch {
      toast.error("Failed to load outbound configuration");
    } finally {
      setOutboundLoading(false);
    }
  }, [apiClient]);

  useEffect(() => {
    loadAutomations();
    loadOutbound();
  }, [loadAutomations, loadOutbound]);

  const handleSaveTarget = useCallback(
    async (targetId: string) => {
      const target = targets.find((t) => t.target.targetId === targetId);
      if (!target) return;
      await apiClient.ironclaw.outbound.setPreferences({ finalReplyTarget: target.target });
      await loadOutbound();
    },
    [apiClient, loadOutbound, targets],
  );

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="space-y-6 p-6 max-w-5xl mx-auto">
      <div className="flex items-center gap-3">
        <div className="flex h-10 w-10 items-center justify-center rounded-full bg-primary/10">
          <Activity className="h-5 w-5 text-primary" />
        </div>
        <div className="flex-1 space-y-0.5">
          <h1 className="text-lg font-semibold text-foreground">Automations</h1>
          <p className="text-sm text-muted-foreground">
            Scheduled tasks and outbound delivery configuration.
          </p>
        </div>
        <Button
          variant="outline"
          size="icon"
          className="h-8 w-8 shrink-0"
          onClick={() => { loadAutomations(); loadOutbound(); }}
          disabled={automationsLoading || outboundLoading}
          title="Refresh automations"
        >
          <RefreshCw className={`size-4 ${automationsLoading ? "animate-spin" : ""}`} />
        </Button>
      </div>

      <OutboundPanel
        prefs={prefs}
        targets={targets}
        loading={outboundLoading}
        onSave={handleSaveTarget}
      />

      <div className="space-y-3">
        <h2 className="text-sm font-semibold text-foreground">Scheduled automations</h2>

        {automationsLoading ? (
          <div className="space-y-3">
            <AutomationSkeleton />
            <AutomationSkeleton />
            <AutomationSkeleton />
          </div>
        ) : automationsError ? (
          <Card className="flex flex-col items-center gap-3 p-6 text-center">
            <AlertCircle className="size-8 text-destructive" />
            <div className="space-y-1">
              <p className="text-sm font-medium text-foreground">Failed to load automations</p>
              <p className="text-xs text-muted-foreground">
                Something went wrong. Check your connection and try again.
              </p>
            </div>
            <Button variant="outline" size="sm" onClick={loadAutomations}>
              <RefreshCw className="mr-1.5 size-3" />
              Retry
            </Button>
          </Card>
        ) : automations.length === 0 ? (
          <Card className="flex flex-col items-center gap-3 p-6 text-center">
            <Clock className="size-8 text-muted-foreground" />
            <div className="space-y-1">
              <p className="text-sm font-medium text-foreground">No automations yet</p>
              <p className="text-xs text-muted-foreground">
                Create a scheduled automation to have IronClaw run tasks automatically.
              </p>
            </div>
          </Card>
        ) : (
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            {automations.map((a) => (
              <AutomationCard key={a.id} automation={a} />
            ))}
          </div>
        )}
      </div>
    </div>
    </div>
  );
}
