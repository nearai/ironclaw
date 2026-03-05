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
const mentorImageArtifactDir = process.env.MENTOR_IMAGE_ARTIFACT_DIR || mentorArtifactDir;
const mentorImageEnabled = (process.env.ENABLE_MENTOR_IMAGE || "true") === "true";
const mentorImageProvider = (process.env.MENTOR_IMAGE_PROVIDER || "auto").trim().toLowerCase();
const mentorImageApiKey = (
  process.env.MENTOR_IMAGE_API_KEY ||
  process.env.MENTOR_VOICE_API_KEY ||
  process.env.MAIN_LLM_API_KEY ||
  process.env.SUB_LLM_API_KEY ||
  ""
).trim();
const mentorImageSize = (process.env.MENTOR_IMAGE_SIZE || "1024x1024").trim().toLowerCase();
const mentorImageResponseFormat = (process.env.MENTOR_IMAGE_RESPONSE_FORMAT || "url")
  .trim()
  .toLowerCase();
const mentorChutesImageMode = (process.env.MENTOR_CHUTES_IMAGE_MODE || "run_api")
  .trim()
  .toLowerCase();
const mentorChutesImageRunEndpoint =
  process.env.MENTOR_CHUTES_IMAGE_RUN_ENDPOINT || mentorVoiceRunEndpoint;
const mentorChutesImageModel =
  process.env.MENTOR_CHUTES_IMAGE_MODEL || "black-forest-labs/FLUX.1-schnell";
const mentorChutesImageEndpoint = (process.env.MENTOR_CHUTES_IMAGE_ENDPOINT || "").trim();
const mentorNovitaApiBaseUrl = (
  process.env.MENTOR_NOVITA_API_BASE_URL || "https://api.novita.ai"
).replace(/\/$/, "");
const mentorNovitaApiKey = (
  process.env.MENTOR_NOVITA_API_KEY ||
  process.env.MENTOR_IMAGE_API_KEY ||
  ""
).trim();
const mentorNovitaImageEndpoint =
  process.env.MENTOR_NOVITA_IMAGE_ENDPOINT || `${mentorNovitaApiBaseUrl}/v3/seedream-3-0-txt2img`;
const mentorNovitaImageModel = process.env.MENTOR_NOVITA_IMAGE_MODEL || "seedream-3.0";
const mentorNovitaResponseFormat = (process.env.MENTOR_NOVITA_RESPONSE_FORMAT || "url")
  .trim()
  .toLowerCase();
const mentorVideoArtifactDir = process.env.MENTOR_VIDEO_ARTIFACT_DIR || mentorArtifactDir;
const mentorVideoEnabled = (process.env.ENABLE_MENTOR_VIDEO || "true") === "true";
const mentorVideoProvider = (process.env.MENTOR_VIDEO_PROVIDER || "auto").trim().toLowerCase();
const mentorVideoApiKey = (
  process.env.MENTOR_VIDEO_API_KEY ||
  process.env.MENTOR_IMAGE_API_KEY ||
  process.env.MENTOR_VOICE_API_KEY ||
  process.env.MAIN_LLM_API_KEY ||
  ""
).trim();
const mentorVideoDurationSeconds = Math.max(
  1,
  Math.min(30, Number(process.env.MENTOR_VIDEO_DURATION_SECONDS || "5")),
);
const mentorVideoSize = (process.env.MENTOR_VIDEO_SIZE || "1024x576").trim().toLowerCase();
const mentorChutesVideoMode = (process.env.MENTOR_CHUTES_VIDEO_MODE || "run_api")
  .trim()
  .toLowerCase();
const mentorChutesVideoRunEndpoint =
  process.env.MENTOR_CHUTES_VIDEO_RUN_ENDPOINT || mentorVoiceRunEndpoint;
const mentorChutesVideoModel =
  process.env.MENTOR_CHUTES_VIDEO_MODEL || "genmo/mochi-1-preview";
const mentorChutesVideoEndpoint = (process.env.MENTOR_CHUTES_VIDEO_ENDPOINT || "").trim();
const mentorNovitaVideoEndpoint =
  process.env.MENTOR_NOVITA_VIDEO_ENDPOINT || `${mentorNovitaApiBaseUrl}/v3/video/t2v`;
const mentorNovitaVideoModel = process.env.MENTOR_NOVITA_VIDEO_MODEL || "seedance-1.0";

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

const imageArgsSchema = z.object({
  prompt: z.string().min(1),
  provider: z.enum(["auto", "chutes", "novita"]).optional(),
  size: z.string().regex(/^\d{2,4}x\d{2,4}$/).optional(),
  negativePrompt: z.string().min(1).optional(),
  style: z.string().min(1).optional(),
  seed: z.number().int().nonnegative().optional(),
});

const videoArgsSchema = z.object({
  prompt: z.string().min(1),
  provider: z.enum(["auto", "chutes", "novita"]).optional(),
  size: z.string().regex(/^\d{2,4}x\d{2,4}$/).optional(),
  negativePrompt: z.string().min(1).optional(),
  durationSeconds: z.number().int().min(1).max(30).optional(),
  seed: z.number().int().nonnegative().optional(),
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
  {
    name: "mentor.image",
    description:
      "Generate an image artifact using the configured provider (Chutes or Novita).",
    inputSchema: {
      type: "object",
      properties: {
        prompt: { type: "string" },
        provider: { type: "string", enum: ["auto", "chutes", "novita"] },
        size: { type: "string" },
        negativePrompt: { type: "string" },
        style: { type: "string" },
        seed: { type: "integer" },
      },
      required: ["prompt"],
      additionalProperties: false,
    },
  },
  {
    name: "mentor.video",
    description:
      "Generate a short video artifact using the configured provider (Chutes or Novita).",
    inputSchema: {
      type: "object",
      properties: {
        prompt: { type: "string" },
        provider: { type: "string", enum: ["auto", "chutes", "novita"] },
        size: { type: "string" },
        negativePrompt: { type: "string" },
        durationSeconds: { type: "integer" },
        seed: { type: "integer" },
      },
      required: ["prompt"],
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
const imageRequestTimeoutMs = Math.max(
  5000,
  parseNumberWithFallback(process.env.MENTOR_IMAGE_REQUEST_TIMEOUT_MS, 120000),
);
const mentorImageDefaultNumImages = Math.max(
  1,
  Math.min(4, Math.trunc(parseNumberWithFallback(process.env.MENTOR_IMAGE_NUM_IMAGES, 1))),
);
const mentorImageMaxPromptChars = Math.max(
  64,
  Math.trunc(parseNumberWithFallback(process.env.MENTOR_IMAGE_MAX_PROMPT_CHARS, 1500)),
);
const videoRequestTimeoutMs = Math.max(
  5000,
  parseNumberWithFallback(process.env.MENTOR_VIDEO_REQUEST_TIMEOUT_MS, 180000),
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

const resolveImageProvider = (requestedProvider) => {
  const normalized = (requestedProvider || "").trim().toLowerCase();
  if (normalized === "chutes" || normalized === "novita") {
    return normalized;
  }

  if (mentorImageProvider === "chutes" || mentorImageProvider === "novita") {
    return mentorImageProvider;
  }

  if (mentorNovitaApiKey) {
    return "novita";
  }

  return "chutes";
};

const resolveVideoProvider = (requestedProvider) => {
  const normalized = (requestedProvider || "").trim().toLowerCase();
  if (normalized === "chutes" || normalized === "novita") {
    return normalized;
  }

  if (mentorVideoProvider === "chutes" || mentorVideoProvider === "novita") {
    return mentorVideoProvider;
  }

  if (mentorNovitaApiKey) {
    return "novita";
  }

  return "chutes";
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

const extensionForImageMimeType = (mimeType) => {
  const normalized = (mimeType || "").toLowerCase();
  if (normalized.includes("png")) {
    return "png";
  }
  if (normalized.includes("jpeg") || normalized.includes("jpg")) {
    return "jpg";
  }
  if (normalized.includes("webp")) {
    return "webp";
  }
  if (normalized.includes("gif")) {
    return "gif";
  }
  return "png";
};

const mimeTypeForImageExtension = (extension) => {
  const normalized = `${extension || ""}`.toLowerCase();
  if (normalized === "jpg" || normalized === "jpeg") {
    return "image/jpeg";
  }
  if (normalized === "webp") {
    return "image/webp";
  }
  if (normalized === "gif") {
    return "image/gif";
  }
  return "image/png";
};

const extensionForVideoMimeType = (mimeType) => {
  const normalized = (mimeType || "").toLowerCase();
  if (normalized.includes("webm")) {
    return "webm";
  }
  if (normalized.includes("quicktime")) {
    return "mov";
  }
  return "mp4";
};

const mimeTypeForVideoExtension = (extension) => {
  const normalized = `${extension || ""}`.toLowerCase();
  if (normalized === "webm") {
    return "video/webm";
  }
  if (normalized === "mov") {
    return "video/quicktime";
  }
  return "video/mp4";
};

const parseImageSize = (sizeValue) => {
  const fallback = { width: 1024, height: 1024, size: "1024x1024" };
  const raw = (sizeValue || mentorImageSize || fallback.size).toString().trim().toLowerCase();
  const match = raw.match(/^(\d{2,4})x(\d{2,4})$/);
  if (!match) {
    return fallback;
  }

  const width = Math.min(2048, Math.max(128, Number(match[1])));
  const height = Math.min(2048, Math.max(128, Number(match[2])));
  return { width, height, size: `${width}x${height}` };
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
    .replace(/<invoke\b[^>]*>[\s\S]*?<\/invoke>/gi, "")
    .replace(/<parameter\b[^>]*>[\s\S]*?<\/parameter>/gi, "")
    .replace(/<think\b[^>]*>[\s\S]*?<\/think>/gi, "")
    .replace(/<thinking\b[^>]*>[\s\S]*?<\/thinking>/gi, "")
    .trim();
};

const normalizeMentorReply = (raw) => {
  const cleaned = stripMinimaxToolCalls(raw.trim());
  if (!cleaned) {
    return "I hit a formatting issue. Please retry.";
  }

  const lower = cleaned.toLowerCase();
  if (
    lower.includes("cli-mcp-serverruncommand")
    || lower.includes("<invoke")
    || lower.includes("</invoke>")
    || lower.includes("runcommand")
  ) {
    return "Read-only mentor mode: I can analyze and advise, but I cannot run commands or modify files.";
  }

  return cleaned;
};

const speechFallbackText = "Keep risk tight, take profit, and stay disciplined.";
const mentorReadOnlyPolicy =
  "Read-only mentor policy: you provide analysis and recommendations only. " +
  "You cannot execute commands, access shells, edit files, trigger deploys, or claim you performed actions. " +
  "If asked to perform actions, state that you are read-only and provide exact next steps for the operator.";

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

const decodePossibleBase64 = (value) => {
  if (typeof value !== "string") {
    return undefined;
  }

  const trimmed = value.trim();
  if (!trimmed) {
    return undefined;
  }

  const normalized = trimmed.startsWith("data:")
    ? trimmed.split(",", 2)[1] || ""
    : trimmed;
  if (!normalized) {
    return undefined;
  }

  try {
    const decoded = Buffer.from(normalized, "base64");
    if (!decoded.length) {
      return undefined;
    }
    return decoded;
  } catch {
    return undefined;
  }
};

const parseImageDataUrl = (value) => {
  if (typeof value !== "string") {
    return undefined;
  }
  const trimmed = value.trim();
  if (!trimmed.startsWith("data:image/")) {
    return undefined;
  }

  const [header, body] = trimmed.split(",", 2);
  if (!header || !body || !header.includes(";base64")) {
    return undefined;
  }
  const mime = header.replace(/^data:/, "").split(";")[0] || "image/png";
  const bytes = decodePossibleBase64(body);
  if (!bytes) {
    return undefined;
  }
  return { mime, bytes };
};

const parseVideoDataUrl = (value) => {
  if (typeof value !== "string") {
    return undefined;
  }
  const trimmed = value.trim();
  if (!trimmed.startsWith("data:video/")) {
    return undefined;
  }

  const [header, body] = trimmed.split(",", 2);
  if (!header || !body || !header.includes(";base64")) {
    return undefined;
  }
  const mime = header.replace(/^data:/, "").split(";")[0] || "video/mp4";
  const bytes = decodePossibleBase64(body);
  if (!bytes) {
    return undefined;
  }
  return { mime, bytes };
};

const extractImageCandidateFromPayload = (payload) => {
  const dataUrl = findStringByCandidates(payload, [
    "image_data_url",
    "data_url",
    "imageDataUrl",
  ]);
  const parsedDataUrl = parseImageDataUrl(dataUrl);
  if (parsedDataUrl) {
    return parsedDataUrl;
  }

  const imageBase64 = findStringByCandidates(payload, [
    "image_b64",
    "image_base64",
    "b64_json",
    "base64",
    "image",
  ]);
  const decoded = decodePossibleBase64(imageBase64);
  if (decoded) {
    return { mime: "image/png", bytes: decoded };
  }

  const imageUrl = findStringByCandidates(payload, [
    "image_url",
    "url",
    "output_url",
    "imageUrl",
  ]);
  if (typeof imageUrl === "string" && imageUrl.startsWith("http")) {
    return { url: imageUrl.trim() };
  }

  return undefined;
};

const extractVideoCandidateFromPayload = (payload) => {
  const dataUrl = findStringByCandidates(payload, [
    "video_data_url",
    "data_url",
    "videoDataUrl",
  ]);
  const parsedDataUrl = parseVideoDataUrl(dataUrl);
  if (parsedDataUrl) {
    return parsedDataUrl;
  }

  const videoBase64 = findStringByCandidates(payload, [
    "video_b64",
    "video_base64",
    "base64_video",
    "b64_json",
    "video",
  ]);
  const decoded = decodePossibleBase64(videoBase64);
  if (decoded) {
    return { mime: "video/mp4", bytes: decoded };
  }

  const videoUrl = findStringByCandidates(payload, [
    "video_url",
    "url",
    "output_url",
    "videoUrl",
  ]);
  if (typeof videoUrl === "string" && videoUrl.startsWith("http")) {
    return { url: videoUrl.trim() };
  }

  return undefined;
};

const downloadImageFromUrl = async (url) => {
  const headers = {};
  if (url.includes("novita.ai") && mentorNovitaApiKey) {
    headers.authorization = `Bearer ${mentorNovitaApiKey}`;
  } else if (url.includes("chutes.ai") && mentorImageApiKey) {
    headers.authorization = `Bearer ${mentorImageApiKey}`;
  }

  const response = await fetchWithTimeout(url, { method: "GET", headers }, imageRequestTimeoutMs);
  if (!response.ok) {
    const errorBody = await response.text();
    throw new Error(`Image download failed (${response.status}): ${errorBody.slice(0, 300)}`);
  }

  const mime = response.headers.get("content-type") || "image/png";
  const bytes = Buffer.from(await response.arrayBuffer());
  return { mime, bytes };
};

const downloadVideoFromUrl = async (url) => {
  const headers = {};
  if (url.includes("novita.ai") && mentorNovitaApiKey) {
    headers.authorization = `Bearer ${mentorNovitaApiKey}`;
  } else if (url.includes("chutes.ai") && mentorVideoApiKey) {
    headers.authorization = `Bearer ${mentorVideoApiKey}`;
  }

  const response = await fetchWithTimeout(url, { method: "GET", headers }, videoRequestTimeoutMs);
  if (!response.ok) {
    const errorBody = await response.text();
    throw new Error(`Video download failed (${response.status}): ${errorBody.slice(0, 300)}`);
  }

  const mime = response.headers.get("content-type") || "video/mp4";
  const bytes = Buffer.from(await response.arrayBuffer());
  return { mime, bytes };
};

const normalizeImagePrompt = (prompt) => {
  const trimmed = `${prompt || ""}`.trim();
  if (!trimmed) {
    throw new Error("Image prompt is required.");
  }

  const bounded = trimmed.slice(0, mentorImageMaxPromptChars);
  return bounded;
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

const callChutesImageGeneration = async ({ prompt, size, negativePrompt, style, seed }) => {
  if (!mentorImageApiKey) {
    throw new Error("MENTOR_IMAGE_API_KEY (or fallback MENTOR_VOICE_API_KEY/MAIN_LLM_API_KEY) is missing.");
  }

  const parsedSize = parseImageSize(size);
  const requestInput = {
    prompt: normalizeImagePrompt(prompt),
    size: parsedSize.size,
    width: parsedSize.width,
    height: parsedSize.height,
    num_images: mentorImageDefaultNumImages,
    response_format: mentorImageResponseFormat,
  };

  if (negativePrompt && negativePrompt.trim()) {
    requestInput.negative_prompt = negativePrompt.trim();
  }
  if (style && style.trim()) {
    requestInput.style = style.trim();
  }
  if (Number.isInteger(seed)) {
    requestInput.seed = seed;
  }

  let payload;
  if (mentorChutesImageMode === "direct") {
    if (!mentorChutesImageEndpoint) {
      throw new Error("MENTOR_CHUTES_IMAGE_ENDPOINT is required when MENTOR_CHUTES_IMAGE_MODE=direct.");
    }
    const response = await fetchWithTimeout(mentorChutesImageEndpoint, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${mentorImageApiKey}`,
      },
      body: JSON.stringify(requestInput),
    }, imageRequestTimeoutMs);

    if (!response.ok) {
      const errorBody = await response.text();
      throw new Error(`Chutes direct image failed (${response.status}): ${errorBody.slice(0, 300)}`);
    }

    const contentType = response.headers.get("content-type") || "";
    if (contentType.includes("application/json")) {
      payload = await response.json();
    } else {
      const bytes = Buffer.from(await response.arrayBuffer());
      return {
        providerUsed: "chutes",
        size: parsedSize.size,
        mime: contentType || "image/png",
        bytes,
      };
    }
  } else {
    const response = await fetchWithTimeout(mentorChutesImageRunEndpoint, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${mentorImageApiKey}`,
      },
      body: JSON.stringify({
        model: mentorChutesImageModel,
        input: requestInput,
      }),
    }, imageRequestTimeoutMs);

    if (!response.ok) {
      const errorBody = await response.text();
      throw new Error(`Chutes image run request failed (${response.status}): ${errorBody.slice(0, 300)}`);
    }

    payload = await response.json();
  }

  const candidate =
    extractImageCandidateFromPayload(payload)
    || extractImageCandidateFromPayload(payload?.output)
    || extractImageCandidateFromPayload(payload?.result);

  if (!candidate) {
    throw new Error("Chutes image response did not include image content or image URL.");
  }

  if (candidate.url) {
    const downloaded = await downloadImageFromUrl(candidate.url);
    return {
      providerUsed: "chutes",
      size: parsedSize.size,
      mime: downloaded.mime,
      bytes: downloaded.bytes,
      sourceUrl: candidate.url,
    };
  }

  return {
    providerUsed: "chutes",
    size: parsedSize.size,
    mime: candidate.mime || "image/png",
    bytes: candidate.bytes,
  };
};

const callNovitaImageGeneration = async ({ prompt, size, negativePrompt, style, seed }) => {
  if (!mentorNovitaApiKey) {
    throw new Error("MENTOR_NOVITA_API_KEY is missing.");
  }

  const parsedSize = parseImageSize(size);
  const payload = {
    prompt: normalizeImagePrompt(prompt),
    model_name: mentorNovitaImageModel,
    width: parsedSize.width,
    height: parsedSize.height,
    size: parsedSize.size,
    response_format: mentorNovitaResponseFormat,
    num_images: mentorImageDefaultNumImages,
  };

  if (negativePrompt && negativePrompt.trim()) {
    payload.negative_prompt = negativePrompt.trim();
  }
  if (style && style.trim()) {
    payload.style = style.trim();
  }
  if (Number.isInteger(seed)) {
    payload.seed = seed;
  }

  const response = await fetchWithTimeout(mentorNovitaImageEndpoint, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${mentorNovitaApiKey}`,
    },
    body: JSON.stringify(payload),
  }, imageRequestTimeoutMs);

  if (!response.ok) {
    const errorBody = await response.text();
    throw new Error(`Novita image request failed (${response.status}): ${errorBody.slice(0, 300)}`);
  }

  const contentType = response.headers.get("content-type") || "";
  if (!contentType.includes("application/json")) {
    return {
      providerUsed: "novita",
      size: parsedSize.size,
      mime: contentType || "image/png",
      bytes: Buffer.from(await response.arrayBuffer()),
    };
  }

  const responsePayload = await response.json();
  const candidate =
    extractImageCandidateFromPayload(responsePayload)
    || extractImageCandidateFromPayload(responsePayload?.data)
    || extractImageCandidateFromPayload(responsePayload?.result);

  if (!candidate) {
    throw new Error("Novita image response did not include image content or image URL.");
  }

  if (candidate.url) {
    const downloaded = await downloadImageFromUrl(candidate.url);
    return {
      providerUsed: "novita",
      size: parsedSize.size,
      mime: downloaded.mime,
      bytes: downloaded.bytes,
      sourceUrl: candidate.url,
    };
  }

  return {
    providerUsed: "novita",
    size: parsedSize.size,
    mime: candidate.mime || "image/png",
    bytes: candidate.bytes,
  };
};

const generateMentorImage = async (args = {}) => {
  if (!mentorImageEnabled) {
    throw new Error("Mentor image generation is disabled (ENABLE_MENTOR_IMAGE=false).");
  }

  const providerUsed = resolveImageProvider(args.provider);
  const request = {
    prompt: args.prompt,
    size: args.size,
    negativePrompt: args.negativePrompt,
    style: args.style,
    seed: args.seed,
  };

  const generated = providerUsed === "novita"
    ? await callNovitaImageGeneration(request)
    : await callChutesImageGeneration(request);

  if (!generated?.bytes || generated.bytes.length === 0) {
    throw new Error("Image backend returned an empty image payload.");
  }

  await ensureDir(mentorImageArtifactDir);
  const extension = extensionForImageMimeType(generated.mime);
  const fileName = `mentor-image-${Date.now()}-${randomUUID()}.${extension}`;
  const outputPath = path.join(mentorImageArtifactDir, fileName);
  await writeFile(outputPath, generated.bytes);

  return {
    imageArtifact: outputPath,
    imageProvider: generated.providerUsed || providerUsed,
    imageMimeType: generated.mime || mimeTypeForImageExtension(extension),
    size: generated.size || parseImageSize(args.size).size,
    sourceUrl: generated.sourceUrl,
  };
};

const callChutesVideoGeneration = async ({ prompt, size, negativePrompt, durationSeconds, seed }) => {
  if (!mentorVideoApiKey) {
    throw new Error("MENTOR_VIDEO_API_KEY (or fallback image/voice/main key) is missing.");
  }

  const parsedSize = parseImageSize(size || mentorVideoSize);
  const requestInput = {
    prompt: normalizeImagePrompt(prompt),
    size: parsedSize.size,
    width: parsedSize.width,
    height: parsedSize.height,
    duration_seconds: Math.max(1, Math.min(30, durationSeconds || mentorVideoDurationSeconds)),
  };
  if (negativePrompt && negativePrompt.trim()) {
    requestInput.negative_prompt = negativePrompt.trim();
  }
  if (Number.isInteger(seed)) {
    requestInput.seed = seed;
  }

  let payload;
  if (mentorChutesVideoMode === "direct") {
    if (!mentorChutesVideoEndpoint) {
      throw new Error("MENTOR_CHUTES_VIDEO_ENDPOINT is required when MENTOR_CHUTES_VIDEO_MODE=direct.");
    }
    const response = await fetchWithTimeout(mentorChutesVideoEndpoint, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${mentorVideoApiKey}`,
      },
      body: JSON.stringify(requestInput),
    }, videoRequestTimeoutMs);
    if (!response.ok) {
      const errorBody = await response.text();
      throw new Error(`Chutes direct video failed (${response.status}): ${errorBody.slice(0, 300)}`);
    }

    const contentType = response.headers.get("content-type") || "";
    if (contentType.includes("application/json")) {
      payload = await response.json();
    } else {
      return {
        providerUsed: "chutes",
        size: parsedSize.size,
        mime: contentType || "video/mp4",
        bytes: Buffer.from(await response.arrayBuffer()),
      };
    }
  } else {
    const response = await fetchWithTimeout(mentorChutesVideoRunEndpoint, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${mentorVideoApiKey}`,
      },
      body: JSON.stringify({
        model: mentorChutesVideoModel,
        input: requestInput,
      }),
    }, videoRequestTimeoutMs);
    if (!response.ok) {
      const errorBody = await response.text();
      throw new Error(`Chutes video run request failed (${response.status}): ${errorBody.slice(0, 300)}`);
    }

    payload = await response.json();
  }

  const candidate =
    extractVideoCandidateFromPayload(payload)
    || extractVideoCandidateFromPayload(payload?.output)
    || extractVideoCandidateFromPayload(payload?.result);

  if (!candidate) {
    throw new Error("Chutes video response did not include video content or video URL.");
  }

  if (candidate.url) {
    const downloaded = await downloadVideoFromUrl(candidate.url);
    return {
      providerUsed: "chutes",
      size: parsedSize.size,
      mime: downloaded.mime,
      bytes: downloaded.bytes,
      sourceUrl: candidate.url,
    };
  }

  return {
    providerUsed: "chutes",
    size: parsedSize.size,
    mime: candidate.mime || "video/mp4",
    bytes: candidate.bytes,
  };
};

const callNovitaVideoGeneration = async ({ prompt, size, negativePrompt, durationSeconds, seed }) => {
  if (!mentorNovitaApiKey) {
    throw new Error("MENTOR_NOVITA_API_KEY is missing.");
  }

  const parsedSize = parseImageSize(size || mentorVideoSize);
  const payload = {
    prompt: normalizeImagePrompt(prompt),
    model_name: mentorNovitaVideoModel,
    width: parsedSize.width,
    height: parsedSize.height,
    duration_seconds: Math.max(1, Math.min(30, durationSeconds || mentorVideoDurationSeconds)),
  };
  if (negativePrompt && negativePrompt.trim()) {
    payload.negative_prompt = negativePrompt.trim();
  }
  if (Number.isInteger(seed)) {
    payload.seed = seed;
  }

  const response = await fetchWithTimeout(mentorNovitaVideoEndpoint, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${mentorNovitaApiKey}`,
    },
    body: JSON.stringify(payload),
  }, videoRequestTimeoutMs);

  if (!response.ok) {
    const errorBody = await response.text();
    throw new Error(`Novita video request failed (${response.status}): ${errorBody.slice(0, 300)}`);
  }

  const contentType = response.headers.get("content-type") || "";
  if (!contentType.includes("application/json")) {
    return {
      providerUsed: "novita",
      size: parsedSize.size,
      mime: contentType || "video/mp4",
      bytes: Buffer.from(await response.arrayBuffer()),
    };
  }

  const responsePayload = await response.json();
  const candidate =
    extractVideoCandidateFromPayload(responsePayload)
    || extractVideoCandidateFromPayload(responsePayload?.data)
    || extractVideoCandidateFromPayload(responsePayload?.result);

  if (!candidate) {
    throw new Error("Novita video response did not include video content or video URL.");
  }

  if (candidate.url) {
    const downloaded = await downloadVideoFromUrl(candidate.url);
    return {
      providerUsed: "novita",
      size: parsedSize.size,
      mime: downloaded.mime,
      bytes: downloaded.bytes,
      sourceUrl: candidate.url,
    };
  }

  return {
    providerUsed: "novita",
    size: parsedSize.size,
    mime: candidate.mime || "video/mp4",
    bytes: candidate.bytes,
  };
};

const generateMentorVideo = async (args = {}) => {
  if (!mentorVideoEnabled) {
    throw new Error("Mentor video generation is disabled (ENABLE_MENTOR_VIDEO=false).");
  }

  const providerUsed = resolveVideoProvider(args.provider);
  const request = {
    prompt: args.prompt,
    size: args.size,
    negativePrompt: args.negativePrompt,
    durationSeconds: args.durationSeconds,
    seed: args.seed,
  };

  const generated = providerUsed === "novita"
    ? await callNovitaVideoGeneration(request)
    : await callChutesVideoGeneration(request);

  if (!generated?.bytes || generated.bytes.length === 0) {
    throw new Error("Video backend returned an empty video payload.");
  }

  await ensureDir(mentorVideoArtifactDir);
  const extension = extensionForVideoMimeType(generated.mime);
  const fileName = `mentor-video-${Date.now()}-${randomUUID()}.${extension}`;
  const outputPath = path.join(mentorVideoArtifactDir, fileName);
  await writeFile(outputPath, generated.bytes);

  return {
    videoArtifact: outputPath,
    videoProvider: generated.providerUsed || providerUsed,
    videoMimeType: generated.mime || mimeTypeForVideoExtension(extension),
    size: generated.size || parseImageSize(args.size || mentorVideoSize).size,
    sourceUrl: generated.sourceUrl,
  };
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
    {
      role: "system",
      content: mentorReadOnlyPolicy,
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
    safety: {
      sandboxAccess: "read_only",
      actionExecution: false,
    },
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
    image: {
      enabled: mentorImageEnabled,
      provider: resolveImageProvider(),
      defaultProvider: mentorImageProvider,
      apiKeyConfigured:
        resolveImageProvider() === "novita"
          ? mentorNovitaApiKey.length > 0
          : mentorImageApiKey.length > 0,
      artifactDir: mentorImageArtifactDir,
      requestTimeoutMs: imageRequestTimeoutMs,
      maxPromptChars: mentorImageMaxPromptChars,
      defaultSize: mentorImageSize,
      defaultResponseFormat: mentorImageResponseFormat,
      chutes: {
        mode: mentorChutesImageMode,
        model: mentorChutesImageModel,
        runEndpoint: mentorChutesImageRunEndpoint,
        endpoint: mentorChutesImageEndpoint || null,
      },
      novita: {
        apiBaseUrl: mentorNovitaApiBaseUrl,
        endpoint: mentorNovitaImageEndpoint,
        model: mentorNovitaImageModel,
        responseFormat: mentorNovitaResponseFormat,
        apiKeyConfigured: mentorNovitaApiKey.length > 0,
      },
    },
    video: {
      enabled: mentorVideoEnabled,
      provider: resolveVideoProvider(),
      defaultProvider: mentorVideoProvider,
      apiKeyConfigured:
        resolveVideoProvider() === "novita"
          ? mentorNovitaApiKey.length > 0
          : mentorVideoApiKey.length > 0,
      artifactDir: mentorVideoArtifactDir,
      requestTimeoutMs: videoRequestTimeoutMs,
      defaultSize: mentorVideoSize,
      defaultDurationSeconds: mentorVideoDurationSeconds,
      chutes: {
        mode: mentorChutesVideoMode,
        model: mentorChutesVideoModel,
        runEndpoint: mentorChutesVideoRunEndpoint,
        endpoint: mentorChutesVideoEndpoint || null,
      },
      novita: {
        apiBaseUrl: mentorNovitaApiBaseUrl,
        endpoint: mentorNovitaVideoEndpoint,
        model: mentorNovitaVideoModel,
        apiKeyConfigured: mentorNovitaApiKey.length > 0,
      },
    },
    memory: {
      file: mentorMemoryFile,
      window: mentorMemoryWindow,
    },
    personaLoaded: persona.length > 0,
  };
};

const handleMentorImage = async (args) => {
  const parsed = imageArgsSchema.parse(args || {});
  const result = await generateMentorImage(parsed);
  return {
    mentor: mentorName,
    prompt: normalizeImagePrompt(parsed.prompt),
    ...result,
  };
};

const handleMentorVideo = async (args) => {
  const parsed = videoArgsSchema.parse(args || {});
  const result = await generateMentorVideo(parsed);
  return {
    mentor: mentorName,
    prompt: normalizeImagePrompt(parsed.prompt),
    ...result,
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
    imageEnabled: mentorImageEnabled,
    imageProvider: resolveImageProvider(),
    videoEnabled: mentorVideoEnabled,
    videoProvider: resolveVideoProvider(),
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
      } else if (name === "mentor.image") {
        result = await handleMentorImage(toolArgs);
      } else if (name === "mentor.video") {
        result = await handleMentorVideo(toolArgs);
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
      mentorImageEnabled,
      mentorImageProvider: resolveImageProvider(),
      mentorChutesImageMode,
      mentorChutesImageModel,
      mentorNovitaImageModel,
      mentorVideoEnabled,
      mentorVideoProvider: resolveVideoProvider(),
      mentorChutesVideoMode,
      mentorChutesVideoModel,
      mentorNovitaVideoModel,
    },
    "mentor-mcp started",
  );
});
