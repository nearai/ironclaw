Create a caller-scoped scheduled trigger (one-time or recurring).

The prompt is the full task each fire performs. When the task is to message someone or post somewhere, say so in the prompt and pin the exact recipient conversation ids, resolved while the user is present — never leave a recipient as a name to look up at fire time.

Do not tell the prompt to send results back to the requesting user. Each fire's final reply is delivered automatically — to this trigger's delivery_target_id when set, otherwise to the user's default outbound delivery target at fire time. Asks like "send me the result" are delivery routing, not a task step: satisfy them with delivery_target_id alone and keep every send-to-requester step — even one with a pinned conversation id — out of the prompt.

When the user asks for this trigger's results on a specific product or channel, pass delivery_target_id with an id from builtin__outbound_delivery_targets_list; builtin__outbound_delivery_target_set changes only the user-wide default shared by everything else.

In the user-facing reply, call it a routine and summarize its name, task, plain-language schedule, and delivery only. Never expose trigger, agent, project, or delivery-target ids, raw cron, stored prompts, internal capability names, result references, or host metadata.
