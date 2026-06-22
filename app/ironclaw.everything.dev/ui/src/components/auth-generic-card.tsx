import { ShieldAlert } from "lucide-react";
import type { AuthGate } from "@/hooks/use-thread-chat-manager";
import { Button } from "@/components/ui/button";

interface AuthGenericCardProps {
  gate: AuthGate;
  onCancel: () => void;
}

export function AuthGenericCard({ gate, onCancel }: AuthGenericCardProps) {
  return (
    <div className="mx-auto w-full max-w-lg rounded-xl border border-amber-500/30 bg-amber-500/5 p-4">
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-8 w-8 place-items-center rounded-md border border-amber-500/25 bg-amber-500/10 text-amber-500">
          <ShieldAlert size={16} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="font-semibold text-foreground truncate">
            {gate.headline || "Authentication Required"}
          </div>
          {gate.provider && (
            <div className="text-xs text-muted-foreground truncate">{gate.provider}</div>
          )}
        </div>
      </div>

      {gate.body && (
        <div className="mb-3 text-sm text-muted-foreground">{gate.body}</div>
      )}

      <div className="mb-3 text-xs text-muted-foreground">
        Open settings to complete this authentication step.
      </div>

      <div className="flex flex-wrap gap-2">
        <Button variant="secondary" size="sm" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </div>
  );
}
