import { Lock } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import type { PendingApproval } from "@/hooks/use-thread-chat-manager";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";

const WRITE_RE = /(write|edit|delete|remove|patch|create|move|rename|chmod|rm\b)/;
const EXEC_RE = /(bash|shell|exec|run|command|terminal|spawn|process)/;
const NETWORK_RE = /(curl|http|fetch|web|network|request|api|gh\b|git|download|upload|browse)/;

function classifyRisk(toolName?: string): { variant: "destructive" | "default" | "secondary" | "outline"; label: string } {
  const name = String(toolName ?? "").toLowerCase();
  if (WRITE_RE.test(name)) return { variant: "destructive", label: "Write" };
  if (EXEC_RE.test(name)) return { variant: "default", label: "Exec" };
  if (NETWORK_RE.test(name)) return { variant: "secondary", label: "Network" };
  return { variant: "outline", label: "Read" };
}

interface ApprovalCardProps {
  approval: PendingApproval;
  onApprove: () => void;
  onDeny: () => void;
  onAlways?: () => void;
}

export function ApprovalCard({ approval, onApprove, onDeny, onAlways }: ApprovalCardProps) {
  const [always, setAlways] = useState(false);
  const risk = useMemo(() => classifyRisk(approval.toolName), [approval.toolName]);

  const handlePrimary = useCallback(() => {
    if (always && approval.allowAlways && onAlways) {
      onAlways();
    } else {
      onApprove();
    }
  }, [always, approval.allowAlways, onAlways, onApprove]);

  const details = useMemo(() => {
    const items: Array<{ label: string; value: string }> = [];
    if (approval.action?.label) items.push({ label: "Action", value: approval.action.label });
    if (approval.destination?.label) items.push({ label: "Destination", value: approval.destination.label });
    if (approval.scope?.label) items.push({ label: "Scope", value: approval.scope.label });
    if (approval.details) {
      for (const detail of approval.details) {
        if (detail?.label && detail.value != null) items.push({ label: detail.label, value: detail.value });
      }
    }
    return items;
  }, [approval.action, approval.destination, approval.scope, approval.details]);

  return (
    <div className="mx-auto w-full max-w-lg rounded-xl border border-amber-500/30 bg-amber-500/5 p-4">
      <div className="mb-3 flex items-center gap-2">
        <span className="grid h-8 w-8 place-items-center rounded-md border border-amber-500/25 bg-amber-500/10 text-amber-500">
          <Lock size={16} />
        </span>
        <span className="font-semibold text-foreground">Approval Required</span>
        <Badge variant={risk.variant} className="ml-auto">
          {risk.label}
        </Badge>
      </div>

      {approval.toolName && (
        <div className="mb-1 break-all font-mono text-sm font-medium text-foreground">
          {approval.toolName}
        </div>
      )}

      {approval.description && (
        <div className="mb-3 break-words text-sm text-muted-foreground">
          {approval.description}
        </div>
      )}

      {details.length > 0 && (
        <dl className="mb-3 max-h-56 overflow-y-auto rounded-md border border-border bg-card text-xs">
          {details.map((detail, i) => (
            <div
              key={i}
              className="grid gap-1 border-b border-border/70 px-3 py-2 last:border-b-0 sm:grid-cols-[7rem_1fr]"
            >
              <dt className="font-medium text-muted-foreground">{detail.label}</dt>
              <dd className="min-w-0 break-all font-mono text-foreground">{detail.value}</dd>
            </div>
          ))}
        </dl>
      )}

      {approval.allowAlways && (
        <label className="mb-3 flex items-center gap-2 text-xs text-muted-foreground">
          <Checkbox
            checked={always}
            onCheckedChange={(checked) => setAlways(checked === true)}
          />
          Always allow {approval.toolName ?? "this tool"}
        </label>
      )}

      <div className="flex flex-wrap gap-2">
        <Button
          variant="default"
          size="sm"
          onClick={handlePrimary}
        >
          {always && approval.allowAlways ? "Approve & Always Allow" : "Approve"}
        </Button>
        <Button
          variant="secondary"
          size="sm"
          onClick={onDeny}
        >
          Deny
        </Button>
      </div>
    </div>
  );
}
