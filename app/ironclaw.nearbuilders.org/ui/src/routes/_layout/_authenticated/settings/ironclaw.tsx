import { createFileRoute } from "@tanstack/react-router";
import { Key, Loader2, Save, Terminal } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { useApiClient } from "@/app";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export const Route = createFileRoute("/_layout/_authenticated/settings/ironclaw")({
  component: IronclawSettings,
});

function IronclawSettings() {
  const apiClient = useApiClient();
  const [tunnelUrl, setTunnelUrl] = useState("");
  const [apiToken, setApiToken] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [hasSettings, setHasSettings] = useState(false);

  useEffect(() => {
    apiClient.ironclaw.settings
      .get()
      .then((res) => {
        setTunnelUrl(res.tunnelUrl);
        setApiToken(res.apiToken);
        setHasSettings(true);
      })
      .catch(() => {
        setHasSettings(false);
      })
      .finally(() => setLoading(false));
  }, [apiClient]);

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    setSaving(true);
    try {
      await apiClient.ironclaw.settings.update({
        tunnelUrl,
        apiToken,
      });
      setHasSettings(true);
      toast.success("IronClaw settings saved");
    } catch (err: any) {
      toast.error(err.message ?? "Failed to save settings");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="space-y-6">
      <div className="space-y-1">
        <h2 className="text-lg font-semibold text-foreground">IronClaw Connection</h2>
        <p className="text-sm text-muted-foreground">
          Configure how this dashboard connects to your ironclaw binary.
        </p>
      </div>

      {loading ? (
        <div className="flex items-center justify-center py-12">
          <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
        </div>
      ) : (
        <form onSubmit={handleSave} className="space-y-4">
          <Card className="space-y-4 p-5">
            <div className="space-y-2">
              <Label htmlFor="tunnelUrl" className="flex items-center gap-1.5">
                <Terminal size={14} />
                Tunnel URL
              </Label>
              <Input
                id="tunnelUrl"
                value={tunnelUrl}
                onChange={(e) => setTunnelUrl(e.target.value)}
                placeholder="https://your-tunnel.ngrok.io"
                required
              />
              <p className="text-xs text-muted-foreground">
                Public URL pointing to your ironclaw reborn binary (e.g. via ngrok, Cloudflare
                Tunnel). Must expose the WebChat v2 API.
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
                placeholder="The bearer token your binary expects"
                required
              />
              <p className="text-xs text-muted-foreground">
                Must match the bearer token configured on your Reborn binary.
              </p>
            </div>
          </Card>

          <div className="flex items-center justify-between gap-4">
            {!hasSettings && (
              <p className="text-xs text-muted-foreground">
                No settings configured yet. Add your tunnel URL and API token to connect.
              </p>
            )}
            <Button type="submit" disabled={saving || !tunnelUrl || !apiToken} className="ml-auto">
              {saving ? <Loader2 className="h-4 w-4 animate-spin" /> : <Save size={14} />}
              {saving ? "Saving..." : "Save settings"}
            </Button>
          </div>
        </form>
      )}

      <div className="rounded-lg border border-border bg-muted/50 px-4 py-3 space-y-1.5">
        <p className="text-xs font-medium text-foreground">How to set up a tunnel</p>
        <p className="text-xs text-muted-foreground">
          Run your ironclaw binary locally, then expose it with ngrok:
        </p>
        <code className="block rounded bg-secondary px-2 py-1.5 text-xs font-mono text-foreground">
          ngrok http http://localhost:3001
        </code>
        <p className="text-xs text-muted-foreground">
          Copy the ngrok URL into the Tunnel URL field above. The API Token must match
          the bearer token configured on your Reborn binary.
        </p>
      </div>
    </div>
  );
}
