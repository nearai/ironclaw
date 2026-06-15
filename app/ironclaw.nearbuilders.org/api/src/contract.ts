import { BAD_REQUEST, UNAUTHORIZED, NOT_FOUND } from "every-plugin/errors";
import { eventIterator, oc } from "every-plugin/orpc";
import { z } from "every-plugin/zod";
import { contract as ironclawContract } from "../../plugins/ironclaw/src/contract";

export const RegisterInputSchema = z.object({
  agentId: z.string().min(1).max(64),
  participantName: z.string().min(1).max(128),
  novaAccountId: z.string().min(1).max(128),
});

export const SubmitInputSchema = z.object({
  agentId: z.string().min(1).max(64),
  novaAccountId: z.string().min(1).max(128),
  novaApiKey: z.string().min(1),
  projectTitle: z.string().min(1).max(256),
  description: z.string().min(1).max(280),
  demoUrl: z.string().url(),
  githubUrl: z.string().url().optional(),
  skillsList: z.string().optional(),
  demoNotes: z.string().optional(),
});

export const LeaderboardEntrySchema = z.object({
  agentId: z.string(),
  participantName: z.string(),
  projectTitle: z.string(),
  submittedAt: z.iso.datetime(),
});

export const IronclawSettingsSchema = z.object({
  tunnelUrl: z.string().url(),
  apiToken: z.string(),
  hasToken: z.boolean().optional(),
  updatedAt: z.iso.datetime().optional(),
});

const ConversationMessageSchema = z.object({
  id: z.string(),
  threadId: z.string(),
  role: z.enum(["user", "assistant"]),
  text: z.string(),
  createdAt: z.string().nullable(),
  status: z.enum(["submitted", "finalized", "failed"]),
  sequence: z.number(),
  runId: z.string().nullable(),
});

const ConversationThreadSchema = z.object({
  threadId: z.string(),
  title: z.string().nullable(),
  tenantId: z.string(),
  agentId: z.string(),
  projectId: z.string().nullable(),
  createdByActorId: z.string(),
});

const ConversationMessagePageSchema = z.object({
  messages: z.array(ConversationMessageSchema),
  nextCursor: z.string().nullable(),
  hasMore: z.boolean(),
  total: z.number(),
});

const ConversationSendAckSchema = z.object({
  threadId: z.string(),
  runId: z.string(),
  acceptedMessageRef: z.string(),
  pendingMessageId: z.string(),
  submittedAt: z.string(),
  eventCursor: z.string().optional(),
});

const ConversationEventSchema = z.object({
  type: z.enum([
    "snapshot",
    "messages_changed",
    "message_added",
    "run_pending",
    "run_finished",
    "error",
    "keep_alive",
  ]),
  threadId: z.string(),
  messages: z.array(ConversationMessageSchema).optional(),
  message: ConversationMessageSchema.optional(),
  runId: z.string().optional(),
  error: z.string().optional(),
});

export const contract = oc.router({
  ping: oc.route({ method: "GET", path: "/ping" }).output(
    z.object({
      status: z.literal("ok"),
      timestamp: z.iso.datetime(),
    }),
  ),

  hackathon: {
    register: oc
      .route({ method: "POST", path: "/hackathon/register" })
      .input(RegisterInputSchema)
      .output(
        z.object({
          success: z.boolean(),
          message: z.string(),
        }),
      )
      .errors({ UNAUTHORIZED, BAD_REQUEST }),

    submit: oc
      .route({ method: "POST", path: "/hackathon/submit" })
      .input(SubmitInputSchema)
      .output(
        z.object({
          success: z.boolean(),
          cid: z.string(),
          message: z.string(),
        }),
      )
      .errors({ UNAUTHORIZED, BAD_REQUEST }),

    leaderboard: oc.route({ method: "GET", path: "/hackathon/leaderboard" }).output(
      z.object({
        entries: z.array(LeaderboardEntrySchema),
      }),
    ),
  },

  ironclaw: {
    ...oc.prefix("/ironclaw").router(ironclawContract),

    settings: {
      get: oc
        .route({ method: "GET", path: "/ironclaw/settings", summary: "Get ironclaw connection settings" })
        .output(IronclawSettingsSchema)
        .errors({ UNAUTHORIZED, NOT_FOUND }),

      update: oc
        .route({ method: "PUT", path: "/ironclaw/settings", summary: "Update ironclaw connection settings" })
        .input(z.object({
          tunnelUrl: z.string().url(),
          apiToken: z.string().min(1),
        }))
        .output(z.object({ success: z.boolean() }))
        .errors({ UNAUTHORIZED, BAD_REQUEST }),
    },
  },

  conversation: {
    listThreads: oc
      .route({ method: "GET", path: "/conversation/threads", summary: "List conversation threads" })
      .output(z.object({ data: z.array(ConversationThreadSchema) }))
      .errors({ UNAUTHORIZED }),

    getMessages: oc
      .route({
        method: "GET",
        path: "/conversation/threads/{threadId}/messages",
        summary: "Get paginated conversation messages",
      })
      .input(
        z.object({
          threadId: z.string(),
          cursor: z.string().optional(),
          limit: z.number().optional(),
        }),
      )
      .output(ConversationMessagePageSchema)
      .errors({ UNAUTHORIZED, NOT_FOUND }),

    sendMessage: oc
      .route({
        method: "POST",
        path: "/conversation/threads/{threadId}/messages",
        summary: "Send a message and get a normalized ack",
      })
      .input(
        z.object({
          threadId: z.string(),
          content: z.string(),
          clientActionId: z.string().optional(),
        }),
      )
      .output(ConversationSendAckSchema)
      .errors({ UNAUTHORIZED, NOT_FOUND }),

    stream: oc
      .route({
        method: "GET",
        path: "/conversation/threads/{threadId}/events",
        summary: "Subscribe to conversation changes",
      })
      .input(
        z.object({
          threadId: z.string(),
          afterCursor: z.string().optional(),
        }),
      )
      .output(eventIterator(ConversationEventSchema))
      .errors({ UNAUTHORIZED, NOT_FOUND }),
  },
});

export type ContractType = typeof contract;
