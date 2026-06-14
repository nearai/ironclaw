import { FileText, MessageSquare, User } from "lucide-react";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetDescription,
} from "@/components/ui/sheet";
import { Separator } from "@/components/ui/separator";
import { Badge } from "@/components/ui/badge";
import type { ThreadState } from "@/hooks/use-thread-state";

interface ChatThreadMetaProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  threadState: ThreadState | null;
}

export function ChatThreadMeta({ open, onOpenChange, threadState }: ChatThreadMetaProps) {
  if (!threadState) return null;

  const { thread, messages } = threadState;
  const summaryArtifacts = threadState.summaryArtifacts ?? [];

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="w-full max-w-sm">
        <SheetHeader className="pb-2">
          <SheetTitle>Thread Info</SheetTitle>
          <SheetDescription>
            {thread.title ?? `Thread ${thread.threadId.slice(0, 8)}`}
          </SheetDescription>
        </SheetHeader>

        <div className="flex-1 overflow-y-auto px-5 pb-4">
          <div className="space-y-4">
            <div>
              <h4 className="mb-1.5 text-xs font-medium text-muted-foreground">Scope</h4>
              <div className="space-y-1 rounded-lg bg-muted/50 px-3 py-2">
                <div className="flex items-center justify-between text-xs">
                  <span className="text-muted-foreground">Tenant</span>
                  <span className="font-medium">{thread.scope?.tenantId || "-"}</span>
                </div>
                <div className="flex items-center justify-between text-xs">
                  <span className="text-muted-foreground">Agent</span>
                  <span className="font-medium">{thread.scope?.agentId || "-"}</span>
                </div>
                {thread.scope?.projectId && (
                  <div className="flex items-center justify-between text-xs">
                    <span className="text-muted-foreground">Project</span>
                    <span className="font-medium">{thread.scope?.projectId}</span>
                  </div>
                )}
              </div>
            </div>

            <Separator />

            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <User size={12} />
              <span>Created by: {thread.createdByActorId || "-"}</span>
            </div>

            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <MessageSquare size={12} />
              <span>Messages: {messages.length}</span>
            </div>

            {summaryArtifacts.length > 0 && (
              <>
                <Separator />
                <div>
                  <h4 className="mb-2 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
                    <FileText size={12} />
                    Summary Artifacts
                  </h4>
                  <div className="space-y-2">
                    {summaryArtifacts.map((artifact, idx) => (
                      <div
                        key={String(artifact.summaryId ?? idx)}
                        className="rounded-lg border border-border bg-card px-3 py-2"
                      >
                        <div className="flex items-center justify-between">
                          <Badge variant="secondary" className="text-[10px]">
                            {String(artifact.summaryKind ?? "")}
                          </Badge>
                          <span className="text-[10px] text-muted-foreground">
                            seq {String(artifact.startSequence ?? "")}-{String(artifact.endSequence ?? "")}
                          </span>
                        </div>
                        <p className="mt-1.5 text-xs text-foreground line-clamp-2">
                          {String(artifact.content ?? "")}
                        </p>
                      </div>
                    ))}
                  </div>
                </div>
              </>
            )}
          </div>
        </div>
      </SheetContent>
    </Sheet>
  );
}
