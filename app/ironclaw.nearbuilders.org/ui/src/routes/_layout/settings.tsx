import { createFileRoute, redirect } from "@tanstack/react-router";
import { Cable, Cog, Globe, Loader2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { useApiClient, sessionQueryOptions } from "@/app";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";

type Tab = "outbound" | "automations" | "extensions";

export const Route = createFileRoute("/_layout/settings")({
  beforeLoad: async ({ context, location }) => {
    const { queryClient, authClient, session } = context;
    const current = await queryClient.ensureQueryData(
      sessionQueryOptions(authClient, session),
    );
    if (!current?.user) {
      throw redirect({ to: "/login", search: { redirect: location.pathname } });
    }
    return { session: current };
  },
  component: SettingsPage,
});

const tabs: { id: Tab; label: string; icon: typeof Cog }[] = [
  { id: "outbound", label: "Outbound", icon: Globe },
  { id: "automations", label: "Automations", icon: Cog },
  { id: "extensions", label: "Extensions", icon: Cable },
];

function TabNav({ active, onSelect }: { active: Tab; onSelect: (t: Tab) => void }) {
  return (
    <div className="flex gap-1 border-b border-border px-4">
      {tabs.map((t) => {
        const Icon = t.icon;
        return (
          <button
            key={t.id}
            type="button"
            onClick={() => onSelect(t.id)}
            className={`flex items-center gap-2 px-4 py-2.5 text-sm font-medium transition-colors border-b-2 -mb-px ${
              active === t.id
                ? "border-foreground text-foreground"
                : "border-transparent text-muted-foreground hover:text-foreground"
            }`}
          >
            <Icon size={14} />
            {t.label}
          </button>
        );
      })}
    </div>
  );
}

function SettingsPage() {
  const apiClient = useApiClient();
  const [activeTab, setActiveTab] = useState<Tab>("outbound");

  return (
    <div className="flex h-full flex-col">
      <TabNav active={activeTab} onSelect={setActiveTab} />
      <ScrollArea className="flex-1">
        <div className="mx-auto max-w-3xl space-y-6 p-6">
          {activeTab === "outbound" && <OutboundTab apiClient={apiClient} />}
          {activeTab === "automations" && <AutomationsTab apiClient={apiClient} />}
          {activeTab === "extensions" && <ExtensionsTab apiClient={apiClient} />}
        </div>
      </ScrollArea>
    </div>
  );
}

function OutboundTab({ apiClient }: { apiClient: ReturnType<typeof useApiClient> }) {
  const [targetId, setTargetId] = useState("");
  const [channel, setChannel] = useState("");
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    (async () => {
      try {
        const prefs = await apiClient.ironclaw.outbound.getPreferences();
        setTargetId(prefs.finalReplyTarget?.targetId ?? "");
        setChannel(prefs.finalReplyTarget?.channel ?? "");
      } catch {
        toast.error("Failed to load outbound preferences");
      } finally {
        setIsLoading(false);
      }
    })();
  }, [apiClient]);

  const save = useCallback(async () => {
    try {
      await apiClient.ironclaw.outbound.setPreferences({
        finalReplyTarget: targetId
          ? { targetId, channel: channel || "slack", displayName: "" }
          : undefined,
      });
      toast.success("Outbound preferences saved");
    } catch {
      toast.error("Failed to save preferences");
    }
  }, [apiClient, targetId, channel]);

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-lg font-semibold">Outbound Delivery</h2>
        <p className="text-sm text-muted-foreground">
          Configure where final replies are sent (Slack, Telegram, etc.)
        </p>
      </div>
      <div className="space-y-4">
        <div className="space-y-2">
          <label className="text-xs font-medium text-muted-foreground">Target ID</label>
          <Input
            value={targetId}
            onChange={(e) => setTargetId(e.target.value)}
            placeholder="Channel target ID"
          />
        </div>
        <div className="space-y-2">
          <label className="text-xs font-medium text-muted-foreground">Channel</label>
          <Input
            value={channel}
            onChange={(e) => setChannel(e.target.value)}
            placeholder="slack, telegram, etc."
          />
        </div>
        <Button onClick={save}>Save</Button>
      </div>
    </div>
  );
}

function AutomationsTab({ apiClient }: { apiClient: ReturnType<typeof useApiClient> }) {
  const [automations, setAutomations] = useState<Array<{ id: string; name?: string; status?: string }>>([]);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    (async () => {
      try {
        const result = await apiClient.ironclaw.automations.list({});
        setAutomations(result.data);
      } catch {
        toast.error("Failed to load automations");
      } finally {
        setIsLoading(false);
      }
    })();
  }, [apiClient]);

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-lg font-semibold">Automations</h2>
        <p className="text-sm text-muted-foreground">Triggered workflows and scheduled tasks</p>
      </div>
      {automations.length === 0 ? (
        <p className="text-sm text-muted-foreground">No automations configured.</p>
      ) : (
        <div className="space-y-2">
          {automations.map((a) => (
            <div
              key={a.id}
              className="flex items-center justify-between rounded-lg border border-border px-4 py-3"
            >
              <div>
                <p className="text-sm font-medium">{a.name ?? a.id}</p>
                <p className="text-xs text-muted-foreground">{a.status}</p>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function ExtensionsTab({ apiClient }: { apiClient: ReturnType<typeof useApiClient> }) {
  const [name, setName] = useState("");
  const [action, setAction] = useState("install");
  const [payload, setPayload] = useState("");
  const [result, setResult] = useState<{ success: boolean; message?: string } | null>(null);
  const [isLoading, setIsLoading] = useState(false);

  const run = useCallback(async () => {
    if (!name.trim()) return;
    setIsLoading(true);
    setResult(null);
    try {
      const res = await apiClient.ironclaw.extensions.setup({
        name: name.trim(),
        action,
        payload: payload ? JSON.parse(payload) : undefined,
      });
      setResult(res);
    } catch {
      toast.error("Extension setup failed");
    } finally {
      setIsLoading(false);
    }
  }, [apiClient, name, action, payload]);

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-lg font-semibold">Extensions</h2>
        <p className="text-sm text-muted-foreground">
          Install, configure, or remove WASM extensions and MCP servers
        </p>
      </div>
      <div className="space-y-4">
        <div className="space-y-2">
          <label className="text-xs font-medium text-muted-foreground">Extension Name</label>
          <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. my-channel" />
        </div>
        <div className="space-y-2">
          <label className="text-xs font-medium text-muted-foreground">Action</label>
          <select
            value={action}
            onChange={(e) => setAction(e.target.value)}
            className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm"
          >
            <option value="install">install</option>
            <option value="configure">configure</option>
            <option value="remove">remove</option>
          </select>
        </div>
        <div className="space-y-2">
          <label className="text-xs font-medium text-muted-foreground">
            Payload <span className="text-muted-foreground/60">(JSON, optional)</span>
          </label>
          <textarea
            value={payload}
            onChange={(e) => setPayload(e.target.value)}
            placeholder='{"key": "value"}'
            rows={4}
            className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm font-mono"
          />
        </div>
        <Button onClick={run} disabled={isLoading || !name.trim()}>
          {isLoading && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
          Run Setup
        </Button>
          {result && (
            <div
              className={`rounded-lg border px-4 py-3 text-sm ${
                result.success
                  ? "border-green-500/30 bg-green-500/5 text-green-600"
                  : "border-destructive/30 bg-destructive/5 text-destructive"
              }`}
            >
              {result.message ?? (result.success ? "Success" : "Failed")}
            </div>
          )}
        </div>
      </div>
    );
  }
