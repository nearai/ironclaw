import type { Automation, ThreadRecord, TimelineMessage } from "@/types";

export function cacheThreads(scope: string, threads: ThreadRecord[]): Promise<void>;
export function cachedThreads(scope: string): Promise<ThreadRecord[]>;
export function cacheTimeline(
  scope: string,
  threadId: string,
  messages: TimelineMessage[]
): Promise<void>;
export function cachedTimeline(scope: string, threadId: string): Promise<TimelineMessage[]>;
export function cacheAutomations(scope: string, rows: Automation[]): Promise<void>;
export function cachedAutomations(scope: string): Promise<Automation[]>;
export function saveDraft(scope: string, threadId: string, content: string): Promise<void>;
export function loadDraft(scope: string, threadId: string): Promise<string>;
