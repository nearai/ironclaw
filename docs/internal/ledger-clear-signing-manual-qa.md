# Ledger clear signing — manual QA checklist

Required by the attested-signing plan §D5: **no real device runs in CI**, so
real-device coverage is this checklist, executed by a human and recorded in the
PR that ships the DMK ceremony.

This exists because of a specific gap. The automated suites prove the *server's*
half is sound — that a signature verifies only against the transaction we bound,
that a missing descriptor blocks signing, that the hash reaches the DOM
complete. None of them can prove the thing the whole feature rests on: **that
the device's screen shows the human what IronClaw actually asked for.** A
scripted transport returns whatever it was scripted to return. Only hardware
answers this.

Treat an unchecked box as a blocker, not a nice-to-have.

## Preconditions

- [ ] Real Ledger device (Nano S Plus / Nano X / Stax / Flex — record which).
- [ ] Ethereum app version recorded.
- [ ] Firmware version recorded.
- [ ] Browser + version recorded (WebHID requires a secure context; Chrome/Edge).
- [ ] `ATTESTED_ALPACA_SIDECAR` and the descriptor source config recorded — a
      pass against an unconfigured descriptor source proves nothing.
- [ ] Test account is a **testnet** account with no mainnet value.

## 1. Connect

- [ ] Device locked → page reports it and does not silently hang.
- [ ] Wrong app open (e.g. Bitcoin) → page names the problem and offers no sign path.
- [ ] Ethereum app open, device unlocked → connect succeeds.
- [ ] Connection requires a user gesture (it must not auto-connect on load).
- [ ] Device disconnected mid-flow → page reports it; no partial state is left
      that a reload would treat as approved.

## 2. Clear-sign render — the load-bearing check

For each transaction below, put the browser and the device side by side and
compare **field by field**. This is the check nothing else in the system can
make for you.

- [ ] **Recipient** on the device matches the recipient on the review page.
- [ ] **Amount** matches, including decimals and token symbol.
- [ ] **Network / chain** matches.
- [ ] **Contract call** (method name and decoded arguments) matches what the
      page shows.
- [ ] **The transaction hash on the device matches the hash on the review page,
      character for character.** Read the whole value, not the ends. If these
      differ, STOP — that is the exact condition the substrate exists to catch,
      and it means something between approval and the device is rewriting the
      transaction.

Transactions to cover:

- [ ] A plain native-token transfer.
- [ ] An ERC-20 `transfer` (descriptor-covered).
- [ ] A contract call with several decoded arguments.

## 3. Reject path

- [ ] Reject on the device → the page reports the rejection.
- [ ] The gate is **not** advanced; the intent stays pending or moves to
      rejected, never approved.
- [ ] No transaction is broadcast. Confirm on a block explorer, not just in the UI.
- [ ] Retry after a rejection behaves sanely (either a clean retry or a clear
      refusal — record which).

## 4. Fail-closed, with hardware attached

- [ ] A transaction with **no** ERC-7730 descriptor → the page blocks and offers
      no sign control, *even with a device connected and unlocked*. The
      automated tests assert this without hardware; confirm the device's
      presence does not unlock a path around it.
- [ ] Descriptor service unreachable → same blocked state, not a blind-sign
      fallback.
- [ ] The device's own raw-hash / blind-signing mode is **never reachable**
      from this flow.

## 5. One-shot behaviour

- [ ] Approve once → broadcast happens once. Re-submitting the same proof is
      refused (sealed-grant CAS).
- [ ] Reload the review page after approval → shows the resolved outcome and
      offers no second signature.

## Recording the result

Paste the completed checklist into the PR with device/firmware/app/browser
versions filled in, plus the block-explorer link for each broadcast. A checklist
without those specifics is not evidence — it is a claim.

## Known deviation from the plan text

Plan §D4 step 5 specifies registering the verifier as a
`ProviderId::ledger_webhid` in the `ProviderRegistry`. **That is not
implementable as written.** `SigningProvider::verify_resume` receives the
signing context and the approved tx hash but *not* the decoded transaction, so a
provider cannot rebuild the canonical signing bytes the step tells it to
ecrecover against — only the driver holds the authoritative binding.

The verifier therefore lives driver-side, in
`ironclaw_attested_runtime::verify_device_signature`, performing the same checks
the step describes (rebuild from the binding → recover → require the gate-bound
signer → chain equality) plus the approved-hash recompute. The plan text should
be amended to match; the code should not be bent to match the plan.
