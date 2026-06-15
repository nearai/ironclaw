import { ArrowDown, MessageSquare } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useRef, useState } from "react";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";

interface ChatMessageListProps {
  children: ReactNode;
  loading?: boolean;
  empty?: boolean;
  emptyMessage?: string;
  streamLoading?: boolean;
}

const NEAR_BOTTOM_THRESHOLD = 200;

export function ChatMessageList({
  children,
  loading,
  empty,
  emptyMessage = "No messages yet",
  streamLoading,
}: ChatMessageListProps) {
  const bottomRef = useRef<HTMLDivElement>(null);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const [showScrollButton, setShowScrollButton] = useState(false);

  const wrapperRef = useCallback((node: HTMLDivElement | null) => {
    if (!node) return;
    const viewport = node.querySelector<HTMLDivElement>("[data-slot='scroll-area-viewport']");
    if (viewport) viewportRef.current = viewport;
  }, []);

  const isNearBottom = useCallback(() => {
    const vp = viewportRef.current;
    if (!vp) return true;
    return vp.scrollHeight - vp.scrollTop - vp.clientHeight < NEAR_BOTTOM_THRESHOLD;
  }, []);

  const scrollToBottom = useCallback((behavior: ScrollBehavior = "smooth") => {
    bottomRef.current?.scrollIntoView({ behavior });
  }, []);

  const prevLoadingRef = useRef(loading);
  const prevEmptyRef = useRef(empty);
  const prevStreamLoadingRef = useRef(streamLoading);
  const childCount = useRef(0);

  useEffect(() => {
    const wasLoading = prevLoadingRef.current;
    const wasEmpty = prevEmptyRef.current;
    prevLoadingRef.current = loading;
    prevEmptyRef.current = empty;

    if (wasEmpty && !empty) {
      requestAnimationFrame(() => scrollToBottom("instant"));
      return;
    }

    if (wasLoading && !loading) {
      scrollToBottom("instant");
      return;
    }

    const wasStreamLoading = prevStreamLoadingRef.current;
    prevStreamLoadingRef.current = streamLoading;
    if (wasStreamLoading && !streamLoading) {
      requestAnimationFrame(() => scrollToBottom("smooth"));
    }

    if (!isNearBottom()) return;

    const prevChildCount = childCount.current;
    const currentChildCount = Array.isArray(children) ? children.length : children ? 1 : 0;
    childCount.current = currentChildCount;

    if (currentChildCount > prevChildCount) {
      requestAnimationFrame(() => scrollToBottom("smooth"));
    } else if (!loading && !empty) {
      scrollToBottom("smooth");
    }
  }, [children, loading, empty, streamLoading, isNearBottom, scrollToBottom]);

  useEffect(() => {
    const vp = viewportRef.current;
    if (!vp) return;

    const onScroll = () => {
      setShowScrollButton(!isNearBottom());
    };

    vp.addEventListener("scroll", onScroll, { passive: true });
    return () => vp.removeEventListener("scroll", onScroll);
  }, [isNearBottom]);

  if (loading) {
    return (
      <div className="flex-1 p-4">
        <div className="mx-auto max-w-4xl space-y-4">
          {Array.from({ length: 4 }).map((_, i) => (
            <div key={i} className={`flex ${i % 2 === 0 ? "justify-end" : "justify-start"}`}>
              <div className={`space-y-2 ${i % 2 === 0 ? "max-w-[60%]" : "max-w-[75%]"}`}>
                <Skeleton className="h-4 w-32" />
                <Skeleton className="h-16 w-full rounded-xl" />
              </div>
            </div>
          ))}
        </div>
      </div>
    );
  }

  if (empty) {
    return (
      <div className="flex flex-1 items-center justify-center p-4">
        <div className="flex flex-col items-center gap-2 text-center">
          <MessageSquare size={24} className="text-muted-foreground" />
          <p className="text-sm text-muted-foreground">{emptyMessage}</p>
        </div>
      </div>
    );
  }

  return (
    <div ref={wrapperRef} className="relative min-h-0 flex-1 overflow-hidden">
      <ScrollArea className="h-full">
        <div className="mx-auto max-w-4xl space-y-4 p-2 sm:p-4">
          {children}
          <div ref={bottomRef} />
        </div>
      </ScrollArea>
      <button
        type="button"
        onClick={() => scrollToBottom("smooth")}
        className={cn(
          "absolute bottom-4 left-1/2 -translate-x-1/2 flex items-center gap-1.5 rounded-full border border-border bg-background px-3 py-1.5 text-xs font-medium text-muted-foreground shadow-md transition-all duration-200 hover:bg-muted hover:text-foreground",
          showScrollButton
            ? "opacity-100 translate-y-0 pointer-events-auto"
            : "opacity-0 translate-y-2 pointer-events-none",
        )}
      >
        <ArrowDown size={12} />
        Scroll to bottom
      </button>
    </div>
  );
}
