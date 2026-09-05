/**
 * Worker conformance tests: a scripted fake host drives the real worker over
 * in-process framed streams — bootstrap, prompt build + resolve, a scripted
 * tool-call model response, capability invocation, final model response,
 * Outcome + OutcomeAck. Plus framing/RPC edge cases.
 */

import { describe, expect, test } from "bun:test";

import { FrameReader, encodeFrame } from "../src/framing.ts";
import { runPiWorker } from "../src/pi-adapter.ts";
import { HostRpc } from "../src/host.ts";
import {
  LOOP_WORKER_MAX_FRAME_BYTES,
  type LoopWorkerBootstrap,
  type LoopModelResponse,
  type LoopPromptBundle,
  type WireResolvedModelMessage,
} from "../src/protocol.ts";

const RUN_ID = "01950000-0000-7000-8000-000000000001";
const MESSAGE_REF = "msg:seed-1";
const CAPABILITY_ID = "builtin.echo";
const TOOL_NAME = "echo";
const TOOL_RESULT_REF = "result:run-1:abc";

function resolvedBootstrap(): LoopWorkerBootstrap {
  return {
    wire_version: 2,
    run_context: {
      scope: {},
      thread_id: "01950000-0000-7000-8000-000000000002",
      turn_id: "01950000-0000-7000-8000-000000000003",
      run_id: RUN_ID,
      resolved_run_profile: {
        resource_budget_policy: {
          max_model_calls: 100,
          max_capability_invocations: 100,
        },
      },
      loop_driver_id: "loop_driver_id:planned",
      loop_driver_version: 1,
      checkpoint_schema_id: "checkpoint_schema_id:planned",
      checkpoint_schema_version: 1,
    },
    invocation: {
      Run: {
        turn_id: "01950000-0000-7000-8000-000000000003",
        run_id: RUN_ID,
        resolved_run_profile: {},
      },
    },
    settings: { default_iteration_limit: 8, model_availability_attempts: null },
    tool_definitions: [
      {
        capability_id: CAPABILITY_ID,
        name: TOOL_NAME,
        description: "Echo the provided text",
        parameters: {
          type: "object",
          properties: { text: { type: "string" } },
          required: ["text"],
        },
      },
    ],
    current_visible_capabilities: null,
    content_visibility: "resolved",
  };
}

function promptBundle(): LoopPromptBundle {
  return {
    bundle_ref: "prompt:run-1:token",
    messages: [{ role: "user", content_ref: MESSAGE_REF }],
    surface_version: "surface:1",
    identity_message_count: 0,
    instruction_snippet_count: 0,
  };
}

function resolvedMessages(): WireResolvedModelMessage[] {
  return [
    {
      role: "user",
      content_ref: MESSAGE_REF,
      content: "Say hi then call echo.",
    },
  ];
}

/** StreamModel reply #1: assistant text + a tool call. */
function toolCallModelResponse(): LoopModelResponse {
  return {
    chunks: [{ safe_text_delta: "Calling the tool. " }],
    output: {
      capability_calls: [
        {
          activity_id: "01950000-0000-7000-8000-00000000000a",
          surface_version: "surface:1",
          capability_id: CAPABILITY_ID,
          input_ref: "input:call-1",
          provider_replay: {
            provider_id: "ironclaw",
            provider_model_id: "ironclaw-host-gateway",
            provider_turn_id: RUN_ID,
            provider_call_id: "call-1",
            provider_tool_name: TOOL_NAME,
            arguments: { text: "hello" },
          },
        },
      ],
    },
    effective_model_profile_id: "model_profile_id:default",
    usage: { input_tokens: 10, output_tokens: 5 },
  };
}

/** StreamModel reply #2: final assistant reply. */
function finalModelResponse(): LoopModelResponse {
  return {
    chunks: [{ safe_text_delta: "Done: echo returned hello" }],
    output: { assistant_reply: { content: "Done: echo returned hello" } },
    effective_model_profile_id: "model_profile_id:default",
    usage: { input_tokens: 20, output_tokens: 8 },
  };
}

function registerResponse(): unknown {
  return {
    activity_id: "01950000-0000-7000-8000-00000000000a",
    surface_version: "surface:1",
    capability_id: CAPABILITY_ID,
    input_ref: "input:call-1",
    provider_replay: {
      provider_id: "ironclaw",
      provider_model_id: "ironclaw-host-gateway",
      provider_turn_id: RUN_ID,
      provider_call_id: "call-1",
      provider_tool_name: TOOL_NAME,
      arguments: { text: "hello" },
    },
  };
}

function invokeResponse(): unknown {
  return {
    done: {
      refs: {
        result: "stored-result-1",
        byte_len: 16,
        origin: TOOL_RESULT_REF,
      },
      verdict: "success",
      summary: "echo completed",
    },
  };
}

/**
 * The fake host answers the worker's `ResolveMessages` calls. The initial
 * resolve carries the prompt-bundle ref; later ones carry the tool result ref.
 */
function resolveReplyFor(
  host: TestHost,
): (call: Record<string, unknown>) => unknown {
  return (call) => {
    const messages = (
      call as { ResolveMessages?: { messages: { content_ref: string }[] } }
    ).ResolveMessages!.messages;
    return messages.map((message) =>
      message.content_ref === MESSAGE_REF
        ? {
            role: "user",
            content_ref: MESSAGE_REF,
            content: "Say hi then call echo.",
          }
        : {
            role: "tool_result_reference",
            content_ref: message.content_ref,
            content: "",
            tool_result: {
              provider_call_id: "call-1",
              content: "echo says hello",
            },
          },
    );
  };
}

/** The scripted worker runner used by every test (mirrors main.ts). */
interface WorkerRun {
  outcome: Promise<unknown>;
  finished: Promise<void>;
  exitCode: Promise<number>;
}

function runWorkerOn(
  stdin: ReadableStream<Uint8Array>,
  stdout: Bun.FileSink,
  bootstrap: LoopWorkerBootstrap,
  host: TestHost,
  options: { cancelAfterFirstCall?: () => void } = {},
): WorkerRun {
  const exitResolver = Promise.withResolvers<number>();
  const run = (async () => {
    const rpc = new HostRpc({
      stdin,
      stdout,
      onEarlyEof: () => exitResolver.resolve(1),
    });
    if (options.cancelAfterFirstCall) {
      options.cancelAfterFirstCall();
    }
    try {
      const outcome = await runPiWorker(bootstrap, rpc);
      await rpc.sendOutcome(outcome);
      exitResolver.resolve(0);
      return outcome;
    } catch {
      exitResolver.resolve(1);
      return undefined;
    }
  })();
  return {
    outcome: run,
    finished: exitResolver.promise.then(() => undefined),
    exitCode: exitResolver.promise,
  };
}

type Reply = {
  matches: (call: Record<string, unknown>) => boolean;
  reply: unknown;
};

/** Minimal typed view of the fake host handle. */
interface TestHost {
  workerFrames: {
    HostRequest?: { id: number; call: Record<string, unknown> };
    Outcome?: unknown;
  }[];
  outcomeSeen: Promise<unknown>;
  sendHostFrame: (frame: unknown) => void;
  closeHostToWorker: () => void;
  done: Promise<void>;
}

import { startFakeHost } from "./fake-host.ts";

describe("loop worker over the membrane", () => {
  test("happy path: bootstrap -> prompt -> tool round -> final reply -> Exit(Completed)", async () => {
    let streamModelCalls = 0;
    let hostHandle: TestHost | null = null;
    const replies: Reply[] = [
      { matches: (call) => "BuildPrompt" in call, reply: promptBundle() },
      {
        matches: (call) => "ResolveMessages" in call,
        reply: (call) => resolveReplyFor(hostHandle as TestHost)(call),
      },
      {
        matches: (call) => "StreamModel" in call,
        reply: () => {
          streamModelCalls += 1;
          return streamModelCalls === 1
            ? toolCallModelResponse()
            : finalModelResponse();
        },
      },
      {
        matches: (call) => "RegisterProviderToolCall" in call,
        reply: registerResponse(),
      },
      {
        matches: (call) => "InvokeCapability" in call,
        reply: invokeResponse(),
      },
      {
        matches: (call) => "AppendCapabilityResultRef" in call,
        reply: "msg:tool-result-1",
      },
      {
        matches: (call) => "FinalizeAssistantMessage" in call,
        reply: "msg:final-1",
      },
      // Checkpoint staging + progress calls accept an opaque ref.
      {
        matches: (call) => "StageCheckpointPayload" in call,
        reply: "staged:1",
      },
      {
        matches: (call) => "Checkpoint" in call,
        reply: "01950000-0000-7000-8000-000000000004",
      },
    ];

    const handle = startFakeHost(
      { bootstrap: resolvedBootstrap(), replies },
      async (stdin, stdout) => {
        const host = new HostRpc({ stdin, stdout });
        const outcome = await runPiWorker(resolvedBootstrap(), host);
        await host.sendOutcome(outcome);
      },
    );
    hostHandle = handle as unknown as TestHost;

    const outcome = (await handle.outcomeSeen) as {
      Exit?: { completed?: Record<string, unknown> };
      Failed?: { kind: string };
    };
    expect(outcome.Exit?.completed).toBeDefined();
    expect(outcome.Exit!.completed!.reply_message_refs).toEqual([
      "msg:final-1",
    ]);
    expect(outcome.Exit!.completed!.completion_kind).toBe("final_reply");
    expect(outcome.Exit!.completed!.model_usage).toMatchObject({
      input_tokens: 30,
      output_tokens: 13,
    });

    // The worker invokes the original host-issued candidate, without registering a second identity.
    const calls = handle.workerFrames.flatMap((frame) =>
      "HostRequest" in frame ? [frame.HostRequest.call] : [],
    );
    expect(calls.filter((call) => "RegisterProviderToolCall" in call)).toEqual(
      [],
    );
    expect(
      calls.flatMap((call) =>
        "InvokeCapability" in call ? [call.InvokeCapability] : [],
      )[0],
    ).toMatchObject({
      activity_id: "01950000-0000-7000-8000-00000000000a",
      input_ref: "input:call-1",
    });
    expect(calls.some((call) => "InvokeCapability" in call)).toBe(true);
    // Tool result ref was resolved back into text.
    expect(
      calls.some(
        (call) =>
          "ResolveMessages" in call &&
          (
            call as { ResolveMessages: { messages: { content_ref: string }[] } }
          ).ResolveMessages.messages.some(
            (message) => message.content_ref === "msg:tool-result-1",
          ),
      ),
    ).toBe(true);
    // Two model calls: the tool round then the final reply.
    expect(calls.filter((call) => "StreamModel" in call)).toHaveLength(2);
  }, 20000);

  test("approval parks a batch, commits its payload, and resumes without another model decision", async () => {
    const bootstrap = resolvedBootstrap();
    let saved: number[] = [];
    let resumed = false;
    let modelCalls = 0;
    const invocations: Record<string, unknown>[] = [];
    const replies: Reply[] = [
      { matches: (call) => "BuildPrompt" in call, reply: promptBundle() },
      {
        matches: (call) => "ResolveMessages" in call,
        reply: resolveReplyFor({} as TestHost),
      },
      {
        matches: (call) => "StreamModel" in call,
        reply: () => {
          modelCalls += 1;
          if (resumed) return finalModelResponse();
          const response = toolCallModelResponse();
          if (!("capability_calls" in response.output))
            throw new Error("invalid fixture");
          const first = response.output.capability_calls[0];
          response.output.capability_calls.push({
            ...first,
            activity_id: "01950000-0000-7000-8000-00000000000b",
            input_ref: "input:call-2",
            provider_replay: {
              ...first.provider_replay!,
              provider_call_id: "call-2",
            },
          });
          return response;
        },
      },
      {
        matches: (call) => "InvokeCapability" in call,
        reply: (call: Record<string, unknown>) => {
          invocations.push(call.InvokeCapability as Record<string, unknown>);
          return resumed
            ? invokeResponse()
            : {
                blocked: {
                  approval: {
                    gate: "01950000-0000-7000-8000-000000000005",
                    origin:
                      "gate:approval-01950000-0000-7000-8000-000000000005",
                    resume: "resume-token:approval-1",
                  },
                },
              };
        },
      },
      {
        matches: (call) => "StageCheckpointPayload" in call,
        reply: (call: Record<string, unknown>) => {
          const stage = call.StageCheckpointPayload as {
            kind: string;
            payload: number[];
          };
          if (stage.kind === "before_block") saved = stage.payload;
          return "staged:1";
        },
      },
      {
        matches: (call) => "Checkpoint" in call,
        reply: "01950000-0000-7000-8000-000000000004",
      },
      {
        matches: (call) => "LoadCheckpointPayload" in call,
        reply: () => ({
          schema_id: bootstrap.run_context.checkpoint_schema_id,
          schema_version: 1,
          payload: saved,
        }),
      },
      {
        matches: (call) => "AppendCapabilityResultRef" in call,
        reply: "msg:tool-result-1",
      },
      {
        matches: (call) => "FinalizeAssistantMessage" in call,
        reply: "msg:final-1",
      },
    ];
    const first = startFakeHost(
      { bootstrap, replies },
      async (stdin, stdout) => {
        const host = new HostRpc({ stdin, stdout });
        await host.sendOutcome(await runPiWorker(bootstrap, host));
      },
    );
    expect(await first.outcomeSeen).toMatchObject({
      Exit: {
        blocked: {
          kind: "approval",
          checkpoint_id: "01950000-0000-7000-8000-000000000004",
          gate_ref: "gate:approval-01950000-0000-7000-8000-000000000005",
        },
      },
    });
    expect(invocations).toHaveLength(1);
    const session = JSON.parse(new TextDecoder().decode(new Uint8Array(saved)));
    expect(
      session.pending.map(
        (entry: { candidate: { input_ref: string } }) =>
          entry.candidate.input_ref,
      ),
    ).toEqual(["input:call-1", "input:call-2"]);
    resumed = true;
    const resume = {
      ...bootstrap,
      invocation: {
        Resume: {
          run_id: RUN_ID,
          turn_id: bootstrap.run_context.turn_id,
          checkpoint_id: "01950000-0000-7000-8000-000000000004",
          resolved_run_profile: {},
        },
      },
    };
    const second = startFakeHost(
      { bootstrap: resume, replies },
      async (stdin, stdout) => {
        const host = new HostRpc({ stdin, stdout });
        await host.sendOutcome(await runPiWorker(resume, host));
      },
    );
    expect(await second.outcomeSeen).toMatchObject({
      Exit: {
        completed: {
          reply_message_refs: ["msg:final-1"],
          model_usage: { input_tokens: 30, output_tokens: 13 },
        },
      },
    });
    expect(modelCalls).toBe(2);
    expect(invocations.map((call) => call.input_ref)).toEqual([
      "input:call-1",
      "input:call-1",
      "input:call-2",
    ]);
    expect(invocations[1].approval_resume).toMatchObject({
      approval_request_id: "01950000-0000-7000-8000-000000000005",
      resume_token: "resume-token:approval-1",
      input_ref: "input:call-1",
    });
  });

  test("blind bootstrap fails fast with content_visibility_required", async () => {
    const bootstrap = {
      ...resolvedBootstrap(),
      content_visibility: "blind" as const,
    };
    const handle = startFakeHost(
      { bootstrap, replies: [] },
      async (stdin, stdout) => {
        const host = new HostRpc({ stdin, stdout });
        const outcome = await runPiWorker(bootstrap, host);
        await host.sendOutcome(outcome);
      },
    );
    const outcome = (await handle.outcomeSeen) as { Failed?: { kind: string } };
    expect(outcome.Failed?.kind).toBe("content_visibility_required");
    expect(
      handle.workerFrames.filter((frame) => "HostRequest" in frame),
    ).toHaveLength(0);
  }, 20000);

  test("oversized frame is rejected", async () => {
    const header = new Uint8Array(4);
    new DataView(header.buffer).setUint32(
      0,
      LOOP_WORKER_MAX_FRAME_BYTES + 1,
      false,
    );
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(header);
        controller.enqueue(new Uint8Array(8));
        controller.close();
      },
    });
    const reader = new FrameReader(stream);
    expect(reader.read()).rejects.toThrow("exceeds");
  });

  test("mid-frame EOF surfaces an error", async () => {
    const header = new Uint8Array(4);
    new DataView(header.buffer).setUint32(0, 64, false);
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(header);
        controller.close();
      },
    });
    const reader = new FrameReader(stream);
    await expect(reader.read()).rejects.toThrow("mid-frame");
  });

  test("concurrent RPC demux: two calls resolve independently", async () => {
    const channel = new TransformStream<Uint8Array, Uint8Array>();
    const writer = channel.writable.getWriter();
    // The worker's "stdin": replies travel here, the worker's HostRpc reads them.
    const workerStdin = new TransformStream<Uint8Array, Uint8Array>();
    const replyWriter = workerStdin.writable.getWriter();
    const sinkLike = {
      write: (chunk: Uint8Array) => {
        void writer.write(chunk);
        return chunk.length;
      },
      flush: () => 0,
    };
    const collected: number[] = [];
    const pump = (async () => {
      const reader = channel.readable.getReader();
      let buffer = new Uint8Array(0);
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        const merged = new Uint8Array(buffer.length + value.length);
        merged.set(buffer, 0);
        merged.set(value, buffer.length);
        buffer = merged;
        while (buffer.length >= 4) {
          const length = new DataView(
            buffer.buffer,
            buffer.byteOffset,
            buffer.byteLength,
          ).getUint32(0, false);
          if (buffer.length < 4 + length) break;
          const body = buffer.slice(4, 4 + length);
          buffer = buffer.slice(4 + length);
          const frame = JSON.parse(new TextDecoder().decode(body));
          collected.push(frame.HostRequest.id);
          // Reply in reverse order to prove id-based demux. Yield a macrotask
          // after scheduling the writes (bun TransformStream quirk: the write
          // must settle before the next read or the worker misses the reply).
          void replyWriter.write(
            encodeFrame({ HostResponse: { id: 2, result: { Ok: "second" } } }),
          );
          void replyWriter.write(
            encodeFrame({ HostResponse: { id: 1, result: { Ok: "first" } } }),
          );
          await new Promise((resolve) => setTimeout(resolve, 0));
        }
      }
    })();

    const host = new HostRpc({
      stdin: workerStdin.readable,
      stdout: sinkLike as unknown as Bun.FileSink,
    });
    const [first, second] = await Promise.all([
      host.call<string>({ EmitProgress: {} }),
      host.call<string>({ VisibleCapabilities: {} }),
    ]);
    expect(first).toBe("first");
    expect(second).toBe("second");
    // Both replies can resolve the calls before the host pump observes the
    // second request frame; wait for it deterministically.
    const deadline = Date.now() + 2000;
    while (collected.length < 2 && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 5));
    }
    expect([...collected].sort()).toEqual([1, 2]);
    void pump;
  });

  test("cancel mid-model-call maps to the cancelled LoopExit", async () => {
    const cancelOnce = Promise.withResolvers<void>();
    const replies: Reply[] = [
      { matches: (call) => "BuildPrompt" in call, reply: promptBundle() },
      {
        matches: (call) => "ResolveMessages" in call,
        reply: resolvedMessages().map((message) => ({
          role: message.role,
          content_ref: message.content_ref,
          content: "Say hi then call echo.",
        })),
      },
      {
        matches: (call) => "StreamModel" in call,
        reply: () => {
          cancelOnce.resolve();
          return new Error("model call cancelled");
        },
      },
      {
        matches: (call) => "StageCheckpointPayload" in call,
        reply: "staged:1",
      },
      {
        matches: (call) => "Checkpoint" in call,
        reply: "01950000-0000-7000-8000-000000000004",
      },
    ];
    const bootstrap = resolvedBootstrap();
    let handle: TestHost | null = null;
    const h = startFakeHost({ bootstrap, replies }, async (stdin, stdout) => {
      const rpc = new HostRpc({ stdin, stdout });
      // Cancel as soon as the model call reaches the host.
      void cancelOnce.promise.then(() => {
        handle?.sendHostFrame({ Cancel: { reason_kind: "host_cancellation" } });
      });
      const outcome = await runPiWorker(bootstrap, rpc);
      await rpc.sendOutcome(outcome);
    });
    handle = h as unknown as TestHost;
    const outcome = (await h.outcomeSeen) as { Exit?: { cancelled?: unknown } };
    expect(outcome.Exit?.cancelled).toBeDefined();
  }, 20000);

  test("early host EOF exits non-zero with a stderr line", async () => {
    const bootstrap = resolvedBootstrap();
    const h = startFakeHost(
      { bootstrap, replies: [] },
      async (stdin, stdout) => {
        const exitResolver = Promise.withResolvers<number>();
        const rpc = new HostRpc({
          stdin,
          stdout,
          onEarlyEof: () => {
            console.error(
              "ironclaw-pi-worker: host closed stdin before the worker finished",
            );
            exitResolver.resolve(1);
          },
        });
        try {
          const outcome = await runPiWorker(bootstrap, rpc);
          // After EOF the ack may never arrive; race it against the exit mark.
          await Promise.race([rpc.sendOutcome(outcome), exitResolver.promise]);
          exitResolver.resolve(0);
        } catch {
          exitResolver.resolve(1);
        }
        (h as unknown as { __exitCode?: number }).__exitCode =
          await exitResolver.promise;
      },
    );
    h.closeHostToWorker();
    await h.done;
    expect((h as unknown as { __exitCode?: number }).__exitCode).toBe(1);
  }, 20000);
});
