import { Effect } from "every-plugin/effect";
import { ORPCError } from "every-plugin/orpc";
import {
  type ConversationEvent,
  type ConversationMessagePage,
  diffMessageSets,
  normalizeTimelinePage,
} from "./conversation";

function isRateLimitError(error: unknown): boolean {
  if (error instanceof ORPCError) {
    const e = error as { status?: unknown; code?: unknown };
    if (e.status === 429 || e.code === 429) return true;
  }
  const candidate = error as { status?: unknown; code?: unknown };
  if (candidate.status === 429 || candidate.code === 429) return true;
  const message = error instanceof Error ? error.message : String(error);
  return /429|too many requests|rate limit/i.test(message);
}

async function fetchTimelinePage(
  ic: any,
  threadId: string,
): Promise<ConversationMessagePage> {
  const delays = [100, 200, 400, 800, 1600];

  for (let attempt = 0; ; attempt++) {
    try {
      const raw = await ic.threads.getTimeline({ id: threadId, limit: 100 });
      return normalizeTimelinePage(raw, threadId);
    } catch (error) {
      if (!isRateLimitError(error) || attempt >= delays.length) {
        throw error;
      }
      await Effect.runPromise(Effect.sleep(`${delays[attempt]!} millis`));
    }
  }
}

export function buildReconcileEvents(
  page: ConversationMessagePage,
  threadId: string,
  knownIds: Set<string>,
  snapshotYielded: boolean,
): ConversationEvent[] {
  if (!snapshotYielded) {
    for (const msg of page.messages) {
      knownIds.add(msg.id);
    }
    return [
      { type: "snapshot", threadId, messages: page.messages } satisfies ConversationEvent,
    ];
  }

  const added = diffMessageSets(knownIds, page.messages);
  if (added.length === 0) return [];

  const events: ConversationEvent[] = [];
  for (const msg of added) {
    knownIds.add(msg.id);
  }
  events.push({
    type: "messages_changed",
    threadId,
    messages: page.messages,
  } satisfies ConversationEvent);
  for (const msg of added) {
    events.push({ type: "message_added", threadId, message: msg } satisfies ConversationEvent);
    if (msg.role === "assistant") {
      events.push({
        type: "run_finished",
        threadId,
        runId: msg.runId ?? undefined,
      } satisfies ConversationEvent);
    }
  }
  return events;
}

const RUN_START_TYPES = new Set(["accepted", "running"]);
const RUN_TERMINAL_TYPES = new Set(["final_reply", "failed", "cancelled"]);
const PROJECTION_TYPES = new Set(["projection_snapshot", "projection_update"]);

export function createConversationStreamHandler(services: { ironclaw: (ctx: any) => any }) {
  return async function* ({ input, signal, context }: any) {
    const ic = services.ironclaw(context);
    const threadId = input.threadId;
    const afterCursor = input.afterCursor;
    const knownIds = new Set<string>();
    let snapshotYielded = false;
    let runActive = false;
    let needsReconcile = false;
    let lastIdleReconcile = 0;
    const IDLE_RECONCILE_THROTTLE_MS = 1000;

    try {
      const rawStream = await ic.threads.streamEvents({
        id: threadId,
        afterCursor,
      });

      for await (const rawEvent of rawStream as AsyncIterable<Record<string, unknown>>) {
        if (signal?.aborted) break;

        const type = rawEvent.type as string;

        if (type === "keep_alive") {
          yield { type: "keep_alive", threadId } satisfies ConversationEvent;
          continue;
        }

        if (RUN_START_TYPES.has(type)) {
          runActive = true;
          continue;
        }

        if (RUN_TERMINAL_TYPES.has(type)) {
          runActive = false;
          if (needsReconcile && !signal?.aborted) {
            try {
              const page = await fetchTimelinePage(ic, threadId);
              const reconcileEvents = buildReconcileEvents(page, threadId, knownIds, snapshotYielded);
              for (const event of reconcileEvents) {
                if (event.type === "snapshot") snapshotYielded = true;
                yield event;
              }
              needsReconcile = false;
            } catch (e) {
              console.error("[convStream] terminal reconcile failed:", e);
            }
          }
          continue;
        }

        if (PROJECTION_TYPES.has(type)) {
          if (runActive) {
            needsReconcile = true;
            continue;
          }

          if (Date.now() - lastIdleReconcile < IDLE_RECONCILE_THROTTLE_MS) {
            continue;
          }

          try {
            const page = await fetchTimelinePage(ic, threadId);
            const reconcileEvents = buildReconcileEvents(page, threadId, knownIds, snapshotYielded);
            for (const event of reconcileEvents) {
              if (event.type === "snapshot") snapshotYielded = true;
              yield event;
            }
            lastIdleReconcile = Date.now();
            needsReconcile = false;
          } catch (e) {
            console.error("[convStream] projection sync failed:", e);
          }
          continue;
        }

        continue;
      }
    } catch (error) {
      console.error("[convStream] stream connection error:", error);
      yield {
        type: "error",
        threadId,
        error: error instanceof Error ? error.message : String(error),
      } satisfies ConversationEvent;
    }
  };
}
