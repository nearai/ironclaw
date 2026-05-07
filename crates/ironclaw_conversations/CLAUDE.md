# ironclaw_conversations guardrails

- Own adapter-safe conversation binding and inbound-turn facade contracts only: external actor/conversation refs, source/reply binding refs, participant checks, message acceptance refs, and idempotency semantics.
- Do not parse concrete Slack/Telegram/Web/CLI payloads in this crate. Product adapters normalize protocol payloads before calling these services.
- Do not persist raw user or assistant message content in turn-facing records. Use content/message refs; durable transcript content belongs to the SessionThreadService/TranscriptStore storage boundary.
- Keep `TurnCoordinator` inputs canonical: `TurnScope`, `TurnActor`, `AcceptedMessageRef`, `SourceBindingRef`, and `ReplyTargetBindingRef`.
- Binding resolution must fail closed for unpaired actors, unknown/inaccessible threads, invalid refs, participant-policy denials, tenant/adapter-installation mismatches, and delimiter-like external IDs that could collide if flattened into strings.
- Conversation binding identity excludes per-message external IDs; bind on stable `(space_id, conversation_id, thread_id)` route identity so adapters that include message IDs do not fork canonical threads.
- Source binding and reply target binding refs are distinct. Egress paths must validate reply targets against the current actor/thread before sending to external destinations, and validation must preserve adapter kind, adapter installation, and full external route fields. Reply routes are owner-scoped unless a future API explicitly marks them shared.
- Accepted inbound messages must snapshot message-scoped reply targets and route ownership; stable conversation bindings ignore and strip per-message IDs, but egress routing for accepted messages must preserve them.
- Accepted inbound message writes must reject mixed source/reply binding refs that do not belong to the same tenant/thread binding and reject external route snapshots whose stable identity differs from the binding.
- Serde deserialization for external ref types must delegate to the same validation rules as constructors.
- Future durable binding repositories must avoid raw wide composite unique indexes for external route fields; use typed rows plus a collision-resistant digest/indirection key derived from length-prefixed components.
- Accepted message idempotency and turn-submission idempotency are separate: adapter retries must reuse the accepted message ref and canonical actor and retry submission until the message is marked submitted. Reject installation-wide external event IDs that drift across stable routes before creating a second thread. Rotate submit idempotency keys after busy/transient failures so turn-store replay cannot strand the message; keep the original submit key for permanent turn rejections.
- Preserve typed `ironclaw_turns::TurnError` values across this boundary instead of flattening turn failures to strings.
- Automatic first-contact binding must not trust raw adapter-supplied agent/project scope hints; use explicit linking or a future trusted thread-creation seam for scoped thread selection.
- Explicit links are idempotent only for the same target thread; never silently retarget an already-bound external conversation to another thread.
- Keep durable PostgreSQL/libSQL adapters out of this crate until the transcript/thread storage boundary has a scoped implementation plan with parity tests.
