import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  NEAR_AI_WALLET_LOGIN_EVENT,
  NEAR_AI_WALLET_LOGIN_MESSAGE,
  NEAR_AI_WALLET_LOGIN_RECIPIENT,
  buildNearAiWalletLoginNonce,
  nearAiWalletLoginFailurePayload,
  nearAiWalletLoginSuccessPayload,
} from "./wallet-connect-core.js";

function source(relativePath) {
  return readFileSync(new URL(relativePath, import.meta.url), "utf8");
}

function assertIncludes(haystack, needles, label) {
  for (const needle of needles) {
    assert.ok(
      haystack.includes(needle),
      `${label} should include ${JSON.stringify(needle)}`
    );
  }
}

test("buildNearAiWalletLoginNonce writes epoch millis and random tail", () => {
  const randomBytes = Array.from({ length: 24 }, (_, index) => index + 1);
  const nonce = buildNearAiWalletLoginNonce({
    now: 1_717_171_717_171,
    getRandomValues(target) {
      target.set(randomBytes);
      return target;
    },
  });

  assert.equal(nonce.length, 32);
  assert.equal(new DataView(nonce.buffer).getBigUint64(0, false), 1717171717171n);
  assert.deepEqual(Array.from(nonce.slice(8)), randomBytes);
});

test("nearAi wallet payloads preserve NEAR AI message and plain nonce array", () => {
  assert.equal(NEAR_AI_WALLET_LOGIN_MESSAGE, "Sign in to NEAR AI Cloud");
  assert.equal(NEAR_AI_WALLET_LOGIN_RECIPIENT, "cloud.near.ai");
  assert.equal(NEAR_AI_WALLET_LOGIN_EVENT, "nearai-wallet-login");

  const payload = nearAiWalletLoginSuccessPayload(
    {
      accountId: "alice.near",
      publicKey: "ed25519:test",
      signature: "sig",
    },
    Uint8Array.from([1, 2, 3])
  );
  assert.deepEqual(payload, {
    type: "nearai-wallet-login",
    ok: true,
    accountId: "alice.near",
    publicKey: "ed25519:test",
    signature: "sig",
    message: "Sign in to NEAR AI Cloud",
    recipient: "cloud.near.ai",
    nonce: [1, 2, 3],
  });
  assert.deepEqual(nearAiWalletLoginFailurePayload(), {
    type: "nearai-wallet-login",
    ok: false,
  });
});

test("wallet popup source keeps isolated channel and wallet connector contracts", () => {
  const popup = source("../wallet-connect.js");
  const html = source("../../wallet-connect.html");

  assertIncludes(
    popup,
    [
      'import { NearConnector } from "@hot-labs/near-connect";',
      'from "./lib/wallet-connect-core.js";',
      'new URLSearchParams(window.location.search).get("channel")',
      'typeof BroadcastChannel !== "function"',
      'new BroadcastChannel(channelName)',
      'network: "mainnet"',
      "features: { signMessage: true }",
      "await connector.connect()",
      "await connector.wallet()",
      "buildNearAiWalletLoginNonce()",
      "message: MESSAGE",
      "recipient: RECIPIENT",
      "nearAiWalletLoginSuccessPayload(signed, nonce)",
      "nearAiWalletLoginFailurePayload()",
      "window.close()",
    ],
    "wallet-connect.js"
  );

  assertIncludes(
    html,
    [
      "@hot-labs/near-connect",
      "https://esm.sh/@hot-labs/near-connect",
      "BroadcastChannel",
      'src="/v2/js/wallet-connect.js"',
      "Connect your NEAR wallet",
    ],
    "wallet-connect.html"
  );
});

test("authenticated app relays wallet payload to protected NEAR AI route", () => {
  const hook = source("../pages/settings/hooks/useProviderLogin.js");
  const api = source("../pages/settings/lib/settings-api.js");

  assertIncludes(
    hook,
    [
      "function walletLoginChannelName()",
      "`nearai-wallet-login:${suffix}`",
      "awaitWalletSignature(popup, channelName)",
      "`/v2/wallet/connect?channel=${encodeURIComponent(channelName)}`",
      "popup.opener = null",
      "account_id: signed.accountId",
      "public_key: signed.publicKey",
      "signature: signed.signature",
      "message: signed.message",
      "recipient: signed.recipient",
      "nonce: signed.nonce",
    ],
    "useProviderLogin"
  );

  assertIncludes(
    api,
    [
      "export function completeNearaiWalletLogin",
      '"/api/webchat/v2/llm/nearai/wallet"',
      'method: "POST"',
    ],
    "settings-api"
  );
});
