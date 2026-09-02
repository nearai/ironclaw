import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

type ProviderLoginHarnessOptions = {
  walletMessages: unknown[];
  onSuccess?: () => Promise<void>;
};

function runProviderLogin({ walletMessages, onSuccess }: ProviderLoginHarnessOptions) {
  const stateLog: Array<{ idx: number; value: unknown }> = [];
  const httpCalls: string[] = [];
  const walletPayloads: unknown[] = [];
  let stateIndex = 0;
  let popupClosed = false;

  class FakeBroadcastChannel {
    private listener: ((event: { data: unknown }) => void) | null = null;
    private closed = false;

    addEventListener(_event: string, listener: (event: { data: unknown }) => void) {
      this.listener = listener;
      Promise.resolve().then(() => {
        for (const data of walletMessages) {
          if (this.closed || !this.listener) return;
          this.listener({ data });
        }
      });
    }

    removeEventListener() {
      this.listener = null;
    }

    close() {
      this.closed = true;
    }
  }

  const context = {
    console,
    Date,
    Math,
    Promise,
    isLoopbackBrowserOrigin: () => false,
    setTimeout: () => 0,
    clearTimeout: () => {},
    setInterval: () => 0,
    clearInterval: () => {},
    React: {
      useState(init: unknown) {
        const idx = stateIndex++;
        return [init, (value: unknown) => stateLog.push({ idx, value })];
      },
      useCallback: <T>(fn: T) => fn,
    },
    useT: () => (key: string) => key,
    useQueryClient: () => ({ invalidateQueries: async () => {} }),
    startNearaiLogin: async () => ({ auth_url: "https://auth.example" }),
    completeNearaiWalletLogin: async (payload: unknown) => {
      httpCalls.push("completeNearaiWalletLogin");
      walletPayloads.push(payload);
      return {};
    },
    fetchLlmProviders: async () => ({ active: null }),
    startCodexLogin: async () => ({
      user_code: "code",
      verification_uri: "https://verify.example",
    }),
    window: {
      location: { hostname: "app.example.com", origin: "https://app.example.com" },
      open: () => ({
        opener: null,
        closed: false,
        close() {
          popupClosed = true;
        },
      }),
      crypto: { randomUUID: () => "uuid" },
      BroadcastChannel: FakeBroadcastChannel,
    },
  };
  const exports = runVmModuleForTest(
    "./useProviderLogin.ts",
    ["useProviderLogin"],
    context,
    import.meta.url,
  );
  const NEARAI_BUSY_SLOT = 0;
  const NEARAI_ERROR_SLOT = 1;
  return {
    hook: exports.useProviderLogin({ onSuccess }),
    httpCalls,
    walletPayloads,
    nearaiErrors: () =>
      stateLog.filter((entry) => entry.idx === NEARAI_ERROR_SLOT).map((entry) => entry.value),
    nearaiBusyCleared: () =>
      stateLog.some((entry) => entry.idx === NEARAI_BUSY_SLOT && entry.value === false),
    popupClosed: () => popupClosed,
  };
}

test("wallet cancellation settles the production login flow without waiting for timeout", async () => {
  const run = runProviderLogin({
    walletMessages: [{ type: "nearai-wallet-login", ok: false }],
  });

  const completed = await Promise.race([
    run.hook.startNearaiWallet().then(() => true),
    new Promise<boolean>((resolve) => setImmediate(() => resolve(false))),
  ]);

  assert.equal(completed, true);
  assert.ok(run.nearaiErrors().includes("onboarding.nearaiFailed"));
  assert.ok(run.nearaiBusyCleared());
  assert.ok(run.popupClosed());
  assert.ok(!run.httpCalls.includes("completeNearaiWalletLogin"));
});

test("wallet login ignores a malformed nonce and accepts the next valid message", async () => {
  const signature = {
    type: "nearai-wallet-login",
    ok: true,
    accountId: "alice.near",
    publicKey: "ed25519:key",
    signature: "signature",
    message: "message",
    recipient: "ai.near",
  };
  const run = runProviderLogin({
    walletMessages: [
      { ...signature, nonce: [1, "2", 3] },
      { ...signature, nonce: [1, 2, 3] },
    ],
  });

  await run.hook.startNearaiWallet();

  assert.equal(
    run.httpCalls.filter((call) => call === "completeNearaiWalletLogin").length,
    1,
  );
  assert.deepEqual(
    (run.walletPayloads[0] as { nonce: unknown }).nonce,
    [1, 2, 3],
  );
  assert.deepEqual(run.nearaiErrors(), [""]);
});

test("wallet login reports a rejected async success callback", async () => {
  const run = runProviderLogin({
    walletMessages: [
      {
        type: "nearai-wallet-login",
        ok: true,
        accountId: "alice.near",
        publicKey: "ed25519:key",
        signature: "signature",
        message: "message",
        recipient: "ai.near",
        nonce: [1, 2, 3],
      },
    ],
    onSuccess: async () => {
      throw new Error("navigation failed");
    },
  });

  await run.hook.startNearaiWallet();

  assert.ok(run.nearaiErrors().includes("onboarding.nearaiFailed"));
  assert.ok(run.nearaiBusyCleared());
});
