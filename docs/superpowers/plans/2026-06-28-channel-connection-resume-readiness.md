# Channel connection resume readiness

## Mission

Make the in-chat channel connection experience reliable across Slack and future inbound/external channel extensions. A user should be able to start a Slack-dependent request from chat, connect from any supported entrypoint, and have every waiting chat recover cleanly without leaking pairing codes to the model or leaving stale connection UI behind.

## Root causes confirmed

- Waiting chat state was component-local. Only mounted chat views subscribed to connection completion events, so background/sidebar threads could keep stale connection panels and never receive the continuation prompt.
- `builtin.extension_activate` used package-level knowledge to say the user's Slack account still needed pairing. After a user connected elsewhere, a later activation card could still tell the model to ask for pairing again.
- Historical activation cards were re-read from timeline on navigation. The stale-panel suppressor only checked continuation messages after a card, so a later stale card could reopen the panel even when an earlier continuation already proved connection completed.
- The extension-state recheck used cached extension data in some paths, so a just-connected Slack account could still look unpaired until cache invalidation caught up.

## Success criteria

- Chat-first Slack activation shows a local pairing panel with DM-the-bot instructions; the pairing code is redeemed through WebUI APIs and is never sent as normal chat content.
- Explicit extension activate/configure flows use the same local redemption semantics and wake waiting chats after successful connection.
- If multiple threads are waiting for the same channel, connecting from one thread resumes the other waiting threads exactly once.
- The thread that submitted the code continues its own original request exactly once and does not get a duplicate continuation from the broadcast path.
- If the user connects from the Extensions tab, every waiting thread for that channel resumes even though no chat component owns those threads.
- Navigating away from and back to a connected thread must not recreate a stale Slack connection panel from old activation cards.
- If current extension state says the channel is already authenticated, activation-card-derived onboarding is cleared instead of shown.
- Stale, expired, or wrong Slack pairing codes stay local, surface an error, keep the panel open, and do not resume chat.
- The pattern is generic for external/inbound channels, not hardcoded only to Slack, except for Slack-specific copy and pairing endpoint.
- Slack delivery-target UX remains honest: connection establishes the user/channel binding and outbound target; message-reading or arbitrary-DM claims require separate capabilities.

## Required automated coverage

- `channel-connection-events` persists waiting threads, resumes matching non-source threads, dedupes per thread, and keeps source-thread continuation owned by the submitter.
- `useChat.submitOnboardingPairing` resumes the pairing panel's thread, not merely the viewed thread, and removes waiting-thread records after success.
- Mounted waiting chats clear their panels on channel-connected events while the persisted waiting-thread registry owns continuation sends.
- Same-chat connection events do not duplicate the continuation.
- Stale activation cards are suppressed when a matching connection continuation exists anywhere in the thread timeline.
- Activation cards are also cleared by a fresh extension-state recheck when the current API says the channel is authenticated/active.
- `builtin.extension_activate` guidance for inbound/external channels is conditional and must not claim the caller is still unpaired.
- Extension search for installed external channels routes the model through activation/pairing guidance instead of treating the channel as a ready message-access tool.

## Local verification checklist

- Run targeted JS tests for channel connection events, extension onboarding, in-chat send/onboarding behavior, Slack pairing API, and Extensions pairing API.
- Run syntax checks for the touched WebUI modules.
- Run targeted Rust tests for WebUI asset embedding and extension lifecycle guidance.
- Run existing Slack host-beta/product workflow tests that cover disconnect/remove, identity cleanup, personal DM target cleanup, pairing-code expiry, and code consumption.
- Build `ironclaw-reborn` with `webui-v2-beta,slack-v2-host-beta,libsql`.
- Restart the local server on `127.0.0.1:8745` with the existing local environment, keep ngrok pointed at the same local port, and verify both local and ngrok health/API routes respond.

## Manual scenarios to re-test

- Unpaired chat-first request: ask for Slack work in a new chat, receive the local Slack connection panel, DM the Slack app, paste code, and verify the chat continues.
- Wrong code: paste an invalid code and verify the panel shows an error, remains open, and no normal chat message containing the code appears.
- Three waiting chats: open three Slack-dependent chats while unpaired, connect in one, and verify the other two receive `Slack is connected. Continue the previous request.` and no stale panels remain after navigation.
- Extension-tab connection: create one or more waiting chats, connect from the Extensions tab, and verify waiting chats resume.
- Already connected: run `/pair` in Slack after connection and verify Slack says already connected; WebChat should not ask for another code solely because an old activation card exists.
- Capability honesty: ask what Slack can do; the agent should not claim unread-message or arbitrary-person DM access unless those model-visible capabilities are actually installed.
