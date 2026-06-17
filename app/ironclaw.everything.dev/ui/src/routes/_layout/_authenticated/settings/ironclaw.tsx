import { createFileRoute, Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { Cloud, Key, Loader2, RefreshCw, Save, Terminal } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { type SessionData, sessionQueryOptions, useApiClient, useAuthClient } from "@/app";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useConnectionMode } from "@/hooks/use-connection-mode";
import { useIronclawStatus } from "@/hooks/use-ironclaw-status";

export const Route = createFileRoute("/_layout/_authenticated/settings/ironclaw")({
  component: IronclawSettings,
});

function IronclawSettings() {
  const apiClient = useApiClient();
  const auth = useAuthClient();
  const { data: session } = useQuery<SessionData | null>(sessionQueryOptions(auth));
  const { connectionMode } = useConnectionMode();
  const { status: connectionStatus, refetch: refetchStatus } = useIronclawStatus();
  const [baseUrl, setBaseUrl] = useState("");
  const [apiToken, setApiToken] = useState("");
  const [tokenConfigured, setTokenConfigured] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [disconnecting, setDisconnecting] = useState(false);
  const [testingConnection, setTestingConnection] = useState(false);
  const [hasSettings, setHasSettings] = useState(false);
  const [scope, setScope] = useState<"personal" | "organization">("personal");

  const activeOrgId = session?.session?.activeOrganizationId;
  const hasOrg = !!activeOrgId;

  useEffect(() => {
    if (connectionMode === "hosted") {
      setLoading(false);
      return;
    }
    apiClient.ironclaw.settings
      .get({ scope })
      .then((res) => {
        setBaseUrl(res.baseUrl);
        setTokenConfigured(res.hasToken ?? false);
        setHasSettings(true);
        if (res.scope === "organization" || res.scope === "personal") {
          setScope(res.scope);
        }
      })
      .catch(() => {
        setHasSettings(false);
      })
      .finally(() => setLoading(false));
  }, [apiClient, connectionMode, scope]);

  const handleTestConnection = async () => {
    setTestingConnection(true);
    try {
      await apiClient.ironclaw.ping();
      toast.success("Connection successful — binary is reachable");
    } catch {
      toast.error("Connection failed — check your tunnel URL and API token");
    } finally {
      setTestingConnection(false);
    }
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    setSaving(true);
    try {
      await apiClient.ironclaw.settings.update({
        baseUrl,
        scope,
        ...(apiToken ? { apiToken } : {}),
      });
      setHasSettings(true);
      if (apiToken) setTokenConfigured(true);
      toast.success("IronClaw settings saved");
    } catch (err: any) {
      toast.error(err.message ?? "Failed to save settings");
    } finally {
      setSaving(false);
    }
  };

  const handleDisconnect = async () => {
    setDisconnecting(true);
    try {
      await apiClient.ironclaw.settings.delete({ scope });
      setBaseUrl("");
      setApiToken("");
      setTokenConfigured(false);
      setHasSettings(false);
      refetchStatus();
      toast.success("Disconnected from tunnel");
    } catch (err: any) {
      toast.error(err.message ?? "Failed to disconnect");
    } finally {
      setDisconnecting(false);
    }
  };

  const isConnected = connectionStatus === "connected";
  const canTest = baseUrl && (apiToken || tokenConfigured);
  const canSave = baseUrl && (apiToken || tokenConfigured);

  return (
    <div className="space-y-6">
      <div className="space-y-1">
        <h2 className="text-lg font-semibold text-foreground">IronClaw Connection</h2>
        <p className="text-sm text-muted-foreground">
          Connect to a tunneled or deployed IronClaw agent.
        </p>
      </div>

      <div className="flex items-center gap-3 rounded-lg border border-border bg-muted/50 px-4 py-3">
        <div
          className={`h-2 w-2 rounded-full shrink-0 ${
            isConnected
              ? "bg-[color:var(--near-green)]"
              : connectionStatus === "disconnected"
                ? "bg-destructive"
                : "bg-muted-foreground"
          }`}
        />
        <span className="text-xs text-muted-foreground flex-1">
          {isConnected
            ? `Connected via ${connectionMode === "hosted" ? "hosted agent" : "local binary"}`
            : connectionStatus === "disconnected"
              ? "Connection lost"
              : "Not connected"}
        </span>
        <span className="rounded-full bg-secondary px-2 py-0.5 text-xs text-muted-foreground">
          Mode: {connectionMode.charAt(0).toUpperCase() + connectionMode.slice(1)}
        </span>
        <button
          type="button"
          onClick={() => {
            refetchStatus();
          }}
          className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors"
        >
          <RefreshCw size={10} />
          Refresh
        </button>
      </div>

      {connectionMode === "hosted" ? (
        <Card className="p-5 space-y-4">
          <div className="flex items-start gap-3">
            <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-secondary">
              <Cloud size={14} className="text-muted-foreground" />
            </div>
            <div>
              <p className="text-sm font-semibold text-foreground">Hosted Agent</p>
              <p className="mt-1 text-sm text-muted-foreground">
                Using the shared hosted agent. Manage API keys from{" "}
                <Link to="/settings/api-keys" className="text-primary underline underline-offset-2">
                  Settings → API Keys
                </Link>.
              </p>
            </div>
          </div>
        </Card>
      ) : loading ? (
        <div className="flex items-center justify-center py-12">
          <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
        </div>
      ) : (
        <form onSubmit={handleSave} className="space-y-4">
          <Card className="space-y-4 p-5">
            <div className="flex items-center gap-2 pb-4 border-b border-border">
              <span className="text-xs text-muted-foreground">Configuring:</span>
              <div className="flex rounded-md border border-border overflow-hidden">
                <button
                  type="button"
                  onClick={() => setScope("personal")}
                  className={`px-3 py-1 text-xs font-medium transition-colors ${
                    scope === "personal"
                      ? "bg-primary text-primary-foreground"
                      : "bg-background text-muted-foreground hover:text-foreground"
                  }`}
                >
                  Personal
                </button>
                <button
                  type="button"
                  onClick={() => hasOrg && setScope("organization")}
                  disabled={!hasOrg}
                  className={`px-3 py-1 text-xs font-medium transition-colors ${
                    scope === "organization"
                      ? "bg-primary text-primary-foreground"
                      : !hasOrg
                        ? "bg-background text-muted-foreground/40 cursor-not-allowed"
                        : "bg-background text-muted-foreground hover:text-foreground"
                  }`}
                  title={!hasOrg ? "You are not a member of an organization" : undefined}
                >
                  Organization
                </button>
              </div>
              {!hasOrg && <span className="text-xs text-muted-foreground/60">(no active org)</span>}
            </div>

            <div className="space-y-2">
              <Label htmlFor="tunnelUrl" className="flex items-center gap-1.5">
                <Terminal size={14} />
                Tunnel URL
              </Label>
              <Input
                id="tunnelUrl"
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
                placeholder="https://your-tunnel.ngrok.io"
                required
              />
              <p className="text-xs font-medium text-muted-foreground">
                The Tunnel URL printed by <code className="rounded bg-secondary px-1 py-0.5 font-mono text-xs">scripts/bos-dev.sh --tunnel</code> (e.g. https://xxx.ngrok.io).
              </p>
            </div>

            <div className="space-y-2">
              <Label htmlFor="apiToken" className="flex items-center gap-1.5">
                <Key size={14} />
                API Token
              </Label>
              <Input
                id="apiToken"
                type="password"
                value={apiToken}
                onChange={(e) => setApiToken(e.target.value)}
                placeholder={
                  tokenConfigured ? "Token is configured" : "Token printed by the setup script"
                }
                required={!tokenConfigured}
              />
              <p className="text-xs font-medium text-muted-foreground">
                {tokenConfigured
                  ? "Token is configured. Leave empty to keep it."
                  : "The API Token printed by the setup script."}
              </p>
            </div>
          </Card>

          <div className="flex items-center justify-between gap-4">
            {!hasSettings && (
              <p className="text-xs font-medium text-muted-foreground">
                No settings configured yet. Add your tunnel URL and API token to connect.
              </p>
            )}
            <div className="flex items-center gap-2 ml-auto">
              {hasSettings && (
                <Button
                  type="button"
                  variant="outline"
                  disabled={disconnecting}
                  onClick={handleDisconnect}
                  className="text-destructive hover:text-destructive"
                >
                  {disconnecting ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <Cloud size={14} />
                  )}
                  {disconnecting ? "Disconnecting..." : "Disconnect"}
                </Button>
              )}
              <Button
                type="button"
                variant="outline"
                disabled={testingConnection || !canTest}
                onClick={handleTestConnection}
              >
                {testingConnection ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <RefreshCw size={14} />
                )}
                {testingConnection ? "Testing..." : "Test connection"}
              </Button>
              <Button type="submit" disabled={saving || !canSave}>
                {saving ? <Loader2 className="h-4 w-4 animate-spin" /> : <Save size={14} />}
                {saving ? "Saving..." : "Save settings"}
              </Button>
            </div>
          </div>
        </form>
      )}

      <div className="rounded-lg border border-border bg-muted/50 px-4 py-3 space-y-1.5">
        <p className="text-xs font-medium text-foreground">How to set up a tunnel</p>
        <p className="text-xs font-medium text-muted-foreground">
          From the repo root, run{" "}
          <code className="rounded bg-secondary px-1 py-0.5 font-mono text-xs">
            ./scripts/bos-dev.sh --tunnel
          </code>
          {" "}— it prints the Tunnel URL and API Token. Paste them above.
        </p>
      </div>
    </div>
  );
}
