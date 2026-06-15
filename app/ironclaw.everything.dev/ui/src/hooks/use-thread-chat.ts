import { useSyncExternalStore, useCallback, useEffect, useRef } from "react";
import type { UIMessage } from "@tanstack/ai";
import { useApiClient } from "@/app";
import { threadChatManager } from "./use-thread-chat-manager";

interface UseThreadChatOptions {
  threadId: string;
  initialMessages: UIMessage[];
}

export function useThreadChat({ threadId, initialMessages }: UseThreadChatOptions) {
  const apiClient = useApiClient();

  const versionRef = useRef(-1);
  const snapshotRef = useRef<any>(null);

  const getSnapshot = useCallback(() => {
    const session = threadChatManager.get(threadId);
    const version = session?.version ?? 0;
    if (versionRef.current === version && snapshotRef.current) {
      return snapshotRef.current;
    }
    versionRef.current = version;
    snapshotRef.current = {
      messages: session?.messages ?? ([] as UIMessage[]),
      isLoading: session?.isLoading ?? false,
      error: session?.error ?? null,
      runId: session?.runId ?? null,
      pendingApprovals: session?.pendingApprovals ?? [],
    };
    return snapshotRef.current;
  }, [threadId]);

  const subscribe = useCallback(
    (cb: () => void) => threadChatManager.subscribe(threadId, cb),
    [threadId],
  );

  const state = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

  useEffect(() => {
    if (initialMessages.length > 0) {
      threadChatManager.hydrate(threadId, initialMessages);
    }
  }, [threadId, initialMessages]);

  useEffect(() => {
    return () => { threadChatManager.destroy(threadId); };
  }, [threadId]);

  const resolveGate = useCallback(
    async (runId: string, gateRef: string, approved: boolean) => {
      await apiClient.conversation.threadApprove({
        threadId,
        runId,
        gateRef,
        approved,
      });
    },
    [apiClient, threadId],
  );

  const sendMessage = useCallback(
    (content: string) => {
      threadChatManager.sendMessage(threadId, content);
    },
    [threadId],
  );

  const stop = useCallback(() => {
    threadChatManager.stop(threadId);
  }, [threadId]);

  const copyConversation = useCallback(async () => {
    const session = threadChatManager.get(threadId);
    if (!session) return;
    const text = session.messages.map((msg) => {
      const role = msg.role === "user" ? "User" : "Assistant";
      const textParts: string[] = [];
      const toolParts: string[] = [];
      for (const p of msg.parts) {
        if (p.type === "text") {
          textParts.push(p.content);
        } else if (p.type === "tool-call") {
          toolParts.push(`  Tool: ${p.name}(${p.arguments})`);
        } else if (p.type === "tool-result") {
          const summary = typeof p.content === "string" ? p.content.slice(0, 200) : String(p.content).slice(0, 200);
          toolParts.push(`  ${p.state === "error" ? "Error" : "Result"}: ${summary}`);
        }
      }
      return `${role}:\n${[...textParts, ...toolParts].join("\n")}`;
    }).join("\n\n---\n\n");
    await navigator.clipboard.writeText(text);
  }, [threadId]);

  return {
    messages: state.messages,
    isLoading: state.isLoading,
    error: state.error,
    runId: state.runId,
    pendingApprovals: state.pendingApprovals,
    sendMessage,
    stop,
    resolveGate,
    copyConversation,
    threadId,
  };
}
