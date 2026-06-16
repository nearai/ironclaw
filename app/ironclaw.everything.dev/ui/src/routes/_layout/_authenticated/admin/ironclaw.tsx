import { createFileRoute } from "@tanstack/react-router";
import { Cloud, Key, Loader2, Save, Terminal } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { useApiClient } from "@/app";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export const Route = createFileRoute("/_layout/_authenticated/admin/ironclaw")({
  component: AdminIronclaw,
  head: () => ({
    title: "Admin | IronClaw Default",
    meta: [
      {
        name: "description",
        content: "Configure the platform-wide default IronClaw tunnel.",
      },
    ],
  }),
});

function AdminIronclaw() {
  const apiClient = useApiClient();
  const [tunnelUrl, setTunnelUrl] = useState("");
  const [apiToken, setApiToken] = useState("");
  const [tokenConfigured, setTokenConfigured] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [disconnecting, setDisconnecting] = useState(false);
  const [hasSettings, setHasSettings] = useState(false);

  useEffect(() => {
    apiClient.ironclaw.settings
      .get({ scope: "platform" })
      .then((res) => {
        setTunnelUrl(res.tunnelUrl);
        setTokenConfigured(res.hasToken ?? false);
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
        ...(apiToken ? { apiToken } : {}),
        scope: "platform",
      });
      setHasSettings(true);
      if (apiToken) setTokenConfigured(true);
      toast.success("IronClaw platform default saved");
    } catch (err: any) {
      toast.error(err.message ?? "Failed to save settings");
    } finally {
      setSaving(false);
    }
  };

  const handleDisconnect = async () => {
    setDisconnecting(true);
    try {
      await apiClient.ironclaw.settings.delete({ scope: "platform" });
      setTunnelUrl("");
      setApiToken("");
      setTokenConfigured(false);
      setHasSettings(false);
      toast.success("Disconnected from platform tunnel");
    } catch (err: any) {
      toast.error(err.message ?? "Failed to disconnect");
    } finally {
      setDisconnecting(false);
    }
  };

  return (
    <div className="space-y-6">
      <div className="space-y-1">
        <h2 className="text-lg font-semibold text-foreground">IronClaw Default Tunnel</h2>
        <p className="text-sm text-muted-foreground">
          Configure the platform-wide default tunnel for all users without a personal or org tunnel.
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
                Tunnel). This is the platform-wide default tunnel. All users without a personal or
                org tunnel will use this.
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
                  tokenConfigured ? "Token is configured" : "The bearer token your binary expects"
                }
                required={!tokenConfigured}
              />
              <p className="text-xs text-muted-foreground">
                {tokenConfigured
                  ? "Token is already configured. Leave empty to keep the existing token."
                  : "Must match the bearer token configured on your Reborn binary."}
              </p>
            </div>
          </Card>

          <div className="flex items-center justify-between gap-4">
            {!hasSettings && (
              <p className="text-xs text-muted-foreground">
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
              <Button type="submit" disabled={saving || !tunnelUrl}>
                {saving ? <Loader2 className="h-4 w-4 animate-spin" /> : <Save size={14} />}
                {saving ? "Saving..." : "Save settings"}
              </Button>
            </div>
          </div>
        </form>
      )}

      <div className="rounded-lg border border-border bg-muted/50 px-4 py-3 space-y-1.5">
        <p className="text-xs font-medium text-foreground">About platform-wide defaults</p>
        <p className="text-xs text-muted-foreground">
          This configuration is used as the default tunnel for all users who have not configured a
          personal or organization-specific tunnel. Only admins can configure this setting.
        </p>
      </div>
    </div>
  );
}
