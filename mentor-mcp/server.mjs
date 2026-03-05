import { randomUUID } from "node:crypto";
import { mkdir, readFile, writeFile, stat } from "node:fs/promises";
import path from "node:path";

import Fastify from "fastify";
import { z } from "zod";

const app = Fastify({ logger: true });

const port = Number(process.env.MENTOR_MCP_PORT || "8791");
const protocolVersion = "2024-11-05";

const mentorName = process.env.MENTOR_NAME || "Lippyclaw Mentor";
const mentorPersonaFile = process.env.MENTOR_PERSONA_FILE || "/mentor/persona.md";
const mentorMemoryFile = process.env.MENTOR_MEMORY_FILE || "/data/mentor/memory.json";
const mentorMemoryWindow = Math.max(1, Number(process.env.MENTOR_MEMORY_WINDOW || "14"));

const llmBaseUrl = (
  process.env.MENTOR_LLM_BASE_URL ||
  process.env.SUB_LLM_BASE_URL ||
  process.env.MAIN_LLM_BASE_URL ||
  "https://llm.chutes.ai/v1"
).replace(/\/$/, "");
const llmApiKey =
  process.env.MENTOR_LLM_API_KEY ||
  process.env.SUB_LLM_API_KEY ||
  process.env.MAIN_LLM_API_KEY ||
  "";
const llmModel =
  process.env.MENTOR_LLM_MODEL ||
  process.env.SUB_LLM_MODEL ||
  process.env.MAIN_LLM_MODEL ||
  "MiniMaxAI/MiniMax-M2.5-TEE";

const mentorVoiceEnabled = (process.env.ENABLE_MENTOR_VOICE || "false") === "true";
const mentorVoiceProvider = (process.env.MENTOR_VOICE_PROVIDER || "auto").trim().toLowerCase();
const mentorVoiceMode = (process.env.MENTOR_CHUTES_VOICE_MODE || "run_api").trim();
const mentorVoiceBaseUrl = (
  process.env.MENTOR_VOICE_API_BASE_URL ||
  process.env.MAIN_LLM_BASE_URL ||
  "https://llm.chutes.ai/v1"
).replace(/\/$/, "");
const mentorVoiceApiKey =
  process.env.MENTOR_VOICE_API_KEY ||
  process.env.MAIN_LLM_API_KEY ||
  process.env.SUB_LLM_API_KEY ||
  "";
const mentorVoiceRunEndpoint =
  process.env.MENTOR_CHUTES_RUN_ENDPOINT || `${mentorVoiceBaseUrl}/run`;
const mentorWhisperEndpoint =
  process.env.MENTOR_CHUTES_WHISPER_ENDPOINT || "https://chutes-whisper-large-v3.chutes.ai/transcribe";
const mentorCsmEndpoint =
  process.env.MENTOR_CHUTES_CSM_ENDPOINT || "https://chutes-csm-1b.chutes.ai/speak";
const mentorKokoroEndpoint =
  process.env.MENTOR_CHUTES_KOKORO_ENDPOINT || "https://chutes-kokoro.chutes.ai/speak";
const mentorWhisperModel =
  process.env.MENTOR_CHUTES_WHISPER_MODEL || "openai/whisper-large-v3-turbo";
const mentorCloneModel = process.env.MENTOR_CHUTES_CSM_MODEL || "sesame/csm-1b";
const mentorKokoroModel =
  process.env.MENTOR_CHUTES_KOKORO_MODEL || "hexgrad/Kokoro-82M";
const mentorEnableKokoroFallback =
  (process.env.MENTOR_CHUTES_ENABLE_KOKORO_FALLBACK || "true") === "true";
const mentorVoiceSamplePath =
  process.env.MENTOR_VOICE_SAMPLE_PATH || "/data/mentor/master-voice.wav";
const mentorVoiceContextPath =
  process.env.MENTOR_VOICE_CONTEXT_PATH || "/data/mentor/voice_context.txt";
const mentorVoiceAutoTranscribe =
  (process.env.MENTOR_VOICE_AUTO_TRANSCRIBE || "true") === "true";
const mentorFishApiBaseUrl = (
  process.env.MENTOR_FISH_API_BASE_URL || "https://api.fish.audio"
).replace(/\/$/, "");
const mentorFishApiKey = process.env.MENTOR_FISH_API_KEY || "";
const mentorFishTtsEndpoint =
  process.env.MENTOR_FISH_TTS_ENDPOINT || `${mentorFishApiBaseUrl}/v1/tts`;
const mentorFishAsrEndpoint =
  process.env.MENTOR_FISH_ASR_ENDPOINT || `${mentorFishApiBaseUrl}/v1/asr`;
const mentorFishModel = process.env.MENTOR_FISH_MODEL || "s1";
const mentorFishReferenceId = process.env.MENTOR_FISH_REFERENCE_ID || "";
const mentorFishFormat = (process.env.MENTOR_FISH_FORMAT || "mp3").toLowerCase();
const mentorFishLatency = process.env.MENTOR_FISH_LATENCY || "normal";
const mentorFishNormalize = (process.env.MENTOR_FISH_NORMALIZE || "true") === "true";
const mentorFishIgnoreTimestamps =
  (process.env.MENTOR_FISH_IGNORE_TIMESTAMPS || "true") === "true";
const mentorFishTemperature = Number(process.env.MENTOR_FISH_TEMPERATURE || "0.7");
const mentorFishTopP = Number(process.env.MENTOR_FISH_TOP_P || "0.7");
const mentorFishRepetitionPenalty = Number(process.env.MENTOR_FISH_REPETITION_PENALTY || "1.2");
const mentorFishMaxNewTokens = Number(process.env.MENTOR_FISH_MAX_NEW_TOKENS || "1024");
const mentorFishChunkLength = Number(process.env.MENTOR_FISH_CHUNK_LENGTH || "240");

const mentorArtifactDir = process.env.MENTOR_ARTIFACT_DIR || "/data/artifacts/mentor";

const callParamsSchema = z.object({
  name: z.string().min(1),
  arguments: z.record(z.any()).optional(),
});

const chatArgsSchema = z.object({
  message: z.string().min(1),
  sessionId: z.string().min(1).optional(),
  voiceReply: z.boolean().optional(),
});

const speakArgsSchema = z.object({
  text: z.string().min(1),
});

const transcribeArgsSchema = z.object({
  base64Audio: z.string().min(1),
  mimeType: z.string().min(1).optional(),
  language: z.string().min(1).optional(),
});

let personaCache;
let memoryCache;
let contextCache;

const tools = [
  {
    name: "mentor.chat",
    description:
      "Chat with the mentor persona. Uses contextual memory and can optionally return a voice artifact.",
    inputSchema: {
      type: "object",
      properties: {
        message: { type: "string" },
        sessionId: { type: "string" },
        voiceReply: { type: "boolean" },
      },
      required: ["message"],
      additionalProperties: false,
    },
  },
  {
    name: "mentor.speak",
    description:
      "Convert text into mentor voice audio using the configured provider (Fish Audio or Chutes voice cloning pipeline).",
    inputSchema: {
      type: "object",
      properties: {
        text: { type: "string" },
      },
      required: ["text"],
      additionalProperties: false,
    },
  },
  {
    name: "mentor.transcribe",
    description: "Transcribe base64 audio to text using the configured STT backend (Fish ASR or Chutes Whisper).",
    inputSchema: {
      type: "object",
      properties: {
        base64Audio: { type: "string" },
        mimeType: { type: "string" },
        language: { type: "string" },
      },
      required: ["base64Audio"],
      additionalProperties: false,
    },
  },
  {
    name: "mentor.voice_bootstrap",
    description:
      "Generate or refresh transcript context from configured mentor voice sample audio.",
    inputSchema: {
      type: "object",
      properties: {},
      additionalProperties: false,
    },
  },
  {
    name: "mentor.status",
    description: "Return mentor runtime status including LLM and voice readiness.",
    inputSchema: {
      type: "object",
      properties: {},
      additionalProperties: false,
    },
  },
];

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

const parseNumberWithFallback = (rawValue, fallbackValue) => {
  const numeric = Number(rawValue);
  if (Number.isFinite(numeric)) {
    return numeric;
  }
  return fallbackValue;
};

const llmRequestTimeoutMs = Math.max(
  5000,
  parseNumberWithFallback(process.env.MENTOR_LLM_REQUEST_TIMEOUT_MS, 45000),
);
const llmMaxRetries = Math.max(
  0,
  parseNumberWithFallback(process.env.MENTOR_LLM_MAX_RETRIES, 2),
);
const llmRetryBackoffMs = Math.max(
  100,
  parseNumberWithFallback(process.env.MENTOR_LLM_RETRY_BACKOFF_MS, 1500),
);
const voiceRequestTimeoutMs = Math.max(
  5000,
  parseNumberWithFallback(process.env.MENTOR_VOICE_REQUEST_TIMEOUT_MS, 120000),
);

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const isRetryableHttpStatus = (status) => status === 429 || status >= 500;

const isRetryableFetchError = (error) => {
  const message = `${error?.message || ""}`.toLowerCase();
  return (
    error?.name === "AbortError" ||
    message.includes("fetch failed") ||
    message.includes("timeout") ||
    message.includes("eai_again") ||
    message.includes("enotfound") ||
    message.includes("econnreset")
  );
};

const fetchWithTimeout = (url, options, timeoutMs) =>
  fetch(url, {
    ...options,
    signal: AbortSignal.timeout(timeoutMs),
  });

const resolveVoiceProvider = () => {
  if (mentorVoiceProvider === "fish" || mentorVoiceProvider === "chutes") {
    return mentorVoiceProvider;
  }

  if (
    mentorVoiceProvider === "openai_compatible" ||
    mentorVoiceProvider === "run_api" ||
    mentorVoiceProvider === "chutes_direct"
  ) {
    return "chutes";
  }

  // Auto mode: Fish takes priority when explicitly configured.
  if (mentorFishApiKey.trim()) {
    return "fish";
  }

  return "chutes";
};

const activeVoiceProvider = resolveVoiceProvider();

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

const extensionForMimeType = (mimeType) => {
  const normalized = (mimeType || "").toLowerCase();
  if (normalized.includes("wav")) {
    return "wav";
  }
  if (normalized.includes("ogg")) {
    return "ogg";
  }
  if (normalized.includes("webm")) {
    return "webm";
  }
  if (normalized.includes("mp4") || normalized.includes("m4a")) {
    return "m4a";
  }
  if (normalized.includes("mpeg") || normalized.includes("mp3")) {
    return "mp3";
  }
  return "ogg";
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

const stripMinimaxToolCalls = (input) => {
  if (typeof input !== "string" || !input) {
    return "";
  }

  return input
    .replace(/<minimax:tool_call\b[^>]*>[\s\S]*?<\/minimax:tool_call>/gi, "")
    .replace(/<minimax:toolcall\b[^>]*>[\s\S]*?<\/minimax:toolcall>/gi, "")
    .replace(/<think\b[^>]*>[\s\S]*?<\/think>/gi, "")
    .replace(/<thinking\b[^>]*>[\s\S]*?<\/thinking>/gi, "")
    .trim();
};

const normalizeMentorReply = (raw) => {
  const cleaned = stripMinimaxToolCalls(raw.trim());
  if (cleaned) {
    return cleaned;
  }

  return "I hit a formatting issue. Please retry.";
};

const speechFallbackText = "Keep risk tight, take profit, and stay disciplined.";

const toSpeakableText = (input) => {
  if (typeof input !== "string") {
    return speechFallbackText;
  }

  let text = input;
  text = text.replace(/```[\s\S]*?```/g, " ");
  text = text.replace(/`[^`]*`/g, " ");
  text = text.replace(/!\[[^\]]*]\(([^)]+)\)/g, " ");
  text = text.replace(/\[([^\]]+)]\(([^)]+)\)/g, "$1");
  text = text.replace(/^\s{0,3}#{1,6}\s+/gm, "");
  text = text.replace(/^\s{0,3}>\s?/gm, "");
  text = text.replace(/^\s*([-*+•]|\d+[.)])\s+/gm, "");
  text = text.replace(/\*\*(.*?)\*\*/g, "$1");
  text = text.replace(/\*(.*?)\*/g, "$1");
  text = text.replace(/__(.*?)__/g, "$1");
  text = text.replace(/_(.*?)_/g, "$1");
  text = text.replace(/~~(.*?)~~/g, "$1");
  text = text.replace(/https?:\/\/\S+/gi, " ");
  text = text.replace(/[\\`*_~#>|[\]{}]/g, " ");
  text = text.replace(/\s+/g, " ").trim();

  return text || speechFallbackText;
};

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
    const response = await fetchWithTimeout(audioUrl, {
      headers: {
        authorization: `Bearer ${mentorVoiceApiKey}`,
      },
    }, voiceRequestTimeoutMs);

    if (!response.ok) {
      throw new Error(`Failed to fetch synthesized audio URL (${response.status}).`);
    }

    return Buffer.from(await response.arrayBuffer());
  }

  throw new Error("Unable to extract audio payload from Chutes response.");
};

const callMentorLlm = async (messages) => {
  if (!llmApiKey) {
    throw new Error("MENTOR_LLM_API_KEY/SUB_LLM_API_KEY/MAIN_LLM_API_KEY is not configured.");
  }

  const requestBody = JSON.stringify({
    model: llmModel,
    temperature: 0.2,
    max_tokens: 900,
    messages,
  });

  let data;
  let lastError;
  for (let attempt = 0; attempt <= llmMaxRetries; attempt += 1) {
    try {
      const response = await fetchWithTimeout(
        `${llmBaseUrl}/chat/completions`,
        {
          method: "POST",
          headers: {
            "content-type": "application/json",
            authorization: `Bearer ${llmApiKey}`,
          },
          body: requestBody,
        },
        llmRequestTimeoutMs,
      );

      if (!response.ok) {
        const errorBody = await response.text();
        const message =
          `Mentor LLM request failed (${response.status})` +
          ` attempt=${attempt + 1}/${llmMaxRetries + 1}: ${errorBody.slice(0, 300)}`;

        if (attempt < llmMaxRetries && isRetryableHttpStatus(response.status)) {
          lastError = new Error(message);
          const backoff = llmRetryBackoffMs * (attempt + 1);
          await sleep(backoff);
          continue;
        }

        throw new Error(message);
      }

      data = await response.json();
      break;
    } catch (error) {
      if (attempt < llmMaxRetries && isRetryableFetchError(error)) {
        lastError = error;
        const backoff = llmRetryBackoffMs * (attempt + 1);
        await sleep(backoff);
        continue;
      }
      throw error;
    }
  }

  if (!data) {
    throw new Error(
      `Mentor LLM request failed after retries: ${lastError?.message || "unknown error"}`,
    );
  }

  const message = data?.choices?.[0]?.message;
  const content = message?.content;

  if (typeof content === "string") {
    return normalizeMentorReply(content);
  }

  if (Array.isArray(content)) {
    const text = content
      .filter((part) => part?.type === "text" && typeof part?.text === "string")
      .map((part) => part.text)
      .join("\n")
      .trim();
    return normalizeMentorReply(text);
  }

  return normalizeMentorReply("");
};

const callChutesRunModel = async (model, input) => {
  const response = await fetchWithTimeout(mentorVoiceRunEndpoint, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${mentorVoiceApiKey}`,
    },
    body: JSON.stringify({
      model,
      input,
    }),
  }, voiceRequestTimeoutMs);

  if (!response.ok) {
    const errorBody = await response.text();
    throw new Error(`Chutes run request failed (${response.status}) model=${model}: ${errorBody.slice(0, 300)}`);
  }

  return response.json();
};

const callChutesDirectJson = async (endpoint, payload, label) => {
  const response = await fetchWithTimeout(endpoint, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${mentorVoiceApiKey}`,
    },
    body: JSON.stringify(payload),
  }, voiceRequestTimeoutMs);

  if (!response.ok) {
    const errorBody = await response.text();
    throw new Error(`Chutes direct ${label} failed (${response.status}) endpoint=${endpoint}: ${errorBody.slice(0, 300)}`);
  }

  return response.json();
};

const callChutesDirectAudio = async (endpoint, payload, label) => {
  const response = await fetchWithTimeout(endpoint, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${mentorVoiceApiKey}`,
    },
    body: JSON.stringify(payload),
  }, voiceRequestTimeoutMs);

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

const transcribeBufferFish = async (audioBuffer, mimeType, language) => {
  if (!mentorFishApiKey) {
    throw new Error("MENTOR_FISH_API_KEY is not configured.");
  }

  const normalizedMime = mimeType || "audio/ogg";
  const extension = extensionForMimeType(normalizedMime);
  const form = new FormData();
  const blob = new Blob([audioBuffer], { type: normalizedMime });
  form.append("audio", blob, `audio.${extension}`);
  form.append("ignore_timestamps", mentorFishIgnoreTimestamps ? "true" : "false");
  if (typeof language === "string" && language.trim()) {
    form.append("language", language.trim());
  }

  const response = await fetchWithTimeout(mentorFishAsrEndpoint, {
    method: "POST",
    headers: {
      authorization: `Bearer ${mentorFishApiKey}`,
    },
    body: form,
  }, voiceRequestTimeoutMs);

  if (!response.ok) {
    const errorBody = await response.text();
    throw new Error(`Fish ASR failed (${response.status}): ${errorBody.slice(0, 300)}`);
  }

  const payload = await response.json();
  const text = extractTextFromPayload(payload);
  if (!text) {
    throw new Error("Fish ASR response did not include transcript text.");
  }
  return text;
};

const synthesizeWithFish = async (text) => {
  if (!mentorFishApiKey) {
    throw new Error("MENTOR_FISH_API_KEY is not configured.");
  }

  const payload = {
    text,
    format: mentorFishFormat,
    normalize: mentorFishNormalize,
    latency: mentorFishLatency,
    temperature: parseNumberWithFallback(mentorFishTemperature, 0.7),
    top_p: parseNumberWithFallback(mentorFishTopP, 0.7),
    repetition_penalty: parseNumberWithFallback(mentorFishRepetitionPenalty, 1.2),
    max_new_tokens: parseNumberWithFallback(mentorFishMaxNewTokens, 1024),
    chunk_length: parseNumberWithFallback(mentorFishChunkLength, 240),
  };

  const referenceId = mentorFishReferenceId.trim();
  if (referenceId) {
    payload.reference_id = referenceId;
  }

  const response = await fetchWithTimeout(mentorFishTtsEndpoint, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${mentorFishApiKey}`,
      model: mentorFishModel,
    },
    body: JSON.stringify(payload),
  }, voiceRequestTimeoutMs);

  if (!response.ok) {
    const errorBody = await response.text();
    throw new Error(`Fish TTS failed (${response.status}): ${errorBody.slice(0, 300)}`);
  }

  const contentType = response.headers.get("content-type") || "";
  if (contentType.includes("application/json")) {
    const responsePayload = await response.json();
    return extractAudioBufferFromPayload(responsePayload);
  }

  return Buffer.from(await response.arrayBuffer());
};

const transcribeBufferOpenAICompatible = async (audioBuffer, mimeType) => {
  const form = new FormData();
  const ext = mimeType.includes("mpeg") || mimeType.includes("mp3") ? "mp3" : "ogg";
  const blob = new Blob([audioBuffer], { type: mimeType });
  form.append("file", blob, `audio.${ext}`);
  form.append("model", mentorWhisperModel);

  const response = await fetchWithTimeout(`${mentorVoiceBaseUrl}/audio/transcriptions`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${mentorVoiceApiKey}`,
    },
    body: form,
  }, voiceRequestTimeoutMs);

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
  if (activeVoiceProvider === "fish") {
    return transcribeBufferFish(audioBuffer, mimeType, language);
  }

  if (!mentorVoiceApiKey) {
    throw new Error("MENTOR_VOICE_API_KEY/MAIN_LLM_API_KEY is not configured.");
  }

  if (mentorVoiceMode === "openai_compatible") {
    return transcribeBufferOpenAICompatible(audioBuffer, mimeType);
  }

  if (mentorVoiceMode === "chutes_direct") {
    const directPayload = {
      audio_b64: audioBuffer.toString("base64"),
    };
    if (typeof language === "string" && language.trim()) {
      directPayload.language = language.trim();
    }

    const payload = await callChutesDirectJson(
      mentorWhisperEndpoint,
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
    payload = await callChutesRunModel(mentorWhisperModel, {
      audio_base64: audioBuffer.toString("base64"),
      mime_type: mimeType,
      language,
    });
  } catch (error) {
    if (mentorWhisperEndpoint) {
      const directFallbackPayload = {
        audio_b64: audioBuffer.toString("base64"),
      };
      if (typeof language === "string" && language.trim()) {
        directFallbackPayload.language = language.trim();
      }

      payload = await callChutesDirectJson(
        mentorWhisperEndpoint,
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

  const response = await fetchWithTimeout(`${mentorVoiceBaseUrl}/audio/speech`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${mentorVoiceApiKey}`,
    },
    body: JSON.stringify(body),
  }, voiceRequestTimeoutMs);

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

  if (mentorVoiceMode === "openai_compatible") {
    return synthesizeWithOpenAICompatibleSpeech(
      mentorCloneModel,
      text,
      contextText,
      sampleAudioBase64,
    );
  }

  if (mentorVoiceMode === "chutes_direct") {
    return callChutesDirectAudio(
      mentorCsmEndpoint,
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
    const payload = await callChutesRunModel(mentorCloneModel, {
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
    if (mentorCsmEndpoint) {
      return callChutesDirectAudio(
        mentorCsmEndpoint,
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
  if (mentorVoiceMode === "openai_compatible") {
    return synthesizeWithOpenAICompatibleSpeech(mentorKokoroModel, text, "", "");
  }

  if (mentorVoiceMode === "chutes_direct") {
    return callChutesDirectAudio(
      mentorKokoroEndpoint,
      {
        text,
        speed: 1,
      },
      "kokoro speak",
    );
  }

  try {
    const payload = await callChutesRunModel(mentorKokoroModel, {
      text,
      format: "mp3",
      output_format: "mp3",
    });
    return extractAudioBufferFromPayload(payload);
  } catch (error) {
    if (mentorKokoroEndpoint) {
      return callChutesDirectAudio(
        mentorKokoroEndpoint,
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

const loadPersona = async () => {
  if (typeof personaCache === "string") {
    return personaCache;
  }

  try {
    personaCache = await readFile(mentorPersonaFile, "utf8");
  } catch {
    personaCache = [
      `You are ${mentorName}.`,
      "You are a pragmatic technical mentor for Lippyclaw operators.",
      "Keep responses concise, operational, and production-oriented.",
      "When users are blocked, provide direct next commands first.",
    ].join("\n");
  }

  return personaCache;
};

const loadMemory = async () => {
  if (memoryCache) {
    return memoryCache;
  }

  try {
    const raw = await readFile(mentorMemoryFile, "utf8");
    const parsed = JSON.parse(raw);
    memoryCache = typeof parsed === "object" && parsed ? parsed : {};
  } catch {
    memoryCache = {};
  }

  return memoryCache;
};

const saveMemory = async () => {
  const store = await loadMemory();
  await ensureDir(path.dirname(mentorMemoryFile));
  await writeFile(mentorMemoryFile, JSON.stringify(store, null, 2), "utf8");
};

const getSessionHistory = async (sessionId) => {
  const store = await loadMemory();
  const turns = Array.isArray(store[sessionId]) ? store[sessionId] : [];
  return turns.slice(-mentorMemoryWindow * 2);
};

const appendSessionTurn = async (sessionId, role, content) => {
  const store = await loadMemory();
  const turns = Array.isArray(store[sessionId]) ? store[sessionId] : [];
  turns.push({ role, content, ts: new Date().toISOString() });
  store[sessionId] = turns.slice(-mentorMemoryWindow * 2);
  await saveMemory();
};

const bootstrapVoiceContext = async () => {
  if (!mentorVoiceEnabled) {
    throw new Error("Mentor voice is disabled (ENABLE_MENTOR_VOICE=false).");
  }

  if (activeVoiceProvider === "fish") {
    if (!mentorFishApiKey) {
      throw new Error("MENTOR_FISH_API_KEY is missing.");
    }

    if (contextCache && contextCache.trim()) {
      return contextCache;
    }

    if (await fileExists(mentorVoiceContextPath)) {
      const existing = (await readFile(mentorVoiceContextPath, "utf8")).trim();
      if (existing) {
        contextCache = existing;
        return existing;
      }
    }

    const defaultFishContext = mentorFishReferenceId.trim()
      ? `Fish voice reference active: ${mentorFishReferenceId.trim()}`
      : "Fish voice provider active (no reference_id configured; using model default voice).";
    await ensureDir(path.dirname(mentorVoiceContextPath));
    await writeFile(mentorVoiceContextPath, defaultFishContext, "utf8");
    contextCache = defaultFishContext;
    return defaultFishContext;
  }

  if (!mentorVoiceApiKey) {
    throw new Error("MENTOR_VOICE_API_KEY/MAIN_LLM_API_KEY is missing.");
  }

  if (contextCache && contextCache.trim()) {
    return contextCache;
  }

  if (await fileExists(mentorVoiceContextPath)) {
    const existing = (await readFile(mentorVoiceContextPath, "utf8")).trim();
    if (existing) {
      contextCache = existing;
      return existing;
    }
  }

  if (!mentorVoiceAutoTranscribe) {
    throw new Error("Mentor voice context missing and auto transcription is disabled.");
  }

  if (!(await fileExists(mentorVoiceSamplePath))) {
    throw new Error(`Mentor voice sample not found: ${mentorVoiceSamplePath}`);
  }

  const sampleAudio = await readFile(mentorVoiceSamplePath);
  const transcript = await transcribeBuffer(
    sampleAudio,
    sampleMimeType(mentorVoiceSamplePath),
  );

  await ensureDir(path.dirname(mentorVoiceContextPath));
  await writeFile(mentorVoiceContextPath, transcript, "utf8");
  contextCache = transcript;
  return transcript;
};

const synthesizeMentorSpeech = async (text) => {
  if (!mentorVoiceEnabled) {
    throw new Error("Mentor voice is disabled (ENABLE_MENTOR_VOICE=false).");
  }

  const speakableText = toSpeakableText(text);
  let audio;
  let backendUsed = "csm";
  let outputExtension = "mp3";

  if (activeVoiceProvider === "fish") {
    await bootstrapVoiceContext();
    audio = await synthesizeWithFish(speakableText);
    backendUsed = mentorFishReferenceId.trim() ? "fish_reference" : "fish";
    outputExtension = mentorFishFormat || "mp3";
  } else {
    if (!mentorVoiceApiKey) {
      throw new Error("MENTOR_VOICE_API_KEY/MAIN_LLM_API_KEY is missing.");
    }

    if (!(await fileExists(mentorVoiceSamplePath))) {
      throw new Error(`Mentor voice sample not found: ${mentorVoiceSamplePath}`);
    }

    const contextText = await bootstrapVoiceContext();
    const sampleAudio = await readFile(mentorVoiceSamplePath);
    try {
      audio = await synthesizeWithCloneModel(speakableText, contextText, sampleAudio);
    } catch (error) {
      if (!mentorEnableKokoroFallback) {
        throw error;
      }

      backendUsed = "kokoro_fallback";
      audio = await synthesizeWithKokoro(speakableText);
    }
  }

  await ensureDir(mentorArtifactDir);
  const fileName = `mentor-${Date.now()}-${randomUUID()}.${outputExtension}`;
  const outputPath = path.join(mentorArtifactDir, fileName);
  await writeFile(outputPath, audio);

  return {
    outputPath,
    backendUsed,
  };
};

const transcribeAudio = async (base64Audio, mimeType, language) => {
  const bytes = Buffer.from(base64Audio, "base64");
  const text = await transcribeBuffer(bytes, mimeType || "audio/ogg", language);
  return {
    text,
  };
};

const handleMentorChat = async (args) => {
  const parsed = chatArgsSchema.parse(args || {});
  const sessionId = parsed.sessionId || "default";
  const persona = await loadPersona();
  const history = await getSessionHistory(sessionId);
  const voiceModeInstruction = parsed.voiceReply
    ? "Voice mode policy: reply in 1-3 short sentences (max ~45 words), witty and practical, no rambling, plain text only. Never use markdown, bullet points, asterisks, code blocks, hashtags, or special wrapper characters."
    : "";

  const messages = [
    {
      role: "system",
      content: persona,
    },
    ...(voiceModeInstruction
      ? [
          {
            role: "system",
            content: voiceModeInstruction,
          },
        ]
      : []),
    ...history.map((turn) => ({ role: turn.role, content: turn.content })),
    {
      role: "user",
      content: parsed.message,
    },
  ];

  const reply = await callMentorLlm(messages);
  const outputReply = parsed.voiceReply ? toSpeakableText(reply) : reply;
  await appendSessionTurn(sessionId, "user", parsed.message);
  await appendSessionTurn(sessionId, "assistant", outputReply);

  let voiceArtifact;
  let voiceBackend;
  if (parsed.voiceReply) {
    const result = await synthesizeMentorSpeech(outputReply);
    voiceArtifact = result.outputPath;
    voiceBackend = result.backendUsed;
  }

  return {
    sessionId,
    mentor: mentorName,
    reply: outputReply,
    voiceArtifact,
    voiceBackend,
  };
};

const handleMentorSpeak = async (args) => {
  const parsed = speakArgsSchema.parse(args || {});
  const result = await synthesizeMentorSpeech(parsed.text);
  return {
    mentor: mentorName,
    voiceArtifact: result.outputPath,
    voiceBackend: result.backendUsed,
  };
};

const handleMentorTranscribe = async (args) => {
  const parsed = transcribeArgsSchema.parse(args || {});
  return transcribeAudio(parsed.base64Audio, parsed.mimeType, parsed.language);
};

const handleMentorVoiceBootstrap = async () => {
  const transcript = await bootstrapVoiceContext();
  return {
    mentor: mentorName,
    transcript,
    contextPath: mentorVoiceContextPath,
    samplePath: mentorVoiceSamplePath,
  };
};

const handleMentorStatus = async () => {
  const persona = await loadPersona();
  const contextReady = await fileExists(mentorVoiceContextPath);
  const sampleFileReady = await fileExists(mentorVoiceSamplePath);
  const fishReferenceConfigured = mentorFishReferenceId.trim().length > 0;
  const sampleReady =
    activeVoiceProvider === "fish" ? fishReferenceConfigured || sampleFileReady : sampleFileReady;

  return {
    mentor: mentorName,
    llm: {
      baseUrl: llmBaseUrl,
      model: llmModel,
      apiKeyConfigured: llmApiKey.length > 0,
    },
    voice: {
      enabled: mentorVoiceEnabled,
      provider: activeVoiceProvider,
      mode: mentorVoiceMode,
      apiBaseUrl: mentorVoiceBaseUrl,
      runEndpoint: mentorVoiceRunEndpoint,
      whisperEndpoint: mentorWhisperEndpoint,
      csmEndpoint: mentorCsmEndpoint,
      kokoroEndpoint: mentorKokoroEndpoint,
      apiKeyConfigured:
        activeVoiceProvider === "fish"
          ? mentorFishApiKey.length > 0
          : mentorVoiceApiKey.length > 0,
      whisperModel: mentorWhisperModel,
      cloneModel: mentorCloneModel,
      kokoroModel: mentorKokoroModel,
      kokoroFallback: mentorEnableKokoroFallback,
      samplePath: mentorVoiceSamplePath,
      contextPath: mentorVoiceContextPath,
      sampleReady,
      contextReady,
      fish: {
        apiBaseUrl: mentorFishApiBaseUrl,
        ttsEndpoint: mentorFishTtsEndpoint,
        asrEndpoint: mentorFishAsrEndpoint,
        model: mentorFishModel,
        apiKeyConfigured: mentorFishApiKey.length > 0,
        referenceConfigured: fishReferenceConfigured,
        format: mentorFishFormat,
      },
    },
    memory: {
      file: mentorMemoryFile,
      window: mentorMemoryWindow,
    },
    personaLoaded: persona.length > 0,
  };
};

app.get("/healthz", async () => {
  const contextReady = await fileExists(mentorVoiceContextPath);
  const sampleReady = await fileExists(mentorVoiceSamplePath);

  return {
    status: "ok",
    service: "mentor-mcp",
    mentor: mentorName,
    llmModel,
    voiceEnabled: mentorVoiceEnabled,
    voiceProvider: activeVoiceProvider,
    voiceMode: mentorVoiceMode,
    sampleReady,
    contextReady,
    timestamp: new Date().toISOString(),
  };
});

app.post("/bootstrap/voice", async (request, reply) => {
  try {
    const result = await handleMentorVoiceBootstrap();
    return reply.send(result);
  } catch (error) {
    request.log.error({ err: error }, "mentor voice bootstrap failed");
    return reply.code(500).send({
      error: error instanceof Error ? error.message : "Unknown bootstrap error",
    });
  }
});

const handleMcpRequest = async (request, reply) => {
  const body = request.body;
  if (!body || typeof body !== "object") {
    return reply.code(400).send(jsonRpcError(null, -32600, "Invalid Request"));
  }

  const { jsonrpc, id, method, params } = body;

  if (jsonrpc !== "2.0" || typeof method !== "string") {
    return reply.code(400).send(jsonRpcError(id ?? null, -32600, "Invalid Request"));
  }

  try {
    if (method === "initialize") {
      return reply.send(
        jsonRpcResult(id ?? null, {
          protocolVersion,
          serverInfo: {
            name: "mentor-mcp",
            version: "1.0.0",
          },
          capabilities: {
            tools: {},
          },
        }),
      );
    }

    if (method === "notifications/initialized") {
      return reply.code(204).send();
    }

    if (method === "tools/list") {
      return reply.send(jsonRpcResult(id ?? null, { tools }));
    }

    if (method === "tools/call") {
      const parsed = callParamsSchema.safeParse(params);
      if (!parsed.success) {
        return reply.send(
          jsonRpcError(id ?? null, -32602, "Invalid params", parsed.error.flatten()),
        );
      }

      const { name, arguments: toolArgs } = parsed.data;
      let result;

      if (name === "mentor.chat") {
        result = await handleMentorChat(toolArgs);
      } else if (name === "mentor.speak") {
        result = await handleMentorSpeak(toolArgs);
      } else if (name === "mentor.transcribe") {
        result = await handleMentorTranscribe(toolArgs);
      } else if (name === "mentor.voice_bootstrap") {
        result = await handleMentorVoiceBootstrap();
      } else if (name === "mentor.status") {
        result = await handleMentorStatus();
      } else {
        return reply.send(jsonRpcError(id ?? null, -32601, `Unknown tool: ${name}`));
      }

      return reply.send(
        jsonRpcResult(id ?? null, {
          content: textContent(result),
        }),
      );
    }

    return reply.send(jsonRpcError(id ?? null, -32601, `Method not found: ${method}`));
  } catch (error) {
    request.log.error({ err: error }, "mentor-mcp request failed");
    return reply.send(
      jsonRpcError(id ?? null, -32000, "Internal error", {
        message: error instanceof Error ? error.message : "Unknown error",
      }),
    );
  }
};

// Accept both legacy root MCP path and explicit /mcp path.
app.post("/", handleMcpRequest);
app.post("/mcp", handleMcpRequest);

app.listen({ host: "0.0.0.0", port }).then(() => {
  app.log.info(
    {
      port,
      mentorName,
      llmBaseUrl,
      llmModel,
      mentorVoiceEnabled,
      mentorVoiceProvider: activeVoiceProvider,
      mentorVoiceMode,
      mentorWhisperModel,
      mentorCloneModel,
      mentorKokoroModel,
      mentorFishModel,
      mentorFishReferenceConfigured: mentorFishReferenceId.trim().length > 0,
    },
    "mentor-mcp started",
  );
});
