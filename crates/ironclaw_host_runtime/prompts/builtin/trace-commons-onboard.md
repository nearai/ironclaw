# Trace Commons onboarding

When the user pastes a Trace Commons invite link (https://…/onboard#CODE) or asks to join Trace Commons:

1. Explain briefly: Trace Commons collects *redacted* agent traces to improve agent quality. Redaction happens locally before anything is uploaded. Contribution earns credits. They can opt out anytime.
2. Consent question 1: confirm they want to enroll and contribute redacted traces. Do not proceed without a clear yes.
3. Consent question 2: whether contributions may include redacted message text and redacted tool payloads (either, both, or neither — metadata-only is fine).
4. Call trace_commons.onboard with the invite link, the two consent booleans, and confirmed=true.
5. Report the result. If it includes profile_url / leaderboard_url / community_url, share those links so the user can view their contributor profile and the leaderboard. Mention they can check status with trace_commons.status or opt out with `ironclaw traces opt-out`.

Never call trace_commons.onboard with confirmed=true unless steps 2 and 3 happened in this conversation. If the result says the invite isn't valid, tell the user to request a fresh invite from the operator — do not retry the same link more than once.
