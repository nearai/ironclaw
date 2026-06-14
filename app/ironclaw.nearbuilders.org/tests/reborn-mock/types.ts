export type ScenarioName =
  | "healthy-empty"
  | "healthy-chat"
  | "bad-token"
  | "unreachable"
  | "stream-final-reply"
  | "stream-gate"
  | "stream-failed"
  | "stream-cancelled"
  | "stream-drop-once"
  | "stream-auth-required"
  | "stream-projection"
  | "stream-capability";

export interface SessionData {
  tenantId: string;
  userId: string;
  capabilities: { operatorWebuiConfig: boolean };
}

export interface ThreadData {
  threadId: string;
  title?: string;
  createdAt: string;
  messages: MessageData[];
}

export interface MessageData {
  messageId: string;
  threadId: string;
  sequence: number;
  kind: "User" | "Assistant" | "System";
  content: string;
  actorId?: string;
  status?: string;
  turnId?: string;
  turnRunId?: string;
  createdAt: string;
}

export interface TimelineData {
  messages: MessageData[];
  nextCursor: string | null;
}

export type SSECallback = (write: (event: string, data: unknown) => void) => Promise<void>;

export interface RebornMockState {
  session: SessionData;
  token: string;
  threads: ThreadData[];
  nextSeq: number;
  automations: Array<{ id: string; name?: string; status?: string; isActive: boolean }>;
  outboundPrefs: { finalReplyTarget?: { targetId: string; channel: string; displayName: string } };
  outboundTargets: Array<{
    target: { targetId: string; channel: string; displayName: string };
    capabilities: { finalReplies: boolean; gatePrompts: boolean; authPrompts: boolean };
  }>;
  extensions: Array<{
    packageRef: { kind: string; id: string };
    displayName: string;
    kind: string;
    description: string;
    active: boolean;
  }>;
  extensionRegistry: Array<{
    packageRef: { kind: string; id: string };
    displayName: string;
    kind: string;
    description: string;
    installed: boolean;
  }>;
  skills: Array<{
    name: string;
    description: string;
    version: string;
    trust: string;
    source: string;
  }>;
  connectableChannels: Array<{ channel: string; displayName: string; strategy: string }>;
  authProviders: Array<{ id: string; name: string; type: string }>;
  sseHandler: SSECallback | null;
  dropCount: number;
  eventCursor: number;
}

export interface RebornMockOptions {
  token?: string;
  port?: number;
  scenario?: ScenarioName;
}

export interface RebornMockHandle {
  baseUrl: string;
  token: string;
  state: RebornMockState;
  stop: () => Promise<void>;
  reset: () => void;
  setScenario: (name: ScenarioName) => void;
}
