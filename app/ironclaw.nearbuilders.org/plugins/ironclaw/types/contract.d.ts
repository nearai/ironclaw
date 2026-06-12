import { z } from "every-plugin/zod";
export declare const SessionSchema: z.ZodObject<{
    tenantId: z.ZodString;
    userId: z.ZodString;
    capabilities: z.ZodObject<{
        operatorWebuiConfig: z.ZodBoolean;
    }, z.core.$strip>;
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
    }, z.core.$strip>>;
    meta: z.ZodObject<{
        total: z.ZodNumber;
        hasMore: z.ZodBoolean;
        nextCursor: z.ZodNullable<z.ZodString>;
    }, z.core.$strip>;
}, z.core.$strip>;
export declare const AcceptedResponseSchema: z.ZodObject<{
    outcome: z.ZodString;
    threadId: z.ZodString;
    acceptedMessageRef: z.ZodString;
    runId: z.ZodOptional<z.ZodString>;
    activeRunId: z.ZodOptional<z.ZodString>;
    turnId: z.ZodOptional<z.ZodString>;
    status: z.ZodString;
    resolvedRunProfileId: z.ZodOptional<z.ZodString>;
    resolvedRunProfileVersion: z.ZodOptional<z.ZodNumber>;
    eventCursor: z.ZodOptional<z.ZodNumber>;
}, z.core.$strip>;
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
    ack: z.ZodOptional<z.ZodObject<{
        outcome: z.ZodString;
        threadId: z.ZodString;
        runId: z.ZodOptional<z.ZodString>;
        activeRunId: z.ZodOptional<z.ZodString>;
        acceptedMessageRef: z.ZodString;
        status: z.ZodString;
        turnId: z.ZodOptional<z.ZodString>;
        eventCursor: z.ZodOptional<z.ZodNumber>;
    }, z.core.$strip>>;
    progress: z.ZodOptional<z.ZodObject<{
        kind: z.ZodString;
        turnRunId: z.ZodOptional<z.ZodString>;
        generatedAt: z.ZodOptional<z.ZodString>;
    }, z.core.$strip>>;
    activity: z.ZodOptional<z.ZodObject<{
        invocationId: z.ZodString;
        turnRunId: z.ZodOptional<z.ZodString>;
        threadId: z.ZodOptional<z.ZodString>;
        capabilityId: z.ZodString;
        status: z.ZodString;
        provider: z.ZodOptional<z.ZodString>;
        runtime: z.ZodOptional<z.ZodString>;
        processId: z.ZodOptional<z.ZodString>;
        outputBytes: z.ZodOptional<z.ZodNumber>;
        errorKind: z.ZodOptional<z.ZodString>;
        updatedAt: z.ZodString;
    }, z.core.$strip>>;
    preview: z.ZodOptional<z.ZodObject<{
        timelineMessageId: z.ZodOptional<z.ZodString>;
        invocationId: z.ZodString;
        turnRunId: z.ZodOptional<z.ZodString>;
        threadId: z.ZodOptional<z.ZodString>;
        capabilityId: z.ZodString;
        status: z.ZodString;
        title: z.ZodString;
        subtitle: z.ZodOptional<z.ZodString>;
        inputSummary: z.ZodOptional<z.ZodString>;
        outputSummary: z.ZodOptional<z.ZodString>;
        outputPreview: z.ZodOptional<z.ZodString>;
        outputKind: z.ZodOptional<z.ZodString>;
        outputBytes: z.ZodOptional<z.ZodNumber>;
        resultRef: z.ZodOptional<z.ZodString>;
        truncated: z.ZodBoolean;
        updatedAt: z.ZodString;
    }, z.core.$strip>>;
    reply: z.ZodOptional<z.ZodObject<{
        text: z.ZodString;
        turnRunId: z.ZodString;
        generatedAt: z.ZodString;
    }, z.core.$strip>>;
    prompt: z.ZodOptional<z.ZodObject<{
        turnRunId: z.ZodString;
        gateRef: z.ZodString;
        headline: z.ZodString;
        body: z.ZodString;
        allowAlways: z.ZodOptional<z.ZodBoolean>;
        approvalContext: z.ZodOptional<z.ZodObject<{
            toolName: z.ZodString;
            action: z.ZodString;
            scope: z.ZodString;
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
        runId: z.ZodString;
        status: z.ZodString;
        eventCursor: z.ZodNumber;
        alreadyTerminal: z.ZodBoolean;
    }, z.core.$strip>>;
    runState: z.ZodOptional<z.ZodObject<{
        turnId: z.ZodString;
        runId: z.ZodString;
        status: z.ZodString;
        eventCursor: z.ZodNumber;
        acceptedMessageRef: z.ZodString;
        resolvedRunProfileId: z.ZodString;
        resolvedRunProfileVersion: z.ZodNumber;
        receivedAt: z.ZodString;
        checkpointId: z.ZodOptional<z.ZodString>;
        gateRef: z.ZodOptional<z.ZodString>;
        failure: z.ZodOptional<z.ZodUnknown>;
    }, z.core.$strip>>;
    state: z.ZodOptional<z.ZodObject<{
        threadId: z.ZodString;
        items: z.ZodArray<z.ZodUnknown>;
    }, z.core.$strip>>;
}, z.core.$strip>;
export declare const AuthProviderSchema: z.ZodObject<{
    id: z.ZodString;
    name: z.ZodString;
    type: z.ZodString;
}, z.core.$strip>;
export declare const AutomationSourceSchema: z.ZodObject<{
    type: z.ZodLiteral<"schedule">;
    cron: z.ZodString;
    timezone: z.ZodString;
}, z.core.$strip>;
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
    source: z.ZodObject<{
        type: z.ZodLiteral<"schedule">;
        cron: z.ZodString;
        timezone: z.ZodString;
    }, z.core.$strip>;
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
        create: import("@orpc/contract").ContractProcedure<import("@orpc/contract").Schema<unknown, unknown>, z.ZodObject<{
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
        }, z.core.$strip>, z.ZodObject<{
            outcome: z.ZodString;
            threadId: z.ZodString;
            acceptedMessageRef: z.ZodString;
            runId: z.ZodOptional<z.ZodString>;
            activeRunId: z.ZodOptional<z.ZodString>;
            turnId: z.ZodOptional<z.ZodString>;
            status: z.ZodString;
            resolvedRunProfileId: z.ZodOptional<z.ZodString>;
            resolvedRunProfileVersion: z.ZodOptional<z.ZodNumber>;
            eventCursor: z.ZodOptional<z.ZodNumber>;
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
            gateRef: z.ZodString;
            resolution: z.ZodEnum<{
                approved: "approved";
                denied: "denied";
                credential_provided: "credential_provided";
                cancelled: "cancelled";
            }>;
            always: z.ZodOptional<z.ZodBoolean>;
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
                outcome: string;
                threadId: string;
                acceptedMessageRef: string;
                status: string;
                runId?: string | undefined;
                activeRunId?: string | undefined;
                turnId?: string | undefined;
                eventCursor?: number | undefined;
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
                updatedAt: string;
                turnRunId?: string | undefined;
                threadId?: string | undefined;
                provider?: string | undefined;
                runtime?: string | undefined;
                processId?: string | undefined;
                outputBytes?: number | undefined;
                errorKind?: string | undefined;
            } | undefined;
            preview?: {
                invocationId: string;
                capabilityId: string;
                status: string;
                title: string;
                truncated: boolean;
                updatedAt: string;
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
                    action: string;
                    scope: string;
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
                eventCursor: number;
                alreadyTerminal: boolean;
            } | undefined;
            runState?: {
                turnId: string;
                runId: string;
                status: string;
                eventCursor: number;
                acceptedMessageRef: string;
                resolvedRunProfileId: string;
                resolvedRunProfileVersion: number;
                receivedAt: string;
                checkpointId?: string | undefined;
                gateRef?: string | undefined;
                failure?: unknown;
            } | undefined;
            state?: {
                threadId: string;
                items: unknown[];
            } | undefined;
        }, unknown, void>, import("@orpc/shared").AsyncIteratorClass<{
            type: "cancelled" | "accepted" | "running" | "capability_progress" | "capability_activity" | "capability_display_preview" | "gate" | "auth_required" | "final_reply" | "failed" | "projection_snapshot" | "projection_update" | "keep_alive";
            cursor?: string | undefined;
            ack?: {
                outcome: string;
                threadId: string;
                acceptedMessageRef: string;
                status: string;
                runId?: string | undefined;
                activeRunId?: string | undefined;
                turnId?: string | undefined;
                eventCursor?: number | undefined;
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
                updatedAt: string;
                turnRunId?: string | undefined;
                threadId?: string | undefined;
                provider?: string | undefined;
                runtime?: string | undefined;
                processId?: string | undefined;
                outputBytes?: number | undefined;
                errorKind?: string | undefined;
            } | undefined;
            preview?: {
                invocationId: string;
                capabilityId: string;
                status: string;
                title: string;
                truncated: boolean;
                updatedAt: string;
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
                    action: string;
                    scope: string;
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
                eventCursor: number;
                alreadyTerminal: boolean;
            } | undefined;
            runState?: {
                turnId: string;
                runId: string;
                status: string;
                eventCursor: number;
                acceptedMessageRef: string;
                resolvedRunProfileId: string;
                resolvedRunProfileVersion: number;
                receivedAt: string;
                checkpointId?: string | undefined;
                gateRef?: string | undefined;
                failure?: unknown;
            } | undefined;
            state?: {
                threadId: string;
                items: unknown[];
            } | undefined;
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
    };
    automations: {
        list: import("@orpc/contract").ContractProcedure<z.ZodObject<{
            limit: z.ZodOptional<z.ZodNumber>;
            runLimit: z.ZodOptional<z.ZodNumber>;
        }, z.core.$strip>, z.ZodObject<{
            data: z.ZodArray<z.ZodObject<{
                id: z.ZodString;
                name: z.ZodString;
                source: z.ZodObject<{
                    type: z.ZodLiteral<"schedule">;
                    cron: z.ZodString;
                    timezone: z.ZodString;
                }, z.core.$strip>;
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
    };
};
export type ContractType = typeof contract;
