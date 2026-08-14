import fs from "node:fs";
import path from "node:path";
import readline from "node:readline";

const workspaceFile = (name) => path.join(process.cwd(), name);
const log = (event) => fs.appendFileSync(workspaceFile("fake-events.log"), `${event}\n`);
const send = (message) => process.stdout.write(`${JSON.stringify(message)}\n`);
let pendingPrompt = null;

log(`env:${Object.keys(process.env).sort().join(",")}`);

readline.createInterface({ input: process.stdin }).on("line", (line) => {
  const message = JSON.parse(line);
  if (message.method === "initialize") {
    send({ jsonrpc: "2.0", id: message.id, result: {
      protocolVersion: 1,
      agentCapabilities: { loadSession: true },
      authMethods: [],
      agentInfo: { name: "ironclaw-acp-fake", version: "1" }
    }});
  } else if (message.method === "session/new") {
    log("new:fake-session");
    log(`new-cwd:${message.params.cwd}`);
    send({ jsonrpc: "2.0", id: message.id, result: { sessionId: "fake-session" }});
  } else if (message.method === "session/load") {
    log(`load:${message.params.sessionId}`);
    log(`load-cwd:${message.params.cwd}`);
    send({ jsonrpc: "2.0", id: message.id, result: {} });
  } else if (message.method === "session/prompt") {
    if (JSON.stringify(message.params.prompt).includes("crash")) {
      log("crash");
      process.exit(17);
    }
    if (JSON.stringify(message.params.prompt).includes("hang")) {
      log("hang");
      return;
    }
    pendingPrompt = { id: message.id, sessionId: message.params.sessionId };
    send({ jsonrpc: "2.0", id: 900, method: "session/request_permission", params: {
      sessionId: pendingPrompt.sessionId,
      toolCall: { toolCallId: "fake-tool" },
      options: [
        { optionId: "reject-once", name: "Reject once", kind: "reject_once" },
        { optionId: "allow-once", name: "Allow once", kind: "allow_once" }
      ]
    }});
  } else if (message.id === 900 && pendingPrompt) {
    log(`permission:${JSON.stringify(message.result)}`);
    const countPath = workspaceFile("prompt-count");
    const count = fs.existsSync(countPath) ? Number(fs.readFileSync(countPath, "utf8")) + 1 : 1;
    fs.writeFileSync(countPath, String(count));
    send({ jsonrpc: "2.0", method: "session/update", params: {
      sessionId: pendingPrompt.sessionId,
      update: { sessionUpdate: "agent_message_chunk", content: {
        type: "text", text: `fake ACP reply ${count}`
      }}
    }});
    send({ jsonrpc: "2.0", id: pendingPrompt.id, result: { stopReason: "end_turn" }});
    pendingPrompt = null;
  }
});
