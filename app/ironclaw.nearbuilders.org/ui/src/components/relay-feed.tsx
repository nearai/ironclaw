import { useQuery } from "@tanstack/react-query";
import type { RelayedTransactionT } from "better-near-auth";
import { CheckCircle2, Clock, ExternalLink, Loader2, XCircle } from "lucide-react";
import { sessionQueryOptions, useAuthClient, useRelayHistory } from "@/app";
import { Badge } from "@/components/ui/badge";

function explorerTxUrl(txHash: string): string {
  return `https://near.rocks/tx/${txHash}`;
}

function truncateHash(hash: string): string {
  if (hash.length <= 16) return hash;
  return `${hash.slice(0, 6)}...${hash.slice(-6)}`;
}

function timeAgo(dateStr: string): string {
  const date = new Date(dateStr);
  const now = new Date();
  const seconds = Math.floor((now.getTime() - date.getTime()) / 1000);
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

function StatusIcon({ status }: { status: string }) {
  switch (status) {
    case "completed":
      return <CheckCircle2 className="h-4 w-4 text-green-600" />;
    case "failed":
      return <XCircle className="h-4 w-4 text-destructive" />;
    default:
      return <Loader2 className="h-4 w-4 text-muted-foreground animate-spin" />;
  }
}

function StatusBadge({ status }: { status: string }) {
  switch (status) {
    case "completed":
      return (
        <Badge variant="secondary" className="bg-green-500/10 text-green-600 border-green-500/20">
          Completed
        </Badge>
      );
    case "failed":
      return (
        <Badge
          variant="secondary"
          className="bg-destructive/10 text-destructive border-destructive/20"
        >
          Failed
        </Badge>
      );
    default:
      return (
        <Badge variant="secondary" className="bg-amber-500/10 text-amber-600 border-amber-500/20">
          Pending
        </Badge>
      );
  }
}

export default function RelayFeed() {
  const auth = useAuthClient();
  const { data: session } = useQuery(sessionQueryOptions(auth));
  const { data: transactions, isPending } = useRelayHistory(session, auth);

  if (isPending) {
    return (
      <div className="space-y-2">
        {[1, 2].map((i) => (
          <div key={i} className="h-12 bg-muted rounded animate-pulse" />
        ))}
      </div>
    );
  }

  if (!transactions || transactions.length === 0) {
    return (
      <div className="text-center py-6 text-sm text-muted-foreground">
        No relayed transactions yet. Try the Guestbook in gasless mode!
      </div>
    );
  }

  return (
    <div className="space-y-2">
      {transactions.map((tx: RelayedTransactionT) => (
        <div
          key={tx.txHash}
          className="flex items-center gap-3 p-3 border rounded-lg hover:bg-muted/50 transition-colors"
        >
          <StatusIcon status={tx.status} />
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2 min-w-0">
              <a
                href={explorerTxUrl(tx.txHash)}
                target="_blank"
                rel="noopener noreferrer"
                className="text-sm font-mono hover:underline truncate min-w-0 flex-1"
              >
                {truncateHash(tx.txHash)}
              </a>
              <ExternalLink className="h-3 w-3 text-muted-foreground flex-shrink-0" />
            </div>
            <div className="flex items-center gap-2 mt-0.5 min-w-0">
              <span className="text-xs text-muted-foreground truncate min-w-0 flex-1">
                {tx.receiverId}
              </span>
              {tx.gasUsed && (
                <span className="text-xs text-muted-foreground shrink-0">
                  {(Number(tx.gasUsed) / 1e12).toFixed(1)} Tgas
                </span>
              )}
            </div>
          </div>
          <div className="flex flex-col items-end gap-1">
            <StatusBadge status={tx.status} />
            <span className="text-[10px] text-muted-foreground flex items-center gap-1">
              <Clock className="h-2.5 w-2.5" />
              {timeAgo(tx.createdAt)}
            </span>
          </div>
        </div>
      ))}
    </div>
  );
}
