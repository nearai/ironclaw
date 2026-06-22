import { z } from "every-plugin/zod";
export declare const ProjectFsEntrySchema: z.ZodObject<{
    name: z.ZodString;
    path: z.ZodString;
    kind: z.ZodEnum<{
        file: "file";
        directory: "directory";
        symlink: "symlink";
        other: "other";
    }>;
}, z.core.$strip>;
export declare const ProjectFsListSchema: z.ZodObject<{
    entries: z.ZodArray<z.ZodObject<{
        name: z.ZodString;
        path: z.ZodString;
        kind: z.ZodEnum<{
            file: "file";
            directory: "directory";
            symlink: "symlink";
            other: "other";
        }>;
    }, z.core.$strip>>;
}, z.core.$strip>;
export declare const ProjectFsStatSchema: z.ZodObject<{
    path: z.ZodString;
    kind: z.ZodEnum<{
        file: "file";
        directory: "directory";
        symlink: "symlink";
        other: "other";
    }>;
    sizeBytes: z.ZodNumber;
    mimeType: z.ZodString;
}, z.core.$strip>;
export declare const DownloadFileResponseSchema: z.ZodObject<{
    contentBase64: z.ZodString;
    mimeType: z.ZodString;
    filename: z.ZodString;
    sizeBytes: z.ZodNumber;
}, z.core.$strip>;
export declare const AttachmentCapabilitiesSchema: z.ZodObject<{
    accept: z.ZodArray<z.ZodString>;
    maxCount: z.ZodNumber;
    maxFileBytes: z.ZodNumber;
    maxTotalBytes: z.ZodNumber;
}, z.core.$strip>;
export declare const AttachmentRefSchema: z.ZodObject<{
    id: z.ZodString;
    kind: z.ZodEnum<{
        audio: "audio";
        image: "image";
        document: "document";
    }>;
    mimeType: z.ZodString;
    filename: z.ZodOptional<z.ZodString>;
    sizeBytes: z.ZodOptional<z.ZodNumber>;
    storageKey: z.ZodOptional<z.ZodString>;
    extractedText: z.ZodOptional<z.ZodString>;
    previewUrl: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
export declare const SessionFeaturesSchema: z.ZodObject<{
    rebornProjects: z.ZodBoolean;
}, z.core.$strip>;
export declare const SessionSchema: z.ZodObject<{
    tenantId: z.ZodString;
    userId: z.ZodString;
    capabilities: z.ZodObject<{
        operatorWebuiConfig: z.ZodBoolean;
        attachments: z.ZodOptional<z.ZodObject<{
            accept: z.ZodArray<z.ZodString>;
            maxCount: z.ZodNumber;
            maxFileBytes: z.ZodNumber;
            maxTotalBytes: z.ZodNumber;
        }, z.core.$strip>>;
    }, z.core.$strip>;
    features: z.ZodOptional<z.ZodObject<{
        rebornProjects: z.ZodBoolean;
    }, z.core.$strip>>;
}, z.core.$strip>;
export declare const ThreadScopeSchema: z.ZodObject<{
    tenantId: z.ZodString;
    agentId: z.ZodString;
    projectId: z.ZodOptional<z.ZodString>;
    ownerUserId: z.ZodOptional<z.ZodString>;
    missionId: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
export declare const ThreadGoalSchema: z.ZodObject<{
    statement: z.ZodString;
    refinedAtSequence: z.ZodNumber;
    refinementCount: z.ZodNumber;
}, z.core.$strip>;
export declare const ThreadSchema: z.ZodObject<{
    threadId: z.ZodString;
    scope: z.ZodObject<{
        tenantId: z.ZodString;
        agentId: z.ZodString;
        projectId: z.ZodOptional<z.ZodString>;
        ownerUserId: z.ZodOptional<z.ZodString>;
        missionId: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>;
    createdByActorId: z.ZodString;
    title: z.ZodOptional<z.ZodString>;
    metadataJson: z.ZodOptional<z.ZodString>;
    goal: z.ZodOptional<z.ZodObject<{
        statement: z.ZodString;
        refinedAtSequence: z.ZodNumber;
        refinementCount: z.ZodNumber;
    }, z.core.$strip>>;
    createdAt: z.ZodOptional<z.ZodString>;
    updatedAt: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
export declare const ThreadListSchema: z.ZodObject<{
    data: z.ZodArray<z.ZodObject<{
        threadId: z.ZodString;
        scope: z.ZodObject<{
            tenantId: z.ZodString;
            agentId: z.ZodString;
            projectId: z.ZodOptional<z.ZodString>;
            ownerUserId: z.ZodOptional<z.ZodString>;
            missionId: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>;
        createdByActorId: z.ZodString;
        title: z.ZodOptional<z.ZodString>;
        metadataJson: z.ZodOptional<z.ZodString>;
        goal: z.ZodOptional<z.ZodObject<{
            statement: z.ZodString;
            refinedAtSequence: z.ZodNumber;
            refinementCount: z.ZodNumber;
        }, z.core.$strip>>;
        createdAt: z.ZodOptional<z.ZodString>;
        updatedAt: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>>;
    meta: z.ZodObject<{
        total: z.ZodNumber;
        hasMore: z.ZodBoolean;
        nextCursor: z.ZodNullable<z.ZodString>;
    }, z.core.$strip>;
}, z.core.$strip>;
export declare const ThreadCreateSchema: z.ZodObject<{
    threadId: z.ZodString;
    title: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
export declare const TimelineEntrySchema: z.ZodObject<{
    messageId: z.ZodString;
    threadId: z.ZodString;
    sequence: z.ZodNumber;
    kind: z.ZodString;
    status: z.ZodString;
    actorId: z.ZodOptional<z.ZodString>;
    sourceBindingId: z.ZodOptional<z.ZodString>;
    replyTargetBindingId: z.ZodOptional<z.ZodString>;
    turnId: z.ZodOptional<z.ZodString>;
    turnRunId: z.ZodOptional<z.ZodString>;
    toolResultRef: z.ZodOptional<z.ZodString>;
    content: z.ZodOptional<z.ZodString>;
    redactionRef: z.ZodOptional<z.ZodString>;
    role: z.ZodOptional<z.ZodString>;
    createdAt: z.ZodOptional<z.ZodString>;
    attachments: z.ZodOptional<z.ZodArray<z.ZodObject<{
        id: z.ZodString;
        kind: z.ZodEnum<{
            audio: "audio";
            image: "image";
            document: "document";
        }>;
        mimeType: z.ZodString;
        filename: z.ZodOptional<z.ZodString>;
        sizeBytes: z.ZodOptional<z.ZodNumber>;
        storageKey: z.ZodOptional<z.ZodString>;
        extractedText: z.ZodOptional<z.ZodString>;
        previewUrl: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>>>;
}, z.core.$strip>;
export declare const TimelineSchema: z.ZodObject<{
    data: z.ZodArray<z.ZodObject<{
        messageId: z.ZodString;
        threadId: z.ZodString;
        sequence: z.ZodNumber;
        kind: z.ZodString;
        status: z.ZodString;
        actorId: z.ZodOptional<z.ZodString>;
        sourceBindingId: z.ZodOptional<z.ZodString>;
        replyTargetBindingId: z.ZodOptional<z.ZodString>;
        turnId: z.ZodOptional<z.ZodString>;
        turnRunId: z.ZodOptional<z.ZodString>;
        toolResultRef: z.ZodOptional<z.ZodString>;
        content: z.ZodOptional<z.ZodString>;
        redactionRef: z.ZodOptional<z.ZodString>;
        role: z.ZodOptional<z.ZodString>;
        createdAt: z.ZodOptional<z.ZodString>;
        attachments: z.ZodOptional<z.ZodArray<z.ZodObject<{
            id: z.ZodString;
            kind: z.ZodEnum<{
                audio: "audio";
                image: "image";
                document: "document";
            }>;
            mimeType: z.ZodString;
            filename: z.ZodOptional<z.ZodString>;
            sizeBytes: z.ZodOptional<z.ZodNumber>;
            storageKey: z.ZodOptional<z.ZodString>;
            extractedText: z.ZodOptional<z.ZodString>;
            previewUrl: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>>>;
    }, z.core.$strip>>;
    meta: z.ZodObject<{
        total: z.ZodNumber;
        hasMore: z.ZodBoolean;
        nextCursor: z.ZodNullable<z.ZodString>;
    }, z.core.$strip>;
}, z.core.$strip>;
export declare const ThreadStateSchema: z.ZodObject<{
    thread: z.ZodObject<{
        threadId: z.ZodString;
        scope: z.ZodObject<{
            tenantId: z.ZodString;
            agentId: z.ZodString;
            projectId: z.ZodOptional<z.ZodString>;
            ownerUserId: z.ZodOptional<z.ZodString>;
            missionId: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>;
        createdByActorId: z.ZodString;
        title: z.ZodOptional<z.ZodString>;
        metadataJson: z.ZodOptional<z.ZodString>;
        goal: z.ZodOptional<z.ZodObject<{
            statement: z.ZodString;
            refinedAtSequence: z.ZodNumber;
            refinementCount: z.ZodNumber;
        }, z.core.$strip>>;
        createdAt: z.ZodOptional<z.ZodString>;
        updatedAt: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>;
    messages: z.ZodArray<z.ZodObject<{
        messageId: z.ZodString;
        threadId: z.ZodString;
        sequence: z.ZodNumber;
        kind: z.ZodString;
        status: z.ZodString;
        actorId: z.ZodOptional<z.ZodString>;
        sourceBindingId: z.ZodOptional<z.ZodString>;
        replyTargetBindingId: z.ZodOptional<z.ZodString>;
        turnId: z.ZodOptional<z.ZodString>;
        turnRunId: z.ZodOptional<z.ZodString>;
        toolResultRef: z.ZodOptional<z.ZodString>;
        content: z.ZodOptional<z.ZodString>;
        redactionRef: z.ZodOptional<z.ZodString>;
        role: z.ZodOptional<z.ZodString>;
        createdAt: z.ZodOptional<z.ZodString>;
        attachments: z.ZodOptional<z.ZodArray<z.ZodObject<{
            id: z.ZodString;
            kind: z.ZodEnum<{
                audio: "audio";
                image: "image";
                document: "document";
            }>;
            mimeType: z.ZodString;
            filename: z.ZodOptional<z.ZodString>;
            sizeBytes: z.ZodOptional<z.ZodNumber>;
            storageKey: z.ZodOptional<z.ZodString>;
            extractedText: z.ZodOptional<z.ZodString>;
            previewUrl: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>>>;
    }, z.core.$strip>>;
    summaryArtifacts: z.ZodArray<z.ZodObject<{
        summaryId: z.ZodString;
        threadId: z.ZodString;
        startSequence: z.ZodNumber;
        endSequence: z.ZodNumber;
        summaryKind: z.ZodString;
        content: z.ZodString;
    }, z.core.$strip>>;
}, z.core.$strip>;
export declare const AcceptedResponseSchema: z.ZodDiscriminatedUnion<[z.ZodObject<{
    outcome: z.ZodLiteral<"submitted">;
    runId: z.ZodString;
    turnId: z.ZodString;
    status: z.ZodString;
    resolvedRunProfileId: z.ZodString;
    resolvedRunProfileVersion: z.ZodNumber;
    eventCursor: z.ZodNumber;
    threadId: z.ZodString;
    acceptedMessageRef: z.ZodString;
}, z.core.$strip>, z.ZodObject<{
    outcome: z.ZodLiteral<"rejected_busy">;
    activeRunId: z.ZodOptional<z.ZodNullable<z.ZodString>>;
    status: z.ZodOptional<z.ZodNullable<z.ZodString>>;
    eventCursor: z.ZodOptional<z.ZodNullable<z.ZodNumber>>;
    notice: z.ZodString;
    threadId: z.ZodString;
    acceptedMessageRef: z.ZodString;
}, z.core.$strip>, z.ZodObject<{
    outcome: z.ZodLiteral<"already_submitted">;
    runId: z.ZodString;
    status: z.ZodString;
    eventCursor: z.ZodNumber;
    threadId: z.ZodString;
    acceptedMessageRef: z.ZodString;
}, z.core.$strip>], "outcome">;
export declare const GateResolutionSchema: z.ZodEnum<{
    approved: "approved";
    denied: "denied";
    credential_provided: "credential_provided";
    cancelled: "cancelled";
}>;
export declare const ChatEventSchema: z.ZodObject<{
    cursor: z.ZodOptional<z.ZodString>;
    type: z.ZodEnum<{
        cancelled: "cancelled";
        accepted: "accepted";
        running: "running";
        capability_progress: "capability_progress";
        capability_activity: "capability_activity";
        capability_display_preview: "capability_display_preview";
        gate: "gate";
        auth_required: "auth_required";
        final_reply: "final_reply";
        failed: "failed";
        projection_snapshot: "projection_snapshot";
        projection_update: "projection_update";
        keep_alive: "keep_alive";
    }>;
    ack: z.ZodOptional<z.ZodDiscriminatedUnion<[z.ZodObject<{
        outcome: z.ZodLiteral<"submitted">;
        threadId: z.ZodString;
        acceptedMessageRef: z.ZodString;
        runId: z.ZodString;
        turnId: z.ZodString;
        status: z.ZodString;
        eventCursor: z.ZodNumber;
    }, z.core.$strip>, z.ZodObject<{
        outcome: z.ZodLiteral<"rejected_busy">;
        threadId: z.ZodString;
        acceptedMessageRef: z.ZodString;
        activeRunId: z.ZodOptional<z.ZodNullable<z.ZodString>>;
        status: z.ZodOptional<z.ZodNullable<z.ZodString>>;
        eventCursor: z.ZodOptional<z.ZodNullable<z.ZodNumber>>;
        notice: z.ZodString;
    }, z.core.$strip>, z.ZodObject<{
        outcome: z.ZodLiteral<"already_submitted">;
        threadId: z.ZodString;
        acceptedMessageRef: z.ZodString;
        runId: z.ZodString;
        status: z.ZodString;
        eventCursor: z.ZodNumber;
    }, z.core.$strip>], "outcome">>;
    progress: z.ZodOptional<z.ZodObject<{
        kind: z.ZodString;
        turnRunId: z.ZodOptional<z.ZodString>;
        generatedAt: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>>;
    activity: z.ZodOptional<z.ZodObject<{
        invocationId: z.ZodCatch<z.ZodString>;
        turnRunId: z.ZodOptional<z.ZodString>;
        threadId: z.ZodOptional<z.ZodString>;
        capabilityId: z.ZodCatch<z.ZodString>;
        status: z.ZodCatch<z.ZodString>;
        provider: z.ZodOptional<z.ZodString>;
        runtime: z.ZodOptional<z.ZodString>;
        processId: z.ZodOptional<z.ZodNullable<z.ZodString>>;
        outputBytes: z.ZodOptional<z.ZodNullable<z.ZodNumber>>;
        errorKind: z.ZodOptional<z.ZodString>;
        updatedAt: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>>;
    preview: z.ZodOptional<z.ZodObject<{
        timelineMessageId: z.ZodOptional<z.ZodString>;
        invocationId: z.ZodCatch<z.ZodString>;
        turnRunId: z.ZodOptional<z.ZodString>;
        threadId: z.ZodOptional<z.ZodString>;
        capabilityId: z.ZodCatch<z.ZodString>;
        status: z.ZodCatch<z.ZodString>;
        title: z.ZodCatch<z.ZodString>;
        subtitle: z.ZodOptional<z.ZodString>;
        inputSummary: z.ZodOptional<z.ZodString>;
        outputSummary: z.ZodOptional<z.ZodString>;
        outputPreview: z.ZodOptional<z.ZodString>;
        outputKind: z.ZodOptional<z.ZodString>;
        outputBytes: z.ZodOptional<z.ZodNumber>;
        resultRef: z.ZodOptional<z.ZodString>;
        truncated: z.ZodCatch<z.ZodBoolean>;
        updatedAt: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>>;
    reply: z.ZodOptional<z.ZodObject<{
        text: z.ZodString;
        turnRunId: z.ZodString;
        generatedAt: z.ZodString;
    }, z.core.$strip>>;
    prompt: z.ZodOptional<z.ZodObject<{
        turnRunId: z.ZodCatch<z.ZodString>;
        gateRef: z.ZodCatch<z.ZodString>;
        headline: z.ZodCatch<z.ZodString>;
        body: z.ZodCatch<z.ZodString>;
        allowAlways: z.ZodOptional<z.ZodBoolean>;
        approvalContext: z.ZodOptional<z.ZodObject<{
            toolName: z.ZodString;
            action: z.ZodUnknown;
            scope: z.ZodUnknown;
            reason: z.ZodOptional<z.ZodString>;
            destination: z.ZodOptional<z.ZodUnknown>;
            details: z.ZodOptional<z.ZodArray<z.ZodUnknown>>;
        }, z.core.$strip>>;
    }, z.core.$strip>>;
    authPrompt: z.ZodOptional<z.ZodObject<{
        turnRunId: z.ZodString;
        authRequestRef: z.ZodString;
        headline: z.ZodString;
        body: z.ZodString;
        challengeKind: z.ZodOptional<z.ZodString>;
        provider: z.ZodOptional<z.ZodString>;
        accountLabel: z.ZodOptional<z.ZodString>;
        authorizationUrl: z.ZodOptional<z.ZodString>;
        expiresAt: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>>;
    response: z.ZodOptional<z.ZodObject<{
        runId: z.ZodCatch<z.ZodString>;
        status: z.ZodCatch<z.ZodString>;
        eventCursor: z.ZodOptional<z.ZodNumber>;
        alreadyTerminal: z.ZodOptional<z.ZodBoolean>;
    }, z.core.$strip>>;
    runState: z.ZodOptional<z.ZodObject<{
        turnId: z.ZodCatch<z.ZodString>;
        runId: z.ZodCatch<z.ZodString>;
        status: z.ZodCatch<z.ZodString>;
        eventCursor: z.ZodOptional<z.ZodNumber>;
        acceptedMessageRef: z.ZodCatch<z.ZodString>;
        resolvedRunProfileId: z.ZodCatch<z.ZodString>;
        resolvedRunProfileVersion: z.ZodCatch<z.ZodNumber>;
        receivedAt: z.ZodCatch<z.ZodString>;
        checkpointId: z.ZodOptional<z.ZodString>;
        gateRef: z.ZodOptional<z.ZodString>;
        failure: z.ZodOptional<z.ZodUnknown>;
    }, z.core.$strip>>;
    state: z.ZodOptional<z.ZodRecord<z.ZodString, z.ZodUnknown>>;
}, z.core.$strip>;
export declare const AuthProviderSchema: z.ZodObject<{
    id: z.ZodString;
    name: z.ZodString;
    type: z.ZodString;
}, z.core.$strip>;
export declare const AutomationSourceSchema: z.ZodDiscriminatedUnion<[z.ZodObject<{
    type: z.ZodLiteral<"schedule">;
    cron: z.ZodString;
    timezone: z.ZodString;
}, z.core.$strip>, z.ZodObject<{
    type: z.ZodLiteral<"once">;
    at: z.ZodString;
    timezone: z.ZodString;
}, z.core.$strip>], "type">;
export declare const AutomationRecentRunSchema: z.ZodObject<{
    runId: z.ZodOptional<z.ZodString>;
    threadId: z.ZodOptional<z.ZodString>;
    fireSlot: z.ZodOptional<z.ZodString>;
    status: z.ZodString;
    submittedAt: z.ZodString;
    completedAt: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
export declare const AutomationSchema: z.ZodObject<{
    id: z.ZodString;
    name: z.ZodString;
    source: z.ZodDiscriminatedUnion<[z.ZodObject<{
        type: z.ZodLiteral<"schedule">;
        cron: z.ZodString;
        timezone: z.ZodString;
    }, z.core.$strip>, z.ZodObject<{
        type: z.ZodLiteral<"once">;
        at: z.ZodString;
        timezone: z.ZodString;
    }, z.core.$strip>], "type">;
    state: z.ZodString;
    nextRunAt: z.ZodOptional<z.ZodString>;
    lastRunAt: z.ZodOptional<z.ZodString>;
    lastStatus: z.ZodOptional<z.ZodString>;
    recentRuns: z.ZodOptional<z.ZodArray<z.ZodObject<{
        runId: z.ZodOptional<z.ZodString>;
        threadId: z.ZodOptional<z.ZodString>;
        fireSlot: z.ZodOptional<z.ZodString>;
        status: z.ZodString;
        submittedAt: z.ZodString;
        completedAt: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>>>;
    isActive: z.ZodBoolean;
    createdAt: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
export declare const OutboundPreferencesSchema: z.ZodObject<{
    finalReplyTarget: z.ZodOptional<z.ZodObject<{
        targetId: z.ZodString;
        channel: z.ZodString;
        displayName: z.ZodString;
        description: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>>;
    status: z.ZodOptional<z.ZodString>;
    modality: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
export declare const OutboundTargetSummarySchema: z.ZodObject<{
    targetId: z.ZodString;
    channel: z.ZodString;
    displayName: z.ZodString;
    description: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
export declare const OutboundTargetCapabilitiesSchema: z.ZodObject<{
    finalReplies: z.ZodBoolean;
    gatePrompts: z.ZodBoolean;
    authPrompts: z.ZodBoolean;
}, z.core.$strip>;
export declare const OutboundTargetSchema: z.ZodObject<{
    target: z.ZodObject<{
        targetId: z.ZodString;
        channel: z.ZodString;
        displayName: z.ZodString;
        description: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>;
    capabilities: z.ZodObject<{
        finalReplies: z.ZodBoolean;
        gatePrompts: z.ZodBoolean;
        authPrompts: z.ZodBoolean;
    }, z.core.$strip>;
}, z.core.$strip>;
export declare const LifecyclePackageRefSchema: z.ZodObject<{
    kind: z.ZodString;
    id: z.ZodString;
}, z.core.$strip>;
export declare const ExtensionSchema: z.ZodObject<{
    packageRef: z.ZodObject<{
        kind: z.ZodString;
        id: z.ZodString;
    }, z.core.$strip>;
    displayName: z.ZodString;
    kind: z.ZodString;
    description: z.ZodString;
    authenticated: z.ZodBoolean;
    active: z.ZodBoolean;
    tools: z.ZodArray<z.ZodString>;
    needsSetup: z.ZodBoolean;
    hasAuth: z.ZodBoolean;
    activationStatus: z.ZodOptional<z.ZodString>;
    activationError: z.ZodOptional<z.ZodString>;
    version: z.ZodOptional<z.ZodString>;
    onboardingState: z.ZodOptional<z.ZodString>;
    onboarding: z.ZodOptional<z.ZodObject<{
        credentialInstructions: z.ZodOptional<z.ZodString>;
        setupUrl: z.ZodOptional<z.ZodString>;
        credentialNextStep: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>>;
}, z.core.$strip>;
export declare const ExtensionRegistryEntrySchema: z.ZodObject<{
    packageRef: z.ZodObject<{
        kind: z.ZodString;
        id: z.ZodString;
    }, z.core.$strip>;
    displayName: z.ZodString;
    kind: z.ZodString;
    description: z.ZodString;
    installed: z.ZodBoolean;
    keywords: z.ZodArray<z.ZodString>;
    version: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
export declare const ExtensionActionResponseSchema: z.ZodObject<{
    success: z.ZodBoolean;
    message: z.ZodString;
    activated: z.ZodOptional<z.ZodBoolean>;
    authUrl: z.ZodOptional<z.ZodString>;
    awaitingToken: z.ZodOptional<z.ZodBoolean>;
    instructions: z.ZodOptional<z.ZodString>;
    onboardingState: z.ZodOptional<z.ZodString>;
    onboarding: z.ZodOptional<z.ZodObject<{
        credentialInstructions: z.ZodOptional<z.ZodString>;
        setupUrl: z.ZodOptional<z.ZodString>;
        credentialNextStep: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>>;
}, z.core.$strip>;
export declare const ExtensionSetupSchema: z.ZodObject<{
    success: z.ZodBoolean;
    message: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
export declare const ExtensionSetupDetailSchema: z.ZodObject<{
    packageRef: z.ZodObject<{
        kind: z.ZodString;
        id: z.ZodString;
    }, z.core.$strip>;
    phase: z.ZodString;
    blockers: z.ZodArray<z.ZodUnknown>;
    payload: z.ZodOptional<z.ZodUnknown>;
    secrets: z.ZodArray<z.ZodObject<{
        name: z.ZodString;
        provider: z.ZodString;
        prompt: z.ZodString;
        optional: z.ZodBoolean;
        provided: z.ZodBoolean;
        setup: z.ZodUnion<readonly [z.ZodLiteral<"manual_token">, z.ZodObject<{
            kind: z.ZodLiteral<"oauth">;
            accountLabel: z.ZodString;
            scopes: z.ZodArray<z.ZodString>;
            invocationId: z.ZodString;
        }, z.core.$strip>]>;
        credentialRef: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>>;
    fields: z.ZodArray<z.ZodObject<{
        name: z.ZodString;
        prompt: z.ZodString;
        optional: z.ZodBoolean;
        placeholder: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>>;
    onboarding: z.ZodOptional<z.ZodObject<{
        credentialInstructions: z.ZodOptional<z.ZodString>;
        setupUrl: z.ZodOptional<z.ZodString>;
        credentialNextStep: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>>;
}, z.core.$strip>;
export declare const SkillSchema: z.ZodObject<{
    name: z.ZodString;
    description: z.ZodString;
    version: z.ZodString;
    trust: z.ZodString;
    source: z.ZodString;
    keywords: z.ZodArray<z.ZodString>;
    usageHint: z.ZodOptional<z.ZodString>;
    setupHint: z.ZodOptional<z.ZodString>;
    bundlePath: z.ZodOptional<z.ZodString>;
    installSourceUrl: z.ZodOptional<z.ZodString>;
    hasRequirements: z.ZodBoolean;
    hasScripts: z.ZodBoolean;
    canEdit: z.ZodBoolean;
    canDelete: z.ZodBoolean;
}, z.core.$strip>;
export declare const SkillActionResponseSchema: z.ZodObject<{
    success: z.ZodBoolean;
    message: z.ZodString;
}, z.core.$strip>;
export declare const SkillContentResponseSchema: z.ZodObject<{
    name: z.ZodString;
    content: z.ZodString;
}, z.core.$strip>;
export declare const SkillSearchResponseSchema: z.ZodObject<{
    catalog: z.ZodArray<z.ZodUnknown>;
    installed: z.ZodArray<z.ZodObject<{
        name: z.ZodString;
        description: z.ZodString;
        version: z.ZodString;
        trust: z.ZodString;
        source: z.ZodString;
        keywords: z.ZodArray<z.ZodString>;
        usageHint: z.ZodOptional<z.ZodString>;
        setupHint: z.ZodOptional<z.ZodString>;
        bundlePath: z.ZodOptional<z.ZodString>;
        installSourceUrl: z.ZodOptional<z.ZodString>;
        hasRequirements: z.ZodBoolean;
        hasScripts: z.ZodBoolean;
        canEdit: z.ZodBoolean;
        canDelete: z.ZodBoolean;
    }, z.core.$strip>>;
    registryUrl: z.ZodString;
    catalogError: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
export declare const ConnectableChannelSchema: z.ZodObject<{
    channel: z.ZodString;
    displayName: z.ZodString;
    strategy: z.ZodString;
    action: z.ZodObject<{
        title: z.ZodString;
        instructions: z.ZodString;
        inputPlaceholder: z.ZodString;
        submitLabel: z.ZodString;
        successMessage: z.ZodString;
        errorMessage: z.ZodString;
    }, z.core.$strip>;
    commandAliases: z.ZodArray<z.ZodString>;
}, z.core.$strip>;
export declare const FsMountInfoSchema: z.ZodObject<{
    mount: z.ZodString;
    label: z.ZodString;
}, z.core.$strip>;
export declare const FsEntrySchema: z.ZodObject<{
    name: z.ZodString;
    path: z.ZodString;
    kind: z.ZodEnum<{
        file: "file";
        directory: "directory";
        symlink: "symlink";
        other: "other";
    }>;
}, z.core.$strip>;
export declare const FsStatResponseSchema: z.ZodObject<{
    stat: z.ZodObject<{
        path: z.ZodString;
        kind: z.ZodEnum<{
            file: "file";
            directory: "directory";
            symlink: "symlink";
            other: "other";
        }>;
        sizeBytes: z.ZodNumber;
        mimeType: z.ZodString;
    }, z.core.$strip>;
}, z.core.$strip>;
export declare const FsContentResponseSchema: z.ZodObject<{
    contentBase64: z.ZodString;
    mimeType: z.ZodString;
    filename: z.ZodString;
    sizeBytes: z.ZodNumber;
}, z.core.$strip>;
export declare const ProjectSchema: z.ZodObject<{
    projectId: z.ZodString;
    name: z.ZodString;
    description: z.ZodOptional<z.ZodString>;
    createdAt: z.ZodOptional<z.ZodString>;
    updatedAt: z.ZodOptional<z.ZodString>;
    memberCount: z.ZodOptional<z.ZodNumber>;
}, z.core.$strip>;
export declare const ProjectMemberSchema: z.ZodObject<{
    userId: z.ZodString;
    role: z.ZodString;
    displayName: z.ZodOptional<z.ZodString>;
    email: z.ZodOptional<z.ZodString>;
}, z.core.$strip>;
export declare const contract: {
    ping: import("@orpc/contract").ContractProcedure<import("@orpc/contract").Schema<unknown, unknown>, z.ZodObject<{
        status: z.ZodLiteral<"ok">;
        timestamp: z.ZodISODateTime;
    }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, Record<never, never>>, Record<never, never>>;
    session: import("@orpc/contract").ContractProcedure<import("@orpc/contract").Schema<unknown, unknown>, z.ZodObject<{
        tenantId: z.ZodString;
        userId: z.ZodString;
        capabilities: z.ZodObject<{
            operatorWebuiConfig: z.ZodBoolean;
            attachments: z.ZodOptional<z.ZodObject<{
                accept: z.ZodArray<z.ZodString>;
                maxCount: z.ZodNumber;
                maxFileBytes: z.ZodNumber;
                maxTotalBytes: z.ZodNumber;
            }, z.core.$strip>>;
        }, z.core.$strip>;
        features: z.ZodOptional<z.ZodObject<{
            rebornProjects: z.ZodBoolean;
        }, z.core.$strip>>;
    }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
        UNAUTHORIZED: {
            status: number;
            message: string;
        };
        NOT_FOUND: {
            status: number;
            message: string;
        };
        BAD_REQUEST: {
            status: number;
            message: string;
        };
        CONFLICT: {
            status: number;
            message: string;
        };
        GATEWAY_ERROR: {
            status: number;
            message: string;
        };
    }>>, Record<never, never>>;
    threads: {
        list: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            limit: z.ZodOptional<z.ZodNumber>;
            cursor: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>, z.ZodObject<{
            data: z.ZodArray<z.ZodObject<{
                threadId: z.ZodString;
                scope: z.ZodObject<{
                    tenantId: z.ZodString;
                    agentId: z.ZodString;
                    projectId: z.ZodOptional<z.ZodString>;
                    ownerUserId: z.ZodOptional<z.ZodString>;
                    missionId: z.ZodOptional<z.ZodString>;
                }, z.core.$strip>;
                createdByActorId: z.ZodString;
                title: z.ZodOptional<z.ZodString>;
                metadataJson: z.ZodOptional<z.ZodString>;
                goal: z.ZodOptional<z.ZodObject<{
                    statement: z.ZodString;
                    refinedAtSequence: z.ZodNumber;
                    refinementCount: z.ZodNumber;
                }, z.core.$strip>>;
                createdAt: z.ZodOptional<z.ZodString>;
                updatedAt: z.ZodOptional<z.ZodString>;
            }, z.core.$strip>>;
            meta: z.ZodObject<{
                total: z.ZodNumber;
                hasMore: z.ZodBoolean;
                nextCursor: z.ZodNullable<z.ZodString>;
            }, z.core.$strip>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        create: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            clientActionId: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>, z.ZodObject<{
            threadId: z.ZodString;
            title: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        delete: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            id: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            success: z.ZodBoolean;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        sendMessage: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            id: z.ZodString;
            content: z.ZodString;
            clientActionId: z.ZodOptional<z.ZodString>;
            attachments: z.ZodOptional<z.ZodArray<z.ZodObject<{
                mimeType: z.ZodString;
                filename: z.ZodOptional<z.ZodString>;
                dataBase64: z.ZodString;
            }, z.core.$strip>>>;
        }, z.core.$strip>, z.ZodDiscriminatedUnion<[z.ZodObject<{
            outcome: z.ZodLiteral<"submitted">;
            runId: z.ZodString;
            turnId: z.ZodString;
            status: z.ZodString;
            resolvedRunProfileId: z.ZodString;
            resolvedRunProfileVersion: z.ZodNumber;
            eventCursor: z.ZodNumber;
            threadId: z.ZodString;
            acceptedMessageRef: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            outcome: z.ZodLiteral<"rejected_busy">;
            activeRunId: z.ZodOptional<z.ZodNullable<z.ZodString>>;
            status: z.ZodOptional<z.ZodNullable<z.ZodString>>;
            eventCursor: z.ZodOptional<z.ZodNullable<z.ZodNumber>>;
            notice: z.ZodString;
            threadId: z.ZodString;
            acceptedMessageRef: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            outcome: z.ZodLiteral<"already_submitted">;
            runId: z.ZodString;
            status: z.ZodString;
            eventCursor: z.ZodNumber;
            threadId: z.ZodString;
            acceptedMessageRef: z.ZodString;
        }, z.core.$strip>], "outcome">, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        getTimeline: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            id: z.ZodString;
            limit: z.ZodOptional<z.ZodNumber>;
            cursor: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>, z.ZodObject<{
            data: z.ZodArray<z.ZodObject<{
                messageId: z.ZodString;
                threadId: z.ZodString;
                sequence: z.ZodNumber;
                kind: z.ZodString;
                status: z.ZodString;
                actorId: z.ZodOptional<z.ZodString>;
                sourceBindingId: z.ZodOptional<z.ZodString>;
                replyTargetBindingId: z.ZodOptional<z.ZodString>;
                turnId: z.ZodOptional<z.ZodString>;
                turnRunId: z.ZodOptional<z.ZodString>;
                toolResultRef: z.ZodOptional<z.ZodString>;
                content: z.ZodOptional<z.ZodString>;
                redactionRef: z.ZodOptional<z.ZodString>;
                role: z.ZodOptional<z.ZodString>;
                createdAt: z.ZodOptional<z.ZodString>;
                attachments: z.ZodOptional<z.ZodArray<z.ZodObject<{
                    id: z.ZodString;
                    kind: z.ZodEnum<{
                        audio: "audio";
                        image: "image";
                        document: "document";
                    }>;
                    mimeType: z.ZodString;
                    filename: z.ZodOptional<z.ZodString>;
                    sizeBytes: z.ZodOptional<z.ZodNumber>;
                    storageKey: z.ZodOptional<z.ZodString>;
                    extractedText: z.ZodOptional<z.ZodString>;
                    previewUrl: z.ZodOptional<z.ZodString>;
                }, z.core.$strip>>>;
            }, z.core.$strip>>;
            meta: z.ZodObject<{
                total: z.ZodNumber;
                hasMore: z.ZodBoolean;
                nextCursor: z.ZodNullable<z.ZodString>;
            }, z.core.$strip>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        cancelRun: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            id: z.ZodString;
            runId: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            success: z.ZodBoolean;
            runId: z.ZodOptional<z.ZodString>;
            status: z.ZodOptional<z.ZodString>;
            eventCursor: z.ZodOptional<z.ZodNumber>;
            alreadyTerminal: z.ZodOptional<z.ZodBoolean>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        resolveGate: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            id: z.ZodString;
            runId: z.ZodString;
            gateRef: z.ZodCatch<z.ZodString>;
            resolution: z.ZodEnum<{
                approved: "approved";
                denied: "denied";
                credential_provided: "credential_provided";
                cancelled: "cancelled";
            }>;
            always: z.ZodOptional<z.ZodBoolean>;
            credentialRef: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>, z.ZodObject<{
            success: z.ZodBoolean;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        streamEvents: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            id: z.ZodString;
            afterCursor: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>, import("@orpc/contract").Schema<AsyncIteratorObject<{
            type: "cancelled" | "accepted" | "running" | "capability_progress" | "capability_activity" | "capability_display_preview" | "gate" | "auth_required" | "final_reply" | "failed" | "projection_snapshot" | "projection_update" | "keep_alive";
            cursor?: string | undefined;
            ack?: {
                outcome: "submitted";
                threadId: string;
                acceptedMessageRef: string;
                runId: string;
                turnId: string;
                status: string;
                eventCursor: number;
            } | {
                outcome: "rejected_busy";
                threadId: string;
                acceptedMessageRef: string;
                notice: string;
                activeRunId?: string | null | undefined;
                status?: string | null | undefined;
                eventCursor?: number | null | undefined;
            } | {
                outcome: "already_submitted";
                threadId: string;
                acceptedMessageRef: string;
                runId: string;
                status: string;
                eventCursor: number;
            } | undefined;
            progress?: {
                kind: string;
                turnRunId?: string | undefined;
                generatedAt?: string | undefined;
            } | undefined;
            activity?: {
                invocationId: string;
                capabilityId: string;
                status: string;
                turnRunId?: string | undefined;
                threadId?: string | undefined;
                provider?: string | undefined;
                runtime?: string | undefined;
                processId?: string | null | undefined;
                outputBytes?: number | null | undefined;
                errorKind?: string | undefined;
                updatedAt?: string | undefined;
            } | undefined;
            preview?: {
                invocationId: string;
                capabilityId: string;
                status: string;
                title: string;
                truncated: boolean;
                timelineMessageId?: string | undefined;
                turnRunId?: string | undefined;
                threadId?: string | undefined;
                subtitle?: string | undefined;
                inputSummary?: string | undefined;
                outputSummary?: string | undefined;
                outputPreview?: string | undefined;
                outputKind?: string | undefined;
                outputBytes?: number | undefined;
                resultRef?: string | undefined;
                updatedAt?: string | undefined;
            } | undefined;
            reply?: {
                text: string;
                turnRunId: string;
                generatedAt: string;
            } | undefined;
            prompt?: {
                turnRunId: string;
                gateRef: string;
                headline: string;
                body: string;
                allowAlways?: boolean | undefined;
                approvalContext?: {
                    toolName: string;
                    action: unknown;
                    scope: unknown;
                    reason?: string | undefined;
                    destination?: unknown;
                    details?: unknown[] | undefined;
                } | undefined;
            } | undefined;
            authPrompt?: {
                turnRunId: string;
                authRequestRef: string;
                headline: string;
                body: string;
                challengeKind?: string | undefined;
                provider?: string | undefined;
                accountLabel?: string | undefined;
                authorizationUrl?: string | undefined;
                expiresAt?: string | undefined;
            } | undefined;
            response?: {
                runId: string;
                status: string;
                eventCursor?: number | undefined;
                alreadyTerminal?: boolean | undefined;
            } | undefined;
            runState?: {
                turnId: string;
                runId: string;
                status: string;
                acceptedMessageRef: string;
                resolvedRunProfileId: string;
                resolvedRunProfileVersion: number;
                receivedAt: string;
                eventCursor?: number | undefined;
                checkpointId?: string | undefined;
                gateRef?: string | undefined;
                failure?: unknown;
            } | undefined;
            state?: Record<string, unknown> | undefined;
        }, unknown, void>, import("@orpc/shared").AsyncIteratorClass<{
            type: "cancelled" | "accepted" | "running" | "capability_progress" | "capability_activity" | "capability_display_preview" | "gate" | "auth_required" | "final_reply" | "failed" | "projection_snapshot" | "projection_update" | "keep_alive";
            cursor?: string | undefined;
            ack?: {
                outcome: "submitted";
                threadId: string;
                acceptedMessageRef: string;
                runId: string;
                turnId: string;
                status: string;
                eventCursor: number;
            } | {
                outcome: "rejected_busy";
                threadId: string;
                acceptedMessageRef: string;
                notice: string;
                activeRunId?: string | null | undefined;
                status?: string | null | undefined;
                eventCursor?: number | null | undefined;
            } | {
                outcome: "already_submitted";
                threadId: string;
                acceptedMessageRef: string;
                runId: string;
                status: string;
                eventCursor: number;
            } | undefined;
            progress?: {
                kind: string;
                turnRunId?: string | undefined;
                generatedAt?: string | undefined;
            } | undefined;
            activity?: {
                invocationId: string;
                capabilityId: string;
                status: string;
                turnRunId?: string | undefined;
                threadId?: string | undefined;
                provider?: string | undefined;
                runtime?: string | undefined;
                processId?: string | null | undefined;
                outputBytes?: number | null | undefined;
                errorKind?: string | undefined;
                updatedAt?: string | undefined;
            } | undefined;
            preview?: {
                invocationId: string;
                capabilityId: string;
                status: string;
                title: string;
                truncated: boolean;
                timelineMessageId?: string | undefined;
                turnRunId?: string | undefined;
                threadId?: string | undefined;
                subtitle?: string | undefined;
                inputSummary?: string | undefined;
                outputSummary?: string | undefined;
                outputPreview?: string | undefined;
                outputKind?: string | undefined;
                outputBytes?: number | undefined;
                resultRef?: string | undefined;
                updatedAt?: string | undefined;
            } | undefined;
            reply?: {
                text: string;
                turnRunId: string;
                generatedAt: string;
            } | undefined;
            prompt?: {
                turnRunId: string;
                gateRef: string;
                headline: string;
                body: string;
                allowAlways?: boolean | undefined;
                approvalContext?: {
                    toolName: string;
                    action: unknown;
                    scope: unknown;
                    reason?: string | undefined;
                    destination?: unknown;
                    details?: unknown[] | undefined;
                } | undefined;
            } | undefined;
            authPrompt?: {
                turnRunId: string;
                authRequestRef: string;
                headline: string;
                body: string;
                challengeKind?: string | undefined;
                provider?: string | undefined;
                accountLabel?: string | undefined;
                authorizationUrl?: string | undefined;
                expiresAt?: string | undefined;
            } | undefined;
            response?: {
                runId: string;
                status: string;
                eventCursor?: number | undefined;
                alreadyTerminal?: boolean | undefined;
            } | undefined;
            runState?: {
                turnId: string;
                runId: string;
                status: string;
                acceptedMessageRef: string;
                resolvedRunProfileId: string;
                resolvedRunProfileVersion: number;
                receivedAt: string;
                eventCursor?: number | undefined;
                checkpointId?: string | undefined;
                gateRef?: string | undefined;
                failure?: unknown;
            } | undefined;
            state?: Record<string, unknown> | undefined;
        }, unknown, void>>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        getState: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            id: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            thread: z.ZodObject<{
                threadId: z.ZodString;
                scope: z.ZodObject<{
                    tenantId: z.ZodString;
                    agentId: z.ZodString;
                    projectId: z.ZodOptional<z.ZodString>;
                    ownerUserId: z.ZodOptional<z.ZodString>;
                    missionId: z.ZodOptional<z.ZodString>;
                }, z.core.$strip>;
                createdByActorId: z.ZodString;
                title: z.ZodOptional<z.ZodString>;
                metadataJson: z.ZodOptional<z.ZodString>;
                goal: z.ZodOptional<z.ZodObject<{
                    statement: z.ZodString;
                    refinedAtSequence: z.ZodNumber;
                    refinementCount: z.ZodNumber;
                }, z.core.$strip>>;
                createdAt: z.ZodOptional<z.ZodString>;
                updatedAt: z.ZodOptional<z.ZodString>;
            }, z.core.$strip>;
            messages: z.ZodArray<z.ZodObject<{
                messageId: z.ZodString;
                threadId: z.ZodString;
                sequence: z.ZodNumber;
                kind: z.ZodString;
                status: z.ZodString;
                actorId: z.ZodOptional<z.ZodString>;
                sourceBindingId: z.ZodOptional<z.ZodString>;
                replyTargetBindingId: z.ZodOptional<z.ZodString>;
                turnId: z.ZodOptional<z.ZodString>;
                turnRunId: z.ZodOptional<z.ZodString>;
                toolResultRef: z.ZodOptional<z.ZodString>;
                content: z.ZodOptional<z.ZodString>;
                redactionRef: z.ZodOptional<z.ZodString>;
                role: z.ZodOptional<z.ZodString>;
                createdAt: z.ZodOptional<z.ZodString>;
                attachments: z.ZodOptional<z.ZodArray<z.ZodObject<{
                    id: z.ZodString;
                    kind: z.ZodEnum<{
                        audio: "audio";
                        image: "image";
                        document: "document";
                    }>;
                    mimeType: z.ZodString;
                    filename: z.ZodOptional<z.ZodString>;
                    sizeBytes: z.ZodOptional<z.ZodNumber>;
                    storageKey: z.ZodOptional<z.ZodString>;
                    extractedText: z.ZodOptional<z.ZodString>;
                    previewUrl: z.ZodOptional<z.ZodString>;
                }, z.core.$strip>>>;
            }, z.core.$strip>>;
            summaryArtifacts: z.ZodArray<z.ZodObject<{
                summaryId: z.ZodString;
                threadId: z.ZodString;
                startSequence: z.ZodNumber;
                endSequence: z.ZodNumber;
                summaryKind: z.ZodString;
                content: z.ZodString;
            }, z.core.$strip>>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        listFiles: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            id: z.ZodString;
            path: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>, z.ZodObject<{
            entries: z.ZodArray<z.ZodObject<{
                name: z.ZodString;
                path: z.ZodString;
                kind: z.ZodEnum<{
                    file: "file";
                    directory: "directory";
                    symlink: "symlink";
                    other: "other";
                }>;
            }, z.core.$strip>>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        statFile: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            id: z.ZodString;
            path: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            path: z.ZodString;
            kind: z.ZodEnum<{
                file: "file";
                directory: "directory";
                symlink: "symlink";
                other: "other";
            }>;
            sizeBytes: z.ZodNumber;
            mimeType: z.ZodString;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        downloadFile: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            id: z.ZodString;
            path: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            contentBase64: z.ZodString;
            mimeType: z.ZodString;
            filename: z.ZodString;
            sizeBytes: z.ZodNumber;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        getAttachment: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            id: z.ZodString;
            messageId: z.ZodString;
            attachmentId: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            contentBase64: z.ZodString;
            mimeType: z.ZodString;
            filename: z.ZodString;
            sizeBytes: z.ZodNumber;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
    };
    automations: {
        list: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            limit: z.ZodOptional<z.ZodNumber>;
            runLimit: z.ZodOptional<z.ZodNumber>;
        }, z.core.$strip>, z.ZodObject<{
            data: z.ZodArray<z.ZodObject<{
                id: z.ZodString;
                name: z.ZodString;
                source: z.ZodDiscriminatedUnion<[z.ZodObject<{
                    type: z.ZodLiteral<"schedule">;
                    cron: z.ZodString;
                    timezone: z.ZodString;
                }, z.core.$strip>, z.ZodObject<{
                    type: z.ZodLiteral<"once">;
                    at: z.ZodString;
                    timezone: z.ZodString;
                }, z.core.$strip>], "type">;
                state: z.ZodString;
                nextRunAt: z.ZodOptional<z.ZodString>;
                lastRunAt: z.ZodOptional<z.ZodString>;
                lastStatus: z.ZodOptional<z.ZodString>;
                recentRuns: z.ZodOptional<z.ZodArray<z.ZodObject<{
                    runId: z.ZodOptional<z.ZodString>;
                    threadId: z.ZodOptional<z.ZodString>;
                    fireSlot: z.ZodOptional<z.ZodString>;
                    status: z.ZodString;
                    submittedAt: z.ZodString;
                    completedAt: z.ZodOptional<z.ZodString>;
                }, z.core.$strip>>>;
                isActive: z.ZodBoolean;
                createdAt: z.ZodOptional<z.ZodString>;
            }, z.core.$strip>>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
    };
    outbound: {
        getPreferences: import("@orpc/contract").ContractProcedure<import("@orpc/contract").Schema<unknown, unknown>, z.ZodObject<{
            finalReplyTarget: z.ZodOptional<z.ZodObject<{
                targetId: z.ZodString;
                channel: z.ZodString;
                displayName: z.ZodString;
                description: z.ZodOptional<z.ZodString>;
            }, z.core.$strip>>;
            status: z.ZodOptional<z.ZodString>;
            modality: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        setPreferences: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            finalReplyTarget: z.ZodOptional<z.ZodObject<{
                targetId: z.ZodString;
                channel: z.ZodString;
                displayName: z.ZodString;
                description: z.ZodOptional<z.ZodString>;
            }, z.core.$strip>>;
            status: z.ZodOptional<z.ZodString>;
            modality: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>, z.ZodObject<{
            success: z.ZodBoolean;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        listTargets: import("@orpc/contract").ContractProcedure<import("@orpc/contract").Schema<unknown, unknown>, z.ZodObject<{
            data: z.ZodArray<z.ZodObject<{
                target: z.ZodObject<{
                    targetId: z.ZodString;
                    channel: z.ZodString;
                    displayName: z.ZodString;
                    description: z.ZodOptional<z.ZodString>;
                }, z.core.$strip>;
                capabilities: z.ZodObject<{
                    finalReplies: z.ZodBoolean;
                    gatePrompts: z.ZodBoolean;
                    authPrompts: z.ZodBoolean;
                }, z.core.$strip>;
            }, z.core.$strip>>;
            nextCursor: z.ZodOptional<z.ZodNullable<z.ZodString>>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
    };
    extensions: {
        list: import("@orpc/contract").ContractProcedure<import("@orpc/contract").Schema<unknown, unknown>, z.ZodObject<{
            data: z.ZodArray<z.ZodObject<{
                packageRef: z.ZodObject<{
                    kind: z.ZodString;
                    id: z.ZodString;
                }, z.core.$strip>;
                displayName: z.ZodString;
                kind: z.ZodString;
                description: z.ZodString;
                authenticated: z.ZodBoolean;
                active: z.ZodBoolean;
                tools: z.ZodArray<z.ZodString>;
                needsSetup: z.ZodBoolean;
                hasAuth: z.ZodBoolean;
                activationStatus: z.ZodOptional<z.ZodString>;
                activationError: z.ZodOptional<z.ZodString>;
                version: z.ZodOptional<z.ZodString>;
                onboardingState: z.ZodOptional<z.ZodString>;
                onboarding: z.ZodOptional<z.ZodObject<{
                    credentialInstructions: z.ZodOptional<z.ZodString>;
                    setupUrl: z.ZodOptional<z.ZodString>;
                    credentialNextStep: z.ZodOptional<z.ZodString>;
                }, z.core.$strip>>;
            }, z.core.$strip>>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        listRegistry: import("@orpc/contract").ContractProcedure<import("@orpc/contract").Schema<unknown, unknown>, z.ZodObject<{
            data: z.ZodArray<z.ZodObject<{
                packageRef: z.ZodObject<{
                    kind: z.ZodString;
                    id: z.ZodString;
                }, z.core.$strip>;
                displayName: z.ZodString;
                kind: z.ZodString;
                description: z.ZodString;
                installed: z.ZodBoolean;
                keywords: z.ZodArray<z.ZodString>;
                version: z.ZodOptional<z.ZodString>;
            }, z.core.$strip>>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        install: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            packageRef: z.ZodObject<{
                kind: z.ZodString;
                id: z.ZodString;
            }, z.core.$strip>;
        }, z.core.$strip>, z.ZodObject<{
            success: z.ZodBoolean;
            message: z.ZodString;
            activated: z.ZodOptional<z.ZodBoolean>;
            authUrl: z.ZodOptional<z.ZodString>;
            awaitingToken: z.ZodOptional<z.ZodBoolean>;
            instructions: z.ZodOptional<z.ZodString>;
            onboardingState: z.ZodOptional<z.ZodString>;
            onboarding: z.ZodOptional<z.ZodObject<{
                credentialInstructions: z.ZodOptional<z.ZodString>;
                setupUrl: z.ZodOptional<z.ZodString>;
                credentialNextStep: z.ZodOptional<z.ZodString>;
            }, z.core.$strip>>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        activate: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            name: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            success: z.ZodBoolean;
            message: z.ZodString;
            activated: z.ZodOptional<z.ZodBoolean>;
            authUrl: z.ZodOptional<z.ZodString>;
            awaitingToken: z.ZodOptional<z.ZodBoolean>;
            instructions: z.ZodOptional<z.ZodString>;
            onboardingState: z.ZodOptional<z.ZodString>;
            onboarding: z.ZodOptional<z.ZodObject<{
                credentialInstructions: z.ZodOptional<z.ZodString>;
                setupUrl: z.ZodOptional<z.ZodString>;
                credentialNextStep: z.ZodOptional<z.ZodString>;
            }, z.core.$strip>>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        remove: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            name: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            success: z.ZodBoolean;
            message: z.ZodString;
            activated: z.ZodOptional<z.ZodBoolean>;
            authUrl: z.ZodOptional<z.ZodString>;
            awaitingToken: z.ZodOptional<z.ZodBoolean>;
            instructions: z.ZodOptional<z.ZodString>;
            onboardingState: z.ZodOptional<z.ZodString>;
            onboarding: z.ZodOptional<z.ZodObject<{
                credentialInstructions: z.ZodOptional<z.ZodString>;
                setupUrl: z.ZodOptional<z.ZodString>;
                credentialNextStep: z.ZodOptional<z.ZodString>;
            }, z.core.$strip>>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        getSetup: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            name: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            packageRef: z.ZodObject<{
                kind: z.ZodString;
                id: z.ZodString;
            }, z.core.$strip>;
            phase: z.ZodString;
            blockers: z.ZodArray<z.ZodUnknown>;
            payload: z.ZodOptional<z.ZodUnknown>;
            secrets: z.ZodArray<z.ZodObject<{
                name: z.ZodString;
                provider: z.ZodString;
                prompt: z.ZodString;
                optional: z.ZodBoolean;
                provided: z.ZodBoolean;
                setup: z.ZodUnion<readonly [z.ZodLiteral<"manual_token">, z.ZodObject<{
                    kind: z.ZodLiteral<"oauth">;
                    accountLabel: z.ZodString;
                    scopes: z.ZodArray<z.ZodString>;
                    invocationId: z.ZodString;
                }, z.core.$strip>]>;
                credentialRef: z.ZodOptional<z.ZodString>;
            }, z.core.$strip>>;
            fields: z.ZodArray<z.ZodObject<{
                name: z.ZodString;
                prompt: z.ZodString;
                optional: z.ZodBoolean;
                placeholder: z.ZodOptional<z.ZodString>;
            }, z.core.$strip>>;
            onboarding: z.ZodOptional<z.ZodObject<{
                credentialInstructions: z.ZodOptional<z.ZodString>;
                setupUrl: z.ZodOptional<z.ZodString>;
                credentialNextStep: z.ZodOptional<z.ZodString>;
            }, z.core.$strip>>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        setup: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            name: z.ZodString;
            action: z.ZodString;
            payload: z.ZodOptional<z.ZodRecord<z.ZodString, z.ZodUnknown>>;
        }, z.core.$strip>, z.ZodObject<{
            success: z.ZodBoolean;
            message: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
    };
    skills: {
        list: import("@orpc/contract").ContractProcedure<import("@orpc/contract").Schema<unknown, unknown>, z.ZodObject<{
            data: z.ZodArray<z.ZodObject<{
                name: z.ZodString;
                description: z.ZodString;
                version: z.ZodString;
                trust: z.ZodString;
                source: z.ZodString;
                keywords: z.ZodArray<z.ZodString>;
                usageHint: z.ZodOptional<z.ZodString>;
                setupHint: z.ZodOptional<z.ZodString>;
                bundlePath: z.ZodOptional<z.ZodString>;
                installSourceUrl: z.ZodOptional<z.ZodString>;
                hasRequirements: z.ZodBoolean;
                hasScripts: z.ZodBoolean;
                canEdit: z.ZodBoolean;
                canDelete: z.ZodBoolean;
            }, z.core.$strip>>;
            count: z.ZodNumber;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        search: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            query: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            catalog: z.ZodArray<z.ZodUnknown>;
            installed: z.ZodArray<z.ZodObject<{
                name: z.ZodString;
                description: z.ZodString;
                version: z.ZodString;
                trust: z.ZodString;
                source: z.ZodString;
                keywords: z.ZodArray<z.ZodString>;
                usageHint: z.ZodOptional<z.ZodString>;
                setupHint: z.ZodOptional<z.ZodString>;
                bundlePath: z.ZodOptional<z.ZodString>;
                installSourceUrl: z.ZodOptional<z.ZodString>;
                hasRequirements: z.ZodBoolean;
                hasScripts: z.ZodBoolean;
                canEdit: z.ZodBoolean;
                canDelete: z.ZodBoolean;
            }, z.core.$strip>>;
            registryUrl: z.ZodString;
            catalogError: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        install: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            name: z.ZodString;
            content: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>, z.ZodObject<{
            success: z.ZodBoolean;
            message: z.ZodString;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        get: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            name: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            name: z.ZodString;
            content: z.ZodString;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        update: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            name: z.ZodString;
            content: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            success: z.ZodBoolean;
            message: z.ZodString;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        remove: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            name: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            success: z.ZodBoolean;
            message: z.ZodString;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
    };
    channels: {
        listConnectable: import("@orpc/contract").ContractProcedure<import("@orpc/contract").Schema<unknown, unknown>, z.ZodObject<{
            data: z.ZodArray<z.ZodObject<{
                channel: z.ZodString;
                displayName: z.ZodString;
                strategy: z.ZodString;
                action: z.ZodObject<{
                    title: z.ZodString;
                    instructions: z.ZodString;
                    inputPlaceholder: z.ZodString;
                    submitLabel: z.ZodString;
                    successMessage: z.ZodString;
                    errorMessage: z.ZodString;
                }, z.core.$strip>;
                commandAliases: z.ZodArray<z.ZodString>;
            }, z.core.$strip>>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
    };
    operator: {
        createAccessSession: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            tenantId: z.ZodString;
            agentId: z.ZodOptional<z.ZodString>;
            projectId: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>, z.ZodObject<{
            token: z.ZodString;
            expiresAt: z.ZodISODateTime;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
    };
    fs: {
        mounts: import("@orpc/contract").ContractProcedure<import("@orpc/contract").Schema<unknown, unknown>, z.ZodObject<{
            mounts: z.ZodArray<z.ZodObject<{
                mount: z.ZodString;
                label: z.ZodString;
            }, z.core.$strip>>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        list: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            mount: z.ZodString;
            path: z.ZodDefault<z.ZodString>;
        }, z.core.$strip>, z.ZodObject<{
            mount: z.ZodString;
            path: z.ZodString;
            entries: z.ZodArray<z.ZodObject<{
                name: z.ZodString;
                path: z.ZodString;
                kind: z.ZodEnum<{
                    file: "file";
                    directory: "directory";
                    symlink: "symlink";
                    other: "other";
                }>;
            }, z.core.$strip>>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        stat: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            mount: z.ZodString;
            path: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            stat: z.ZodObject<{
                path: z.ZodString;
                kind: z.ZodEnum<{
                    file: "file";
                    directory: "directory";
                    symlink: "symlink";
                    other: "other";
                }>;
                sizeBytes: z.ZodNumber;
                mimeType: z.ZodString;
            }, z.core.$strip>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        content: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            mount: z.ZodString;
            path: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            contentBase64: z.ZodString;
            mimeType: z.ZodString;
            filename: z.ZodString;
            sizeBytes: z.ZodNumber;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
    };
    projects: {
        list: import("@orpc/contract").ContractProcedure<import("@orpc/contract").Schema<unknown, unknown>, z.ZodObject<{
            projects: z.ZodArray<z.ZodObject<{
                projectId: z.ZodString;
                name: z.ZodString;
                description: z.ZodOptional<z.ZodString>;
                createdAt: z.ZodOptional<z.ZodString>;
                updatedAt: z.ZodOptional<z.ZodString>;
                memberCount: z.ZodOptional<z.ZodNumber>;
            }, z.core.$strip>>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        create: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            name: z.ZodString;
            description: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>, z.ZodObject<{
            projectId: z.ZodString;
            name: z.ZodString;
            description: z.ZodOptional<z.ZodString>;
            createdAt: z.ZodOptional<z.ZodString>;
            updatedAt: z.ZodOptional<z.ZodString>;
            memberCount: z.ZodOptional<z.ZodNumber>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        get: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            id: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            projectId: z.ZodString;
            name: z.ZodString;
            description: z.ZodOptional<z.ZodString>;
            createdAt: z.ZodOptional<z.ZodString>;
            updatedAt: z.ZodOptional<z.ZodString>;
            memberCount: z.ZodOptional<z.ZodNumber>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        update: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            id: z.ZodString;
            name: z.ZodOptional<z.ZodString>;
            description: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>, z.ZodObject<{
            projectId: z.ZodString;
            name: z.ZodString;
            description: z.ZodOptional<z.ZodString>;
            createdAt: z.ZodOptional<z.ZodString>;
            updatedAt: z.ZodOptional<z.ZodString>;
            memberCount: z.ZodOptional<z.ZodNumber>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        delete: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            id: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            success: z.ZodBoolean;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        listMembers: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            id: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            members: z.ZodArray<z.ZodObject<{
                userId: z.ZodString;
                role: z.ZodString;
                displayName: z.ZodOptional<z.ZodString>;
                email: z.ZodOptional<z.ZodString>;
            }, z.core.$strip>>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        addMember: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            id: z.ZodString;
            userId: z.ZodString;
            role: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            userId: z.ZodString;
            role: z.ZodString;
            displayName: z.ZodOptional<z.ZodString>;
            email: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        updateMember: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            id: z.ZodString;
            userId: z.ZodString;
            role: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            userId: z.ZodString;
            role: z.ZodString;
            displayName: z.ZodOptional<z.ZodString>;
            email: z.ZodOptional<z.ZodString>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        removeMember: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            id: z.ZodString;
            userId: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            success: z.ZodBoolean;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
    };
    auth: {
        listProviders: import("@orpc/contract").ContractProcedure<import("@orpc/contract").Schema<unknown, unknown>, z.ZodObject<{
            data: z.ZodArray<z.ZodObject<{
                id: z.ZodString;
                name: z.ZodString;
                type: z.ZodString;
            }, z.core.$strip>>;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        exchangeLoginTicket: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            loginTicket: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            token: z.ZodString;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        logout: import("@orpc/contract").ContractProcedure<import("@orpc/contract").Schema<unknown, unknown>, z.ZodObject<{
            success: z.ZodBoolean;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
        submitManualToken: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            provider: z.ZodString;
            accountLabel: z.ZodString;
            token: z.ZodString;
            threadId: z.ZodString;
            runId: z.ZodString;
            gateRef: z.ZodString;
        }, z.core.$strip>, z.ZodObject<{
            credentialRef: z.ZodString;
        }, z.core.$strip>, import("@orpc/contract").MergedErrorMap<Record<never, never>, import("@orpc/contract").MergedErrorMap<Record<never, never>, {
            UNAUTHORIZED: {
                status: number;
                message: string;
            };
            NOT_FOUND: {
                status: number;
                message: string;
            };
            BAD_REQUEST: {
                status: number;
                message: string;
            };
            CONFLICT: {
                status: number;
                message: string;
            };
            GATEWAY_ERROR: {
                status: number;
                message: string;
            };
        }>>, Record<never, never>>;
    };
};
export type ContractType = typeof contract;
export type ChatEvent = z.infer<typeof ChatEventSchema>;
export type AcceptedResponse = z.infer<typeof AcceptedResponseSchema>;
export type TimelineEntry = z.infer<typeof TimelineEntrySchema>;
export type Thread = z.infer<typeof ThreadSchema>;
export type Session = z.infer<typeof SessionSchema>;
export type GateResolution = z.infer<typeof GateResolutionSchema>;
export type ThreadCreate = z.infer<typeof ThreadCreateSchema>;
export type FsMountInfo = z.infer<typeof FsMountInfoSchema>;
export type FsEntry = z.infer<typeof FsEntrySchema>;
export type FsStatResponse = z.infer<typeof FsStatResponseSchema>;
export type FsContentResponse = z.infer<typeof FsContentResponseSchema>;
export type Project = z.infer<typeof ProjectSchema>;
export type ProjectMember = z.infer<typeof ProjectMemberSchema>;
