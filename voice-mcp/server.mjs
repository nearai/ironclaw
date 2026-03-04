import { randomUUID } from "node:crypto";
import { mkdir, readFile, writeFile, stat } from "node:fs/promises";
import path from "node:path";

import Fastify from "fastify";
import { z } from "zod";

const app = Fastify({ logger: true });

const port = Number(process.env.VOICE_MCP_PORT || "8792");
const protocolVersion = "2024-11-05";

// ============================================================================
// Configuration
// ============================================================================

const voiceName = process.env.VOICE_NAME || "Lippyclaw Voice";
const voiceArtifactDir = process.env.VOICE_ARTIFACT_DIR || "/data/artifacts/voice";

// Voice API configuration (Chutes.ai or OpenAI-compatible)
const voiceEnabled = (process.env.ENABLE_VOICE || "false") === "true";
const voiceMode = (process.env.VOICE_MODE || "run_api").trim();
const voiceBaseUrl = (
  process.env.VOICE_API_BASE_URL ||
  process.env.MAIN_LLM_BASE_URL ||
  "https://llm.chutes.ai/v1"
).replace(/\/$/, "");
const voiceApiKey =
  process.env.VOICE_API_KEY ||
  process.env.MAIN_LLM_API_KEY ||
  "";
const voiceRunEndpoint =
  process.env.VOICE_RUN_ENDPOINT || `${voiceBaseUrl}/run`;
const voiceWhisperEndpoint =
  process.env.VOICE_CHUTES_WHISPER_ENDPOINT || "https://chutes-whisper-large-v3.chutes.ai/transcribe";
const voiceCsmEndpoint =
  process.env.VOICE_CHUTES_CSM_ENDPOINT || "https://chutes-csm-1b.chutes.ai/speak";
const voiceKokoroEndpoint =
  process.env.VOICE_CHUTES_KOKORO_ENDPOINT || "https://chutes-kokoro.chutes.ai/speak";

// STT (Speech-to-Text) configuration
const whisperModel =
  process.env.VOICE_WHISPER_MODEL || "openai/whisper-large-v3-turbo";

// TTS (Text-to-Speech) configuration
const cloneModel = process.env.VOICE_CLONE_MODEL || "sesame/csm-1b";
const kokoroModel = process.env.VOICE_KOKORO_MODEL || "hexgrad/Kokoro-82M";
const enableKokoroFallback =
  (process.env.VOICE_ENABLE_KOKORO_FALLBACK || "true") === "true";
const voiceSamplePath =
  process.env.VOICE_SAMPLE_PATH || "/data/voice/master-voice.wav";
const voiceContextPath =
  process.env.VOICE_CONTEXT_PATH || "/data/voice/voice_context.txt";

// ============================================================================
// MCP Tools Definition
// ============================================================================

const tools = [
  {
    name: "voice.transcribe",
    description:
      "Transcribe base64-encoded audio to text using Whisper model. Supports multiple audio formats.",
    inputSchema: {
      type: "object",
      properties: {
        base64Audio: {
          type: "string",
          description: "Base64-encoded audio data (data URI or raw base64)",
        },
        mimeType: {
          type: "string",
          description: "MIME type of the audio (e.g., audio/wav, audio/ogg, audio/mp4)",
        },
        language: {
          type: "string",
          description: "Optional language code for transcription (e.g., 'en', 'es')",
        },
      },
      required: ["base64Audio"],
      additionalProperties: false,
    },
  },
  {
    name: "voice.speak",
    description:
      "Convert text to speech using voice cloning. Uses a master voice sample for cloning.",
    inputSchema: {
      type: "object",
      properties: {
        text: {
          type: "string",
          description: "Text to synthesize into speech",
        },
        useCloning: {
          type: "boolean",
          description: "Whether to use voice cloning (default: true)",
        },
      },
      required: ["text"],
      additionalProperties: false,
    },
  },
  {
    name: "voice.bootstrap",
    description:
      "Generate or refresh transcript context from the master voice sample audio.",
    inputSchema: {
      type: "object",
      properties: {},
      additionalProperties: false,
    },
  },
  {
    name: "voice.status",
    description:
      "Return voice MCP server status including API readiness and model availability.",
    inputSchema: {
      type: "object",
      properties: {},
      additionalProperties: false,
    },
  },
];

// ============================================================================
// JSON-RPC Helpers
// ============================================================================

const jsonRpcResult = (id, result) => ({ jsonrpc: "2.0", id, result });

const jsonRpcError = (id, code, message, data) => ({
  jsonrpc: "2.0",
  id,
  error: {
    code,
    message,
    ...(data ? { data } : {}),
  },
});

const textContent = (payload) => [
  {
    type: "text",
    text: JSON.stringify(payload, null, 2),
  },
];

// ============================================================================
// Utility Functions
// ============================================================================

const ensureDir = async (targetPath) => {
  await mkdir(targetPath, { recursive: true });
};

const fileExists = async (filePath) => {
  try {
    await stat(filePath);
    return true;
  } catch {
    return false;
  }
};

const sampleMimeType = (samplePath) => {
  const ext = path.extname(samplePath).toLowerCase();
  if (ext === ".mp4" || ext === ".m4a") {
    return "audio/mp4";
  }
  if (ext === ".wav") {
    return "audio/wav";
  }
  if (ext === ".ogg") {
    return "audio/ogg";
  }
  if (ext === ".webm") {
    return "audio/webm";
  }
  return "audio/mpeg";
};

const stripDataUrl = (value) =>
  value.replace(/^data:[a-zA-Z0-9/._+-]+;base64,/, "").trim();

const decodeBase64Audio = (value) => {
  const clean = stripDataUrl(value);
  return Buffer.from(clean, "base64");
};

const findStringByCandidates = (node, candidates) => {
  if (!node || typeof node !== "object") {
    return undefined;
  }

  for (const key of candidates) {
    const value = node[key];
    if (typeof value === "string" && value.trim()) {
      return value.trim();
    }
  }

  for (const value of Object.values(node)) {
    if (Array.isArray(value)) {
      for (const item of value) {
        const found = findStringByCandidates(item, candidates);
        if (found) {
          return found;
        }
      }
    } else if (value && typeof value === "object") {
      const found = findStringByCandidates(value, candidates);
      if (found) {
        return found;
      }
    }
  }

  return undefined;
};

const extractTextFromPayload = (payload) =>
  findStringByCandidates(payload, ["text", "transcript", "output_text", "content"]);

const extractAudioBufferFromPayload = async (payload) => {
  const audioBase64 = findStringByCandidates(payload, [
    "audio_base64",
    "audio",
    "b64_json",
    "output_audio",
  ]);

  if (audioBase64) {
    return decodeBase64Audio(audioBase64);
  }

  const audioUrl = findStringByCandidates(payload, ["audio_url", "url"]);
  if (audioUrl && audioUrl.startsWith("http")) {
    const response = await fetch(audioUrl, {
      headers: {
        authorization: `Bearer ${voiceApiKey}`,
      },
    });

    if (!response.ok) {
      throw new Error(`Failed to fetch synthesized audio URL (${response.status}).`);
    }

    return Buffer.from(await response.arrayBuffer());
  }

  throw new Error("Unable to extract audio payload from response.");
};

// ============================================================================
// Voice API Calls
// ============================================================================

const callChutesRunModel = async (model, input) => {
  const response = await fetch(voiceRunEndpoint, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${voiceApiKey}`,
    },
    body: JSON.stringify({
      model,
      input,
    }),
  });

  if (!response.ok) {
    const errorBody = await response.text();
    throw new Error(`Chutes run request failed (${response.status}) model=${model}: ${errorBody.slice(0, 300)}`);
  }

  return response.json();
};

const callChutesDirectJson = async (endpoint, payload, label) => {
  const response = await fetch(endpoint, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${voiceApiKey}`,
    },
    body: JSON.stringify(payload),
  });

  if (!response.ok) {
    const errorBody = await response.text();
    throw new Error(`Chutes direct ${label} failed (${response.status}) endpoint=${endpoint}: ${errorBody.slice(0, 300)}`);
  }

  return response.json();
};

const callChutesDirectAudio = async (endpoint, payload, label) => {
  const response = await fetch(endpoint, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${voiceApiKey}`,
    },
    body: JSON.stringify(payload),
  });

  if (!response.ok) {
    const errorBody = await response.text();
    throw new Error(`Chutes direct ${label} failed (${response.status}) endpoint=${endpoint}: ${errorBody.slice(0, 300)}`);
  }

  const contentType = response.headers.get("content-type") || "";
  if (contentType.includes("application/json")) {
    const payloadJson = await response.json();
    return extractAudioBufferFromPayload(payloadJson);
  }

  return Buffer.from(await response.arrayBuffer());
};

const transcribeBufferOpenAICompatible = async (audioBuffer, mimeType) => {
  const form = new FormData();
  const ext = mimeType.includes("mpeg") || mimeType.includes("mp3") ? "mp3" : "ogg";
  const blob = new Blob([audioBuffer], { type: mimeType });
  form.append("file", blob, `audio.${ext}`);
  form.append("model", whisperModel);

  const response = await fetch(`${voiceBaseUrl}/audio/transcriptions`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${voiceApiKey}`,
    },
    body: form,
  });

  if (!response.ok) {
    const errorBody = await response.text();
    throw new Error(`Whisper transcription failed (${response.status}): ${errorBody.slice(0, 300)}`);
  }

  const payload = await response.json();
  const text = extractTextFromPayload(payload);
  if (!text) {
    throw new Error("Whisper response did not include transcript text.");
  }

  return text;
};

const transcribeBuffer = async (audioBuffer, mimeType, language) => {
  if (!voiceApiKey) {
    throw new Error("VOICE_API_KEY/MAIN_LLM_API_KEY is not configured.");
  }

  if (voiceMode === "openai_compatible") {
    return transcribeBufferOpenAICompatible(audioBuffer, mimeType);
  }

  if (voiceMode === "chutes_direct") {
    const directPayload = {
      audio_b64: audioBuffer.toString("base64"),
    };
    if (typeof language === "string" && language.trim()) {
      directPayload.language = language.trim();
    }

    const payload = await callChutesDirectJson(
      voiceWhisperEndpoint,
      directPayload,
      "whisper",
    );
    const text = extractTextFromPayload(payload);
    if (!text) {
      throw new Error("Chutes direct whisper response did not include transcript text.");
    }
    return text;
  }

  let payload;
  try {
    payload = await callChutesRunModel(whisperModel, {
      audio_base64: audioBuffer.toString("base64"),
      mime_type: mimeType,
      language,
    });
  } catch (error) {
    if (voiceWhisperEndpoint) {
      const directFallbackPayload = {
        audio_b64: audioBuffer.toString("base64"),
      };
      if (typeof language === "string" && language.trim()) {
        directFallbackPayload.language = language.trim();
      }

      payload = await callChutesDirectJson(
        voiceWhisperEndpoint,
        directFallbackPayload,
        "whisper",
      );
    } else {
      throw error;
    }
  }

  const text = extractTextFromPayload(payload);
  if (!text) {
    throw new Error("Chutes whisper run response did not include transcript text.");
  }

  return text;
};

const synthesizeWithOpenAICompatibleSpeech = async (model, text, contextText, sampleAudioBase64) => {
  const body = {
    model,
    input: text,
    format: "mp3",
    voice_context_text: contextText,
    context_text: contextText,
    voice_context_audio_base64: sampleAudioBase64,
    context_audio_base64: sampleAudioBase64,
  };

  const response = await fetch(`${voiceBaseUrl}/audio/speech`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${voiceApiKey}`,
    },
    body: JSON.stringify(body),
  });

  if (!response.ok) {
    const errorBody = await response.text();
    throw new Error(`OpenAI-compatible speech failed (${response.status}) model=${model}: ${errorBody.slice(0, 300)}`);
  }

  const contentType = response.headers.get("content-type") || "";
  if (contentType.includes("application/json")) {
    const payload = await response.json();
    return extractAudioBufferFromPayload(payload);
  }

  return Buffer.from(await response.arrayBuffer());
};

const synthesizeWithCloneModel = async (text, contextText, sampleAudioBuffer) => {
  const sampleAudioBase64 = sampleAudioBuffer.toString("base64");

  if (voiceMode === "openai_compatible") {
    return synthesizeWithOpenAICompatibleSpeech(
      cloneModel,
      text,
      contextText,
      sampleAudioBase64,
    );
  }

  if (voiceMode === "chutes_direct") {
    return callChutesDirectAudio(
      voiceCsmEndpoint,
      {
        text,
        max_duration_ms: 10000,
        audio_b64: sampleAudioBase64,
        context_text: contextText,
      },
      "csm speak",
    );
  }

  try {
    const payload = await callChutesRunModel(cloneModel, {
      text,
      target_text: text,
      text_context: contextText,
      context_text: contextText,
      audio_context_base64: sampleAudioBase64,
      context_audio_base64: sampleAudioBase64,
      format: "mp3",
      output_format: "mp3",
    });
    return extractAudioBufferFromPayload(payload);
  } catch (error) {
    if (voiceCsmEndpoint) {
      return callChutesDirectAudio(
        voiceCsmEndpoint,
        {
          text,
          max_duration_ms: 10000,
          audio_b64: sampleAudioBase64,
          context_text: contextText,
        },
        "csm speak",
      );
    }
    throw error;
  }
};

const synthesizeWithKokoro = async (text) => {
  if (voiceMode === "openai_compatible") {
    return synthesizeWithOpenAICompatibleSpeech(kokoroModel, text, "", "");
  }

  if (voiceMode === "chutes_direct") {
    return callChutesDirectAudio(
      voiceKokoroEndpoint,
      {
        text,
        speed: 1,
      },
      "kokoro speak",
    );
  }

  try {
    const payload = await callChutesRunModel(kokoroModel, {
      text,
      format: "mp3",
      output_format: "mp3",
    });
    return extractAudioBufferFromPayload(payload);
  } catch (error) {
    if (voiceKokoroEndpoint) {
      return callChutesDirectAudio(
        voiceKokoroEndpoint,
        {
          text,
          speed: 1,
        },
        "kokoro speak",
      );
    }
    throw error;
  }
};

const loadVoiceContext = async () => {
  try {
    return await readFile(voiceContextPath, "utf8");
  } catch {
    return "Voice sample for cloning.";
  }
};

const loadVoiceSample = async () => {
  try {
    return await readFile(voiceSamplePath);
  } catch {
    throw new Error(`Master voice sample not found at ${voiceSamplePath}. Please provide a voice sample for cloning.`);
  }
};

// ============================================================================
// HTTP Endpoints (for direct API access)
// ============================================================================

/**
 * POST /transcribe
 * Transcribe audio to text
 */
app.post("/transcribe", async (request, reply) => {
  const schema = z.object({
    base64Audio: z.string().min(1),
    mimeType: z.string().optional().default("audio/wav"),
    language: z.string().optional(),
  });

  try {
    const { base64Audio, mimeType, language } = schema.parse(request.body);

    if (!voiceEnabled) {
      return reply.code(503).send({ error: "Voice service is disabled" });
    }

    const audioBuffer = decodeBase64Audio(base64Audio);
    const text = await transcribeBuffer(audioBuffer, mimeType, language);

    return reply.send({ text });
  } catch (error) {
    app.log.error({ err: error }, "Transcribe endpoint error");
    return reply.code(400).send({
      error: error instanceof Error ? error.message : "Transcription failed",
    });
  }
});

/**
 * POST /speak
 * Synthesize text to speech
 */
app.post("/speak", async (request, reply) => {
  const schema = z.object({
    text: z.string().min(1),
    useCloning: z.boolean().optional().default(true),
  });

  try {
    const { text, useCloning } = schema.parse(request.body);

    if (!voiceEnabled) {
      return reply.code(503).send({ error: "Voice service is disabled" });
    }

    let audioBuffer;

    if (useCloning) {
      const sampleBuffer = await loadVoiceSample();
      const contextText = await loadVoiceContext();
      audioBuffer = await synthesizeWithCloneModel(text, contextText, sampleBuffer);
    } else {
      audioBuffer = await synthesizeWithKokoro(text);
    }

    // Save artifact
    const artifactId = randomUUID();
    const artifactPath = path.join(voiceArtifactDir, `${artifactId}.mp3`);
    await ensureDir(voiceArtifactDir);
    await writeFile(artifactPath, audioBuffer);

    return reply.send({
      text,
      artifactPath,
      artifactId,
    });
  } catch (error) {
    app.log.error({ err: error }, "Speak endpoint error");
    return reply.code(400).send({
      error: error instanceof Error ? error.message : "Speech synthesis failed",
    });
  }
});

/**
 * GET /healthz
 * Health check endpoint
 */
app.get("/healthz", async () => {
  return {
    status: "ok",
    voiceEnabled,
    voiceMode,
    whisperModel,
    cloneModel,
    kokoroModel,
  };
});

// ============================================================================
// MCP Protocol Handlers
// ============================================================================

const handleMcpRequest = async (request, reply) => {
  const body = request.body;

  if (body.method === "initialize") {
    return reply.send(
      jsonRpcResult(body.id, {
        protocolVersion,
        capabilities: {
          tools: {},
        },
        serverInfo: {
          name: "voice-mcp",
          version: "1.0.0",
        },
      }),
    );
  }

  if (body.method === "notifications/initialized") {
    return reply.send({});
  }

  if (body.method === "tools/list") {
    return reply.send(jsonRpcResult(body.id, { tools }));
  }

  if (body.method === "tools/call") {
    const params = body.params;
    const validationResult = z
      .object({
        name: z.string(),
        arguments: z.record(z.any()).optional(),
      })
      .safeParse(params);

    if (!validationResult.success) {
      return reply.send(
        jsonRpcError(body.id, -32602, "Invalid params", validationResult.error),
      );
    }

    const { name, arguments: args } = validationResult.data;

    try {
      switch (name) {
        case "voice.transcribe": {
          if (!voiceEnabled) {
            return reply.send(
              jsonRpcError(body.id, -32000, "Voice service is disabled"),
            );
          }

          const transcribeSchema = z.object({
            base64Audio: z.string().min(1),
            mimeType: z.string().optional().default("audio/wav"),
            language: z.string().optional(),
          });

          const { base64Audio, mimeType, language } = transcribeSchema.parse(args);
          const audioBuffer = decodeBase64Audio(base64Audio);
          const text = await transcribeBuffer(audioBuffer, mimeType, language);

          return reply.send(
            jsonRpcResult(body.id, {
              content: [{ type: "text", text }],
            }),
          );
        }

        case "voice.speak": {
          if (!voiceEnabled) {
            return reply.send(
              jsonRpcError(body.id, -32000, "Voice service is disabled"),
            );
          }

          const speakSchema = z.object({
            text: z.string().min(1),
            useCloning: z.boolean().optional().default(true),
          });

          const { text, useCloning } = speakSchema.parse(args);

          let audioBuffer;
          if (useCloning) {
            const sampleBuffer = await loadVoiceSample();
            const contextText = await loadVoiceContext();
            audioBuffer = await synthesizeWithCloneModel(text, contextText, sampleBuffer);
          } else {
            audioBuffer = await synthesizeWithKokoro(text);
          }

          // Save artifact
          const artifactId = randomUUID();
          const artifactPath = path.join(voiceArtifactDir, `${artifactId}.mp3`);
          await ensureDir(voiceArtifactDir);
          await writeFile(artifactPath, audioBuffer);

          return reply.send(
            jsonRpcResult(body.id, {
              content: [
                {
                  type: "text",
                  text: `Voice artifact created: ${artifactPath}`,
                },
              ],
              artifact: {
                id: artifactId,
                path: artifactPath,
                mimeType: "audio/mpeg",
              },
            }),
          );
        }

        case "voice.bootstrap": {
          const contextText = await loadVoiceContext();
          return reply.send(
            jsonRpcResult(body.id, {
              content: [
                {
                  type: "text",
                  text: `Voice context loaded: ${contextText.slice(0, 200)}...`,
                },
              ],
            }),
          );
        }

        case "voice.status": {
          const sampleExists = await fileExists(voiceSamplePath);
          return reply.send(
            jsonRpcResult(body.id, {
              content: [
                {
                  type: "text",
                  text: JSON.stringify(
                    {
                      voiceEnabled,
                      voiceMode,
                      whisperModel,
                      cloneModel,
                      kokoroModel,
                      whisperEndpoint: voiceWhisperEndpoint,
                      csmEndpoint: voiceCsmEndpoint,
                      kokoroEndpoint: voiceKokoroEndpoint,
                      sampleExists,
                      samplePath: voiceSamplePath,
                    },
                    null,
                    2,
                  ),
                },
              ],
            }),
          );
        }

        default:
          return reply.send(
            jsonRpcError(body.id, -32601, `Unknown tool: ${name}`),
          );
      }
    } catch (error) {
      return reply.send(
        jsonRpcError(
          body.id,
          -32000,
          error instanceof Error ? error.message : "Tool execution failed",
        ),
      );
    }
  }

  return reply.send(
    jsonRpcError(body.id, -32601, `Unknown method: ${body.method}`),
  );
};

// Accept both legacy root MCP path and explicit /mcp path.
app.post("/", handleMcpRequest);
app.post("/mcp", handleMcpRequest);

// ============================================================================
// Server Startup
// ============================================================================

const start = async () => {
  await ensureDir(voiceArtifactDir);

  app.log.info(`Starting voice-mcp server on port ${port}`);
  app.log.info(`Voice enabled: ${voiceEnabled}`);
  app.log.info(`Voice mode: ${voiceMode}`);
  app.log.info(`Whisper model: ${whisperModel}`);
  app.log.info(`Clone model: ${cloneModel}`);
  app.log.info(`Kokoro model: ${kokoroModel}`);

  try {
    await app.listen({ port, host: "0.0.0.0" });
  } catch (error) {
    app.log.error({ err: error }, "Failed to start voice-mcp server");
    process.exit(1);
  }
};

start();
