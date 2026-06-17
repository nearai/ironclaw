import { ExternalLink } from "lucide-react";
import { useCallback, useMemo } from "react";
import type { AuthGate } from "@/hooks/use-thread-chat-manager";
import { Button } from "@/components/ui/button";

interface AuthOauthCardProps {
  gate: AuthGate;
  onCancel: () => void;
}

export function AuthOauthCard({ gate, onCancel }: AuthOauthCardProps) {
  const hasHttpsUrl = useMemo(() => {
    if (!gate.authorizationUrl) return false;
    try {
      return new URL(gate.authorizationUrl).protocol === "https:";
    } catch {
      return false;
    }
  }, [gate.authorizationUrl]);

  const handleOpen = useCallback(() => {
    if (!hasHttpsUrl) return;
    window.open(gate.authorizationUrl!, "_blank", "noopener,noreferrer");
  }, [gate.authorizationUrl, hasHttpsUrl]);

  const subtitle = gate.accountLabel || gate.provider || "";

  return (
    <div className="mx-auto w-full max-w-lg rounded-xl border border-sky-500/30 bg-sky-500/5 p-4">
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-8 w-8 place-items-center rounded-md border border-sky-500/25 bg-sky-500/10 text-sky-400">
          <ExternalLink size={16} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="font-semibold text-foreground truncate">
            {gate.headline || "Authorize via OAuth"}
          </div>
          {subtitle && (
            <div className="text-xs text-muted-foreground truncate">{subtitle}</div>
          )}
        </div>
      </div>

      {gate.body && (
        <div className="mb-3 text-sm text-muted-foreground">{gate.body}</div>
      )}

      <div className="mb-3 text-xs text-muted-foreground">
        Authorization page will open in a new tab.
      </div>

      {gate.expiresAt && (
        <div className="mb-3 text-xs text-muted-foreground">
          Expires: {new Date(gate.expiresAt).toLocaleString()}
        </div>
      )}

      <div className="flex flex-wrap gap-2">
        <Button
          variant="default"
          size="sm"
          disabled={!hasHttpsUrl}
          onClick={handleOpen}
        >
          Open Authorization Page
        </Button>
        <Button
          variant="secondary"
          size="sm"
          onClick={onCancel}
        >
          Cancel
        </Button>
      </div>
    </div>
  );
}
