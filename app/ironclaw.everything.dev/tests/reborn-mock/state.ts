import type { RebornMockState } from "./types";

export function createDefaultState(token: string): RebornMockState {
  return {
    session: {
      tenantId: "test-tenant",
      userId: "test-user",
      capabilities: { operatorWebuiConfig: true },
    },
    token,
    threads: [],
    nextSeq: 1,
    eventCursor: 0,
    dropCount: 0,
    sseHandler: null,
    automations: [],
    outboundPrefs: {},
    outboundTargets: [],
    extensions: [],
    extensionRegistry: [],
    skills: [],
    connectableChannels: [],
    authProviders: [],
  };
}

export function cloneState(state: RebornMockState): RebornMockState {
  return JSON.parse(JSON.stringify(state));
}
