import { ArrowDown, MessageSquare } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useRef, useState } from "react";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";

interface ChatMessageListProps {
  children: ReactNode;
  empty?: boolean;
  emptyMessage?: string;
  streamLoading?: boolean;
}

const NEAR_BOTTOM_THRESHOLD = 120;

export function ChatMessageList({
  children,
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
    bottomRef.current?.scrollIntoView({ behavior, block: "end" });
  }, []);

  const prevEmptyRef = useRef(empty);
  const prevStreamLoadingRef = useRef(streamLoading);
  const userScrolledAwayRef = useRef(false);

  useEffect(() => {
    const wasEmpty = prevEmptyRef.current;
    prevEmptyRef.current = empty;
    prevStreamLoadingRef.current = streamLoading;

    if (wasEmpty && !empty) {
      userScrolledAwayRef.current = false;
      requestAnimationFrame(() => scrollToBottom("instant"));
      return;
    }

    if (userScrolledAwayRef.current) return;

    if (isNearBottom()) {
      requestAnimationFrame(() => scrollToBottom("smooth"));
    }
  }, [children, empty, streamLoading, isNearBottom, scrollToBottom]);

  useEffect(() => {
    const vp = viewportRef.current;
    if (!vp) return;

    const onScroll = () => {
      const nearBottom = isNearBottom();
      setShowScrollButton(!nearBottom);
      if (!nearBottom && prevStreamLoadingRef.current) {
        userScrolledAwayRef.current = true;
      }
      if (nearBottom) {
        userScrolledAwayRef.current = false;
      }
    };

    vp.addEventListener("scroll", onScroll, { passive: true });
    return () => vp.removeEventListener("scroll", onScroll);
  }, [isNearBottom]);

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
