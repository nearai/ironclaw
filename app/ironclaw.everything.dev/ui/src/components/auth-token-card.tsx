import { KeyRound } from "lucide-react";
import { useState, useCallback } from "react";
import type { AuthGate } from "@/hooks/use-thread-chat-manager";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

interface AuthTokenCardProps {
  gate: AuthGate;
  onSubmit: (token: string) => Promise<void>;
  onCancel: () => void;
}

export function AuthTokenCard({ gate, onSubmit, onCancel }: AuthTokenCardProps) {
  const [token, setToken] = useState("");
  const [submitting, setSubmitting] = useState(false);

  const handleSubmit = useCallback(async () => {
    if (!token.trim() || submitting) return;
    setSubmitting(true);
    try {
      await onSubmit(token.trim());
    } finally {
      setSubmitting(false);
    }
  }, [token, submitting, onSubmit]);

  const subtitle = gate.accountLabel || gate.provider || "";

  return (
    <div className="mx-auto w-full max-w-lg rounded-xl border border-sky-500/30 bg-sky-500/5 p-4">
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-8 w-8 place-items-center rounded-md border border-sky-500/25 bg-sky-500/10 text-sky-400">
          <KeyRound size={16} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="font-semibold text-foreground truncate">
            {gate.headline || "Authentication Required"}
          </div>
          {subtitle && (
            <div className="text-xs text-muted-foreground truncate">{subtitle}</div>
          )}
        </div>
      </div>

      {gate.body && (
        <div className="mb-3 text-sm text-muted-foreground">{gate.body}</div>
      )}

      <div className="flex flex-col gap-2">
        <Input
          type="password"
          placeholder="Enter API key or token..."
          value={token}
          onChange={(e) => setToken(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") handleSubmit();
          }}
          disabled={submitting}
        />
        <div className="flex gap-2">
          <Button
            variant="default"
            size="sm"
            onClick={handleSubmit}
            disabled={!token.trim() || submitting}
          >
            {submitting ? "Submitting..." : "Submit"}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={onCancel}
            disabled={submitting}
          >
            Cancel
          </Button>
        </div>
      </div>
    </div>
  );
}
