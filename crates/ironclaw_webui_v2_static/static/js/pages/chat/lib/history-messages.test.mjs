import assert from "node:assert/strict";
import test from "node:test";

import { messagesFromTimeline } from "./history-messages.js";

test("messagesFromTimeline: finalized assistant records are marked as final replies", () => {
  const messages = messagesFromTimeline([
    {
      message_id: "final",
      kind: "assistant",
      status: "finalized",
      content: "Done.",
    },
    {
      message_id: "draft",
      kind: "assistant",
      status: "draft",
      content: "I will check.",
    },
  ]);

  assert.equal(messages[0].id, "msg-final");
  assert.equal(messages[0].isFinalReply, true);
  assert.equal(messages[1].id, "msg-draft");
  assert.equal(messages[1].isFinalReply, false);
});
