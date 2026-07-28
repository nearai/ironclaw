import type { Automation, ThreadRecord, TimelineMessage } from "@/types";

const threadStore = new Map<string, ThreadRecord[]>();
const timelineStore = new Map<string, TimelineMessage[]>();
const automationStore = new Map<string, Automation[]>();
const draftStore = new Map<string, string>();

export async function cacheThreads(scope: string, threads: ThreadRecord[]): Promise<void> {
  threadStore.set(scope, threads);
}

export async function cachedThreads(scope: string): Promise<ThreadRecord[]> {
  return threadStore.get(scope) ?? [];
}

export async function cacheTimeline(
  scope: string,
  threadId: string,
  messages: TimelineMessage[]
): Promise<void> {
  timelineStore.set(`${scope}|${threadId}`, messages);
}

export async function cachedTimeline(
  scope: string,
  threadId: string
): Promise<TimelineMessage[]> {
  return timelineStore.get(`${scope}|${threadId}`) ?? [];
}

export async function cacheAutomations(scope: string, rows: Automation[]): Promise<void> {
  automationStore.set(scope, rows);
}

export async function cachedAutomations(scope: string): Promise<Automation[]> {
  return automationStore.get(scope) ?? [];
}

export async function saveDraft(scope: string, threadId: string, content: string): Promise<void> {
  draftStore.set(`${scope}|${threadId}`, content);
}

export async function loadDraft(scope: string, threadId: string): Promise<string> {
  return draftStore.get(`${scope}|${threadId}`) ?? "";
}
