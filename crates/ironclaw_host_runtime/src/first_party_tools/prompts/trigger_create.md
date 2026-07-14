Create a caller-scoped scheduled trigger (one-time or recurring).

The prompt is the full task each fire performs. If delivery_target_id is set, never put a send, post, or deliver-results step for that result in the prompt; each fire's final reply is delivered automatically to that target.

Do not tell the prompt to send results back to the requesting user. Asks like "send me the result" are delivery routing, not a task step: pass delivery_target_id with an id from builtin__outbound_delivery_targets_list and keep every send-to-requester step — even one with a pinned conversation id — out of the prompt.

Put messaging in the prompt only when messaging someone else is itself the task; pin that third-party recipient, resolved while the user is present. Never leave a recipient as a name to look up at fire time.

Without delivery_target_id, the user's default outbound target applies at fire time; builtin__outbound_delivery_target_set changes that user-wide default.

In the user-facing reply, call it a routine and summarize its name, task, plain-language schedule, and delivery only; never expose trigger, agent, project, or delivery-target ids, raw cron, stored prompts, internal capability names, result references, or host metadata.
