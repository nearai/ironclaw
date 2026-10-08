# Sendblue iMessage / SMS

Connect an assigned Sendblue line to IronClaw for direct text conversations.
The extension includes phone pairing, inbound webhooks, replies, and stored DM
destinations. Sendblue selects iMessage or SMS according to the recipient.

## Start from a fresh account

1. Build/install an IronClaw version containing this extension. Run
   `ironclaw onboard` to configure model access, the encrypted credential store,
   and WebUI. Use `ironclaw status` to open the login link. Model access is
   separate from a Sendblue messaging account.
2. Run `npx --yes @sendblue/cli@0.10.0 setup --phone +YOUR_PERSONAL_NUMBER`.
   Follow the verification challenge: the human sends the requested text from
   their own phone. Run `npx --yes @sendblue/cli@0.10.0 setup --check` until it
   completes (exit 3 means pending). The resulting
   `~/.sendblue/credentials.json` contains `apiKey`, `apiSecret`, and
   `assignedNumber`. Keep credentials out of chat, logs, and Git.
3. For an additional free-plan recipient, run
   `npx --yes @sendblue/cli@0.10.0 add-contact +RECIPIENT` and have that person
   text the assigned line once. Local allowlisting does not complete Sendblue's
   contact verification.
4. In IronClaw's extension configuration, fill the **Sendblue messaging
   configuration** form:

   | Field | Value |
   |---|---|
   | API key ID | `apiKey` |
   | API secret key | `apiSecret` |
   | Webhook shared secret | A fresh random value, e.g. `openssl rand -hex 32` |
   | Assigned Sendblue line | `assignedNumber`, not your personal phone |
   | Allowed sender phones | Comma-separated personal `+E164` numbers |

   All three credential fields are encrypted host secrets. Only the HTTP host
   injects the two API headers; the adapter and model never receive their values.
   An empty sender list admits nobody. `*` explicitly allows all senders, with
   IronClaw phone pairing still required.
5. Install/activate the Sendblue extension. Expose only its webhook route via a
   public HTTPS reverse proxy or tunnel:
   `https://YOUR_HOST/webhooks/extensions/sendblue/messages`.
   Keep the rest of the host behind its normal authentication.
6. In Sendblue, add a receive webhook using that URL, the same webhook secret,
   and the assigned line. The API equivalent is **POST**
   `https://api.sendblue.com/api/account/webhooks`, with `sb-api-key-id` and
   `sb-api-secret-key` headers and the following JSON. Use a private request
   file/dashboard for the secret rather than literal shell history.

   ```json
   {"webhooks":{"receive":[{"url":"https://YOUR_HOST/webhooks/extensions/sendblue/messages","secret":"YOUR_WEBHOOK_SECRET","sendblue_numbers":["+ASSIGNED_LINE"]}]}}
   ```

   POST appends registrations. Inspect existing registrations before repeating
   setup to avoid duplicates. IronClaw does not replace your account's other
   webhooks. Sendblue authenticates callbacks with a shared `sb-signing-secret`
   header; this is not an HMAC signature.
7. From Sendblue's connection card in IronClaw, generate a pairing code. Text
   that code **from the allowed, verified personal phone to the assigned line**.
   Wait for the pairing confirmation. This binds the phone to your existing
   IronClaw user; webhook delivery alone grants no user authority.
8. Text `Remember cobalt`, wait for the answer on your handset, then ask
   `What word did I ask you to remember?`. Verify the second reply and the
   conversation in IronClaw. An HTTP acceptance response alone does not prove
   handset delivery. Try an unallowed phone and confirm it starts no turn.

## Behavior and troubleshooting

- Direct text only. Groups and outbound/status callbacks are ignored. Inbound
  attachments become a notice asking for text; external media URLs are never
  downloaded. Final replies containing files direct the user to IronClaw.
- Replies split into 2,000 Unicode characters. No transport retry occurs after
  uncertain acceptance; partially sent or ambiguous replies keep a checkpoint
  that prevents replay. Inspect Sendblue history before manually retrying. A
  crash between provider acceptance and host persistence can still leave an
  uncertain outcome; this is not an exactly-once carrier guarantee.
- IronClaw owns durable replay protection, identity, permission gates, and
  delivery state. Generated-code pairing, local sender allowlisting, and
  Sendblue contact verification are separate checks.
- A 401 callback means the registered and configured webhook secrets differ.
  Ignored callbacks can indicate the wrong assigned line, an unallowed sender,
  a group, or an outbound/status event.
- `QUEUED` is recorded as uncertain acceptance, never confirmed delivery. It
  stops subsequent chunks and blocks automatic replay; inspect provider history
  before retrying or sending the remainder. `SENT`, `DELIVERED`, and `READ` retain
  provider delivery evidence.
- A message accepted by Sendblue can still fail downstream; consult provider
  status for carrier issues. An HTTP 200 with `ERROR` is treated as a rejection.
- Approval/authentication notices are rendered as text; follow their IronClaw
  links to resolve actions requiring the web UI.

## Remove

Deactivate/uninstall the extension, remove only this deployment's webhook in
Sendblue, and remove its public reverse-proxy route. Clear the extension's
administrator credentials if no longer needed. Keep identities and history
unless you deliberately choose to delete them through normal product controls.

## Development checks

```bash
cargo test -p ironclaw_sendblue_extension
cargo test -p ironclaw_extension_host --lib egress
RUST_MIN_STACK=8388608 cargo test -p ironclaw_integration_tests --test reborn_integration_extension_delivery sendblue_phone_pairing_and_text_reply_use_the_production_host
cargo test -p ironclaw_architecture_tests
```

The integration test uses the real host and model decorator chain with a scripted
model SDK and recorded vendor transport. It sends no real text messages. The
handset smoke test above remains a separate release check.

References: [credentials](https://docs.sendblue.com/getting-started/credentials),
[send-message](https://docs.sendblue.com/api/resources/messages/methods/send),
[webhooks](https://docs.sendblue.com/api/resources/webhooks).
