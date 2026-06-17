import { eventIterator, oc } from "@orpc/contract";
import { z } from "every-plugin/zod";

const Errors = {
  UNAUTHORIZED: { status: 401, message: "Not authenticated" },
  NOT_FOUND: { status: 404, message: "Resource not found" },
  BAD_REQUEST: { status: 400, message: "Bad request" },
  CONFLICT: { status: 409, message: "Resource conflict" },
  GATEWAY_ERROR: { status: 502, message: "Ironclaw gateway error" },
};

export const ProjectFsEntrySchema = z.object({
  name: z.string(),
  path: z.string(),
  kind: z.enum(["file", "directory", "symlink", "other"]),
});

export const ProjectFsListSchema = z.object({
  entries: z.array(ProjectFsEntrySchema),
});

export const ProjectFsStatSchema = z.object({
  path: z.string(),
  kind: z.enum(["file", "directory", "symlink", "other"]),
  sizeBytes: z.number(),
  mimeType: z.string(),
});

export const DownloadFileResponseSchema = z.object({
  contentBase64: z.string(),
  mimeType: z.string(),
  filename: z.string(),
  sizeBytes: z.number(),
});

export const AttachmentCapabilitiesSchema = z.object({
  accept: z.array(z.string()),
  maxCount: z.number(),
  maxFileBytes: z.number(),
  maxTotalBytes: z.number(),
});

export const AttachmentRefSchema = z.object({
  id: z.string(),
  kind: z.enum(["audio", "image", "document"]),
  mimeType: z.string(),
  filename: z.string().optional(),
  sizeBytes: z.number().optional(),
  storageKey: z.string().optional(),
  extractedText: z.string().optional(),
  previewUrl: z.string().optional(),
});

export const SessionSchema = z.object({
  tenantId: z.string(),
  userId: z.string(),
  capabilities: z.object({
    operatorWebuiConfig: z.boolean(),
    attachments: AttachmentCapabilitiesSchema.optional(),
  }),
});

export const ThreadScopeSchema = z.object({
  tenantId: z.string(),
  agentId: z.string(),
  projectId: z.string().optional(),
  ownerUserId: z.string().optional(),
  missionId: z.string().optional(),
});

export const ThreadGoalSchema = z.object({
  statement: z.string(),
  refinedAtSequence: z.number(),
  refinementCount: z.number(),
});

export const ThreadSchema = z.object({
  threadId: z.string(),
  scope: ThreadScopeSchema,
  createdByActorId: z.string(),
  title: z.string().optional(),
  metadataJson: z.string().optional(),
  goal: ThreadGoalSchema.optional(),
  createdAt: z.string().datetime().optional(),
  updatedAt: z.string().datetime().optional(),
});

export const ThreadListSchema = z.object({
  data: z.array(ThreadSchema),
  meta: z.object({
    total: z.number(),
    hasMore: z.boolean(),
    nextCursor: z.string().nullable(),
  }),
});

export const ThreadCreateSchema = z.object({
  threadId: z.string(),
  title: z.string().optional(),
});

export const TimelineEntrySchema = z.object({
  messageId: z.string(),
  threadId: z.string(),
  sequence: z.number(),
  kind: z.string(),
  status: z.string(),
  actorId: z.string().optional(),
  sourceBindingId: z.string().optional(),
  replyTargetBindingId: z.string().optional(),
  turnId: z.string().optional(),
  turnRunId: z.string().optional(),
  toolResultRef: z.string().optional(),
  content: z.string().optional(),
  redactionRef: z.string().optional(),
  role: z.string().optional(),
  createdAt: z.string().optional(),
  attachments: z.array(AttachmentRefSchema).optional(),
});

export const TimelineSchema = z.object({
  data: z.array(TimelineEntrySchema),
  meta: z.object({
    total: z.number(),
    hasMore: z.boolean(),
    nextCursor: z.string().nullable(),
  }),
});

export const ThreadStateSchema = z.object({
  thread: ThreadSchema,
  messages: z.array(TimelineEntrySchema),
  summaryArtifacts: z.array(
    z.object({
      summaryId: z.string(),
      threadId: z.string(),
      startSequence: z.number(),
      endSequence: z.number(),
      summaryKind: z.string(),
      content: z.string(),
    }),
  ),
});

const AcceptedResponseCommon = {
  threadId: z.string(),
  acceptedMessageRef: z.string(),
};

export const AcceptedResponseSchema = z.discriminatedUnion("outcome", [
  z.object({
    ...AcceptedResponseCommon,
    outcome: z.literal("submitted"),
    runId: z.string(),
    turnId: z.string(),
    status: z.string(),
    resolvedRunProfileId: z.string(),
    resolvedRunProfileVersion: z.number(),
    eventCursor: z.number(),
  }),
  z.object({
    ...AcceptedResponseCommon,
    outcome: z.literal("rejected_busy"),
    activeRunId: z.string().nullable().optional(),
    status: z.string().nullable().optional(),
    eventCursor: z.number().nullable().optional(),
    notice: z.string(),
  }),
  z.object({
    ...AcceptedResponseCommon,
    outcome: z.literal("already_submitted"),
    runId: z.string(),
    status: z.string(),
    eventCursor: z.number(),
  }),
]);

export const GateResolutionSchema = z.enum([
  "approved",
  "denied",
  "credential_provided",
  "cancelled",
]);

export const ChatEventSchema = z.object({
  cursor: z.string().optional(),
  type: z.enum([
    "accepted",
    "running",
    "capability_progress",
    "capability_activity",
    "capability_display_preview",
    "gate",
    "auth_required",
    "final_reply",
    "cancelled",
    "failed",
    "projection_snapshot",
    "projection_update",
    "keep_alive",
  ]),
  ack: z
    .discriminatedUnion("outcome", [
      z.object({
        outcome: z.literal("submitted"),
        threadId: z.string(),
        acceptedMessageRef: z.string(),
        runId: z.string(),
        turnId: z.string(),
        status: z.string(),
        eventCursor: z.number(),
      }),
      z.object({
        outcome: z.literal("rejected_busy"),
        threadId: z.string(),
        acceptedMessageRef: z.string(),
        activeRunId: z.string().nullable().optional(),
        status: z.string().nullable().optional(),
        eventCursor: z.number().nullable().optional(),
        notice: z.string(),
      }),
      z.object({
        outcome: z.literal("already_submitted"),
        threadId: z.string(),
        acceptedMessageRef: z.string(),
        runId: z.string(),
        status: z.string(),
        eventCursor: z.number(),
      }),
    ])
    .optional(),
  progress: z
    .object({
      kind: z.string(),
      turnRunId: z.string().optional(),
      generatedAt: z.string().optional(),
    })
    .optional(),
  activity: z
    .object({
      invocationId: z.string().catch(""),
      turnRunId: z.string().optional(),
      threadId: z.string().optional(),
      capabilityId: z.string().catch(""),
      status: z.string().catch(""),
      provider: z.string().optional(),
      runtime: z.string().optional(),
      processId: z.string().nullable().optional(),
      outputBytes: z.number().nullable().optional(),
      errorKind: z.string().optional(),
      updatedAt: z.string().optional(),
    })
    .optional(),
  preview: z
    .object({
      timelineMessageId: z.string().optional(),
      invocationId: z.string().catch(""),
      turnRunId: z.string().optional(),
      threadId: z.string().optional(),
      capabilityId: z.string().catch(""),
      status: z.string().catch(""),
      title: z.string().catch(""),
      subtitle: z.string().optional(),
      inputSummary: z.string().optional(),
      outputSummary: z.string().optional(),
      outputPreview: z.string().optional(),
      outputKind: z.string().optional(),
      outputBytes: z.number().optional(),
      resultRef: z.string().optional(),
      truncated: z.boolean().catch(false),
      updatedAt: z.string().optional(),
    })
    .optional(),
  reply: z
    .object({
      text: z.string(),
      turnRunId: z.string(),
      generatedAt: z.string(),
    })
    .optional(),
  prompt: z
    .object({
      turnRunId: z.string().catch(""),
      gateRef: z.string().catch(""),
      headline: z.string().catch(""),
      body: z.string().catch(""),
      allowAlways: z.boolean().optional(),
      approvalContext: z
        .object({
          toolName: z.string(),
          action: z.unknown(),
          scope: z.unknown(),
          reason: z.string().optional(),
          destination: z.unknown().optional(),
          details: z.array(z.unknown()).optional(),
        })
        .optional(),
    })
    .optional(),
  authPrompt: z
    .object({
      turnRunId: z.string(),
      authRequestRef: z.string(),
      headline: z.string(),
      body: z.string(),
      challengeKind: z.string().optional(),
      provider: z.string().optional(),
      accountLabel: z.string().optional(),
      authorizationUrl: z.string().optional(),
      expiresAt: z.string().optional(),
    })
    .optional(),
  response: z
    .object({
      runId: z.string().catch(""),
      status: z.string().catch(""),
      eventCursor: z.number().optional(),
      alreadyTerminal: z.boolean().optional(),
    })
    .optional(),
  runState: z
    .object({
      turnId: z.string().catch(""),
      runId: z.string().catch(""),
      status: z.string().catch(""),
      eventCursor: z.number().optional(),
      acceptedMessageRef: z.string().catch(""),
      resolvedRunProfileId: z.string().catch(""),
      resolvedRunProfileVersion: z.number().catch(0),
      receivedAt: z.string().catch(""),
      checkpointId: z.string().optional(),
      gateRef: z.string().optional(),
      failure: z.unknown().optional(),
    })
    .optional(),
  state: z.record(z.string(), z.unknown()).optional(),
});

export const AuthProviderSchema = z.object({
  id: z.string(),
  name: z.string(),
  type: z.string(),
});

export const AutomationSourceSchema = z.object({
  type: z.literal("schedule"),
  cron: z.string(),
  timezone: z.string(),
});

export const AutomationRecentRunSchema = z.object({
  runId: z.string().optional(),
  threadId: z.string().optional(),
  fireSlot: z.string().optional(),
  status: z.string(),
  submittedAt: z.string(),
  completedAt: z.string().optional(),
});

export const AutomationSchema = z.object({
  id: z.string(),
  name: z.string(),
  source: AutomationSourceSchema,
  state: z.string(),
  nextRunAt: z.string().optional(),
  lastRunAt: z.string().optional(),
  lastStatus: z.string().optional(),
  recentRuns: z.array(AutomationRecentRunSchema).optional(),
  isActive: z.boolean(),
  createdAt: z.string().optional(),
});

export const OutboundPreferencesSchema = z.object({
  finalReplyTarget: z
    .object({
      targetId: z.string(),
      channel: z.string(),
      displayName: z.string(),
      description: z.string().optional(),
    })
    .optional(),
  status: z.string().optional(),
  modality: z.string().optional(),
});

export const OutboundTargetSummarySchema = z.object({
  targetId: z.string(),
  channel: z.string(),
  displayName: z.string(),
  description: z.string().optional(),
});

export const OutboundTargetCapabilitiesSchema = z.object({
  finalReplies: z.boolean(),
  gatePrompts: z.boolean(),
  authPrompts: z.boolean(),
});

export const OutboundTargetSchema = z.object({
  target: OutboundTargetSummarySchema,
  capabilities: OutboundTargetCapabilitiesSchema,
});

export const LifecyclePackageRefSchema = z.object({
  kind: z.string(),
  id: z.string(),
});

export const ExtensionSchema = z.object({
  packageRef: LifecyclePackageRefSchema,
  displayName: z.string(),
  kind: z.string(),
  description: z.string(),
  authenticated: z.boolean(),
  active: z.boolean(),
  tools: z.array(z.string()),
  needsSetup: z.boolean(),
  hasAuth: z.boolean(),
  activationStatus: z.string().optional(),
  activationError: z.string().optional(),
  version: z.string().optional(),
  onboardingState: z.string().optional(),
  onboarding: z
    .object({
      credentialInstructions: z.string().optional(),
      setupUrl: z.string().optional(),
      credentialNextStep: z.string().optional(),
    })
    .optional(),
});

export const ExtensionRegistryEntrySchema = z.object({
  packageRef: LifecyclePackageRefSchema,
  displayName: z.string(),
  kind: z.string(),
  description: z.string(),
  installed: z.boolean(),
  keywords: z.array(z.string()),
  version: z.string().optional(),
});

export const ExtensionActionResponseSchema = z.object({
  success: z.boolean(),
  message: z.string(),
  activated: z.boolean().optional(),
  authUrl: z.string().optional(),
  awaitingToken: z.boolean().optional(),
  instructions: z.string().optional(),
  onboardingState: z.string().optional(),
  onboarding: z
    .object({
      credentialInstructions: z.string().optional(),
      setupUrl: z.string().optional(),
      credentialNextStep: z.string().optional(),
    })
    .optional(),
});

export const ExtensionSetupSchema = z.object({
  success: z.boolean(),
  message: z.string().optional(),
});

export const ExtensionSetupDetailSchema = z.object({
  packageRef: LifecyclePackageRefSchema,
  phase: z.string(),
  blockers: z.array(z.unknown()),
  payload: z.unknown().optional(),
  secrets: z.array(
    z.object({
      name: z.string(),
      provider: z.string(),
      prompt: z.string(),
      optional: z.boolean(),
      provided: z.boolean(),
      setup: z.union([
        z.literal("manual_token"),
        z.object({
          kind: z.literal("oauth"),
          accountLabel: z.string(),
          scopes: z.array(z.string()),
          invocationId: z.string(),
        }),
      ]),
      credentialRef: z.string().optional(),
    }),
  ),
  fields: z.array(
    z.object({
      name: z.string(),
      prompt: z.string(),
      optional: z.boolean(),
      placeholder: z.string().optional(),
    }),
  ),
  onboarding: z
    .object({
      credentialInstructions: z.string().optional(),
      setupUrl: z.string().optional(),
      credentialNextStep: z.string().optional(),
    })
    .optional(),
});

export const SkillSchema = z.object({
  name: z.string(),
  description: z.string(),
  version: z.string(),
  trust: z.string(),
  source: z.string(),
  keywords: z.array(z.string()),
  usageHint: z.string().optional(),
  setupHint: z.string().optional(),
  bundlePath: z.string().optional(),
  installSourceUrl: z.string().optional(),
  hasRequirements: z.boolean(),
  hasScripts: z.boolean(),
  canEdit: z.boolean(),
  canDelete: z.boolean(),
});

export const SkillActionResponseSchema = z.object({
  success: z.boolean(),
  message: z.string(),
});

export const SkillContentResponseSchema = z.object({
  name: z.string(),
  content: z.string(),
});

export const SkillSearchResponseSchema = z.object({
  catalog: z.array(z.unknown()),
  installed: z.array(SkillSchema),
  registryUrl: z.string(),
  catalogError: z.string().optional(),
});

export const ConnectableChannelSchema = z.object({
  channel: z.string(),
  displayName: z.string(),
  strategy: z.string(),
  action: z.object({
    title: z.string(),
    instructions: z.string(),
    inputPlaceholder: z.string(),
    submitLabel: z.string(),
    successMessage: z.string(),
    errorMessage: z.string(),
  }),
  commandAliases: z.array(z.string()),
});

export const contract = oc.router({
  ping: oc
    .route({ method: "GET", path: "/ping", summary: "Health check" })
    .output(z.object({ status: z.literal("ok"), timestamp: z.iso.datetime() })),

  session: oc
    .route({ method: "GET", path: "/session", summary: "Get current ironclaw session" })
    .output(SessionSchema)
    .errors(Errors),

  threads: {
    list: oc
      .route({ method: "GET", path: "/threads", summary: "List threads" })
      .input(
        z.object({
          limit: z.number().optional(),
          cursor: z.string().optional(),
        }),
      )
      .output(ThreadListSchema)
      .errors(Errors),

    create: oc
      .route({ method: "POST", path: "/threads", summary: "Create a new thread" })
      .input(
        z.object({
          clientActionId: z.string().optional(),
        }),
      )
      .output(ThreadCreateSchema)
      .errors(Errors),

    delete: oc
      .route({ method: "DELETE", path: "/threads/{id}", summary: "Delete a thread" })
      .input(z.object({ id: z.string() }))
      .output(z.object({ success: z.boolean() }))
      .errors(Errors),

    sendMessage: oc
      .route({ method: "POST", path: "/threads/{id}/messages", summary: "Send a message" })
      .input(
        z.object({
          id: z.string(),
          content: z.string(),
          clientActionId: z.string().optional(),
          attachments: z
            .array(
              z.object({
                mimeType: z.string(),
                filename: z.string().optional(),
                dataBase64: z.string(),
              }),
            )
            .optional(),
        }),
      )
      .output(AcceptedResponseSchema)
      .errors(Errors),

    getTimeline: oc
      .route({ method: "GET", path: "/threads/{id}/timeline", summary: "Get thread timeline" })
      .input(
        z.object({
          id: z.string(),
          limit: z.number().optional(),
          cursor: z.string().optional(),
        }),
      )
      .output(TimelineSchema)
      .errors(Errors),

    cancelRun: oc
      .route({ method: "POST", path: "/threads/{id}/runs/{runId}/cancel", summary: "Cancel a run" })
      .input(
        z.object({
          id: z.string(),
          runId: z.string(),
        }),
      )
      .output(
        z.object({
          success: z.boolean(),
          runId: z.string().optional(),
          status: z.string().optional(),
          eventCursor: z.number().optional(),
          alreadyTerminal: z.boolean().optional(),
        }),
      )
      .errors(Errors),

    resolveGate: oc
      .route({
        method: "POST",
        path: "/threads/{id}/runs/{runId}/gates/{gateRef}/resolve",
        summary: "Resolve an approval gate",
      })
      .input(
        z.object({
          id: z.string(),
          runId: z.string(),
          gateRef: z.string().catch(""),
          resolution: GateResolutionSchema,
          always: z.boolean().optional(),
          credentialRef: z.string().optional(),
        }),
      )
      .output(z.object({ success: z.boolean() }))
      .errors(Errors),

    streamEvents: oc
      .route({
        method: "GET",
        path: "/threads/{id}/events",
        summary: "Stream chat events via SSE",
      })
      .input(
        z.object({
          id: z.string(),
          afterCursor: z.string().optional(),
        }),
      )
      .output(eventIterator(ChatEventSchema))
      .errors(Errors),

    getState: oc
      .route({
        method: "GET",
        path: "/threads/{id}/state",
        summary: "Get authoritative thread state for UI rebuild",
      })
      .input(z.object({ id: z.string() }))
      .output(ThreadStateSchema)
      .errors(Errors),

    listFiles: oc
      .route({
        method: "GET",
        path: "/threads/{id}/files",
        summary: "List files in the thread's project workspace",
      })
      .input(
        z.object({
          id: z.string(),
          path: z.string().optional(),
        }),
      )
      .output(ProjectFsListSchema)
      .errors(Errors),

    statFile: oc
      .route({
        method: "GET",
        path: "/threads/{id}/files/stat",
        summary: "Get metadata for a project file",
      })
      .input(
        z.object({
          id: z.string(),
          path: z.string(),
        }),
      )
      .output(ProjectFsStatSchema)
      .errors(Errors),

    downloadFile: oc
      .route({
        method: "GET",
        path: "/threads/{id}/files/content",
        summary: "Download a file from the thread's project workspace",
      })
      .input(
        z.object({
          id: z.string(),
          path: z.string(),
        }),
      )
      .output(DownloadFileResponseSchema)
      .errors(Errors),

    getAttachment: oc
      .route({
        method: "GET",
        path: "/threads/{id}/messages/{messageId}/attachments/{attachmentId}",
        summary: "Get attachment bytes from a message",
      })
      .input(
        z.object({
          id: z.string(),
          messageId: z.string(),
          attachmentId: z.string(),
        }),
      )
      .output(DownloadFileResponseSchema)
      .errors(Errors),
  },

  automations: {
    list: oc
      .route({ method: "GET", path: "/automations", summary: "List automations" })
      .input(
        z.object({
          limit: z.number().optional(),
          runLimit: z.number().optional(),
        }),
      )
      .output(z.object({ data: z.array(AutomationSchema) }))
      .errors(Errors),
  },

  outbound: {
    getPreferences: oc
      .route({ method: "GET", path: "/outbound/preferences", summary: "Get outbound preferences" })
      .output(OutboundPreferencesSchema)
      .errors(Errors),

    setPreferences: oc
      .route({ method: "POST", path: "/outbound/preferences", summary: "Set outbound preferences" })
      .input(OutboundPreferencesSchema)
      .output(z.object({ success: z.boolean() }))
      .errors(Errors),

    listTargets: oc
      .route({ method: "GET", path: "/outbound/targets", summary: "List delivery targets" })
      .output(
        z.object({
          data: z.array(OutboundTargetSchema),
          nextCursor: z.string().nullable().optional(),
        }),
      )
      .errors(Errors),
  },

  extensions: {
    list: oc
      .route({ method: "GET", path: "/extensions", summary: "List installed extensions" })
      .output(z.object({ data: z.array(ExtensionSchema) }))
      .errors(Errors),

    listRegistry: oc
      .route({ method: "GET", path: "/extensions/registry", summary: "List extension registry" })
      .output(z.object({ data: z.array(ExtensionRegistryEntrySchema) }))
      .errors(Errors),

    install: oc
      .route({ method: "POST", path: "/extensions/install", summary: "Install an extension" })
      .input(z.object({ packageRef: LifecyclePackageRefSchema }))
      .output(ExtensionActionResponseSchema)
      .errors(Errors),

    activate: oc
      .route({
        method: "POST",
        path: "/extensions/{name}/activate",
        summary: "Activate an extension",
      })
      .input(z.object({ name: z.string() }))
      .output(ExtensionActionResponseSchema)
      .errors(Errors),

    remove: oc
      .route({ method: "POST", path: "/extensions/{name}/remove", summary: "Remove an extension" })
      .input(z.object({ name: z.string() }))
      .output(ExtensionActionResponseSchema)
      .errors(Errors),

    getSetup: oc
      .route({
        method: "GET",
        path: "/extensions/{name}/setup",
        summary: "Get extension setup state",
      })
      .input(z.object({ name: z.string() }))
      .output(ExtensionSetupDetailSchema)
      .errors(Errors),

    setup: oc
      .route({
        method: "POST",
        path: "/extensions/{name}/setup",
        summary: "Setup/configure an extension",
      })
      .input(
        z.object({
          name: z.string(),
          action: z.string(),
          payload: z.record(z.string(), z.unknown()).optional(),
        }),
      )
      .output(ExtensionSetupSchema)
      .errors(Errors),
  },

  skills: {
    list: oc
      .route({ method: "GET", path: "/skills", summary: "List installed skills" })
      .output(z.object({ data: z.array(SkillSchema), count: z.number() }))
      .errors(Errors),

    search: oc
      .route({ method: "POST", path: "/skills/search", summary: "Search skill catalog" })
      .input(z.object({ query: z.string() }))
      .output(SkillSearchResponseSchema)
      .errors(Errors),

    install: oc
      .route({ method: "POST", path: "/skills/install", summary: "Install a skill" })
      .input(z.object({ name: z.string(), content: z.string().optional() }))
      .output(SkillActionResponseSchema)
      .errors(Errors),

    get: oc
      .route({ method: "GET", path: "/skills/{name}", summary: "Get skill content" })
      .input(z.object({ name: z.string() }))
      .output(SkillContentResponseSchema)
      .errors(Errors),

    update: oc
      .route({ method: "PUT", path: "/skills/{name}", summary: "Update a skill" })
      .input(z.object({ name: z.string(), content: z.string() }))
      .output(SkillActionResponseSchema)
      .errors(Errors),

    remove: oc
      .route({ method: "DELETE", path: "/skills/{name}", summary: "Remove a skill" })
      .input(z.object({ name: z.string() }))
      .output(SkillActionResponseSchema)
      .errors(Errors),
  },

  channels: {
    listConnectable: oc
      .route({
        method: "GET",
        path: "/channels/connectable",
        summary: "List connectable channels",
      })
      .output(z.object({ data: z.array(ConnectableChannelSchema) }))
      .errors(Errors),
  },

  operator: {
    createAccessSession: oc
      .route({
        method: "POST",
        path: "/operator/access-sessions",
        summary: "Mint a tenant-scoped access session token",
      })
      .input(
        z.object({
          tenantId: z.string(),
          agentId: z.string().optional(),
          projectId: z.string().optional(),
        }),
      )
      .output(
        z.object({
          token: z.string(),
          expiresAt: z.iso.datetime(),
        }),
      )
      .errors(Errors),
  },

  auth: {
    listProviders: oc
      .route({ method: "GET", path: "/auth/providers", summary: "List OAuth providers" })
      .output(z.object({ data: z.array(AuthProviderSchema) }))
      .errors(Errors),

    exchangeLoginTicket: oc
      .route({
        method: "POST",
        path: "/auth/session/exchange",
        summary: "Exchange login ticket for bearer token",
      })
      .input(z.object({ loginTicket: z.string() }))
      .output(z.object({ token: z.string() }))
      .errors(Errors),

    logout: oc
      .route({ method: "POST", path: "/auth/logout", summary: "Logout / revoke session" })
      .output(z.object({ success: z.boolean() }))
      .errors(Errors),

    submitManualToken: oc
      .route({
        method: "POST",
        path: "/product-auth/manual-token/submit",
        summary: "Submit a manual token for credential storage",
      })
      .input(
        z.object({
          provider: z.string(),
          accountLabel: z.string(),
          token: z.string(),
          threadId: z.string(),
          runId: z.string(),
          gateRef: z.string(),
        }),
      )
      .output(z.object({ credentialRef: z.string() }))
      .errors(Errors),
  },
});

export type ContractType = typeof contract;

export type ChatEvent = z.infer<typeof ChatEventSchema>;
export type AcceptedResponse = z.infer<typeof AcceptedResponseSchema>;
export type TimelineEntry = z.infer<typeof TimelineEntrySchema>;
export type Thread = z.infer<typeof ThreadSchema>;
export type Session = z.infer<typeof SessionSchema>;
export type GateResolution = z.infer<typeof GateResolutionSchema>;
export type ThreadCreate = z.infer<typeof ThreadCreateSchema>;
