import assert from "node:assert/strict";
import test from "node:test";
import nacl from "tweetnacl";
import bs58 from "bs58";
import { secretKeyFromNearPrivateKey } from "../src/lib.mjs";

test("secretKeyFromNearPrivateKey returns 64-byte nacl.sign secret (OpenClaw parity)", () => {
  const seed = nacl.sign.keyPair();
  const nearPrivateKey = `ed25519:${bs58.encode(seed.secretKey)}`;
  const signing = secretKeyFromNearPrivateKey(nearPrivateKey);
  assert.equal(signing.length, 64);
  const msg = new TextEncoder().encode("identyclaw-login");
  const sig = nacl.sign.detached(msg, signing);
  assert.equal(sig.length, 64);
  assert.equal(nacl.sign.detached.verify(msg, sig, seed.publicKey), true);
});

test("secretKeyFromNearPrivateKey expands 32-byte seed keys", () => {
  const seed = crypto.getRandomValues(new Uint8Array(32));
  const pair = nacl.sign.keyPair.fromSeed(seed);
  const nearPrivateKey = `ed25519:${bs58.encode(seed)}`;
  const signing = secretKeyFromNearPrivateKey(nearPrivateKey);
  assert.equal(signing.length, 64);
  assert.deepEqual(Buffer.from(signing), Buffer.from(pair.secretKey));
});
