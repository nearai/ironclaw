// Turn a recorded clip into 16 kHz mono WAV.
//
// Why this exists: `MediaRecorder` writes `audio/webm` (Chrome/Firefox) or
// `audio/mp4` (Safari), and the transcription endpoint decodes neither —
// measured 2026-08-17 against NEAR AI's `/v1/audio/transcriptions`, which
// accepts wav/ogg/mp3/flac and answers HTTP 400 "supported format" for webm
// and mp4. There is no container both browsers record that it accepts, so the
// plan's preferred fix (pick one common `mimeType`) is not available.
//
// Converting in the browser is cheap here because the browser already owns a
// decoder for the container it just wrote: `decodeAudioData` handles webm/opus
// and mp4/aac natively. All we add is a downmix, a resample, and a WAV header.
// 16 kHz mono is not a quality compromise for speech — it is the rate Whisper
// models operate at — and it keeps the upload ~10x smaller than 48 kHz stereo.
//
// This module is only reachable from `voice-recorder.ts`, which is itself
// loaded on demand, so none of it is in the initial /chat bundle.

/** Sample rate every clip is resampled to. Whisper's own working rate. */
export const TARGET_SAMPLE_RATE = 16000;

/**
 * Downmix an AudioBuffer's channels to a single mono track.
 *
 * Averaging (rather than taking channel 0) keeps a speaker who happens to sit
 * on one side of a stereo capture from being halved in level.
 */
export function downmixToMono(channels, length) {
  if (channels.length === 1) return channels[0];
  const mono = new Float32Array(length);
  for (let i = 0; i < length; i += 1) {
    let sum = 0;
    for (let c = 0; c < channels.length; c += 1) sum += channels[c][i] || 0;
    mono[i] = sum / channels.length;
  }
  return mono;
}

/**
 * Linear-interpolation resample to `targetRate`.
 *
 * Deliberately not an `OfflineAudioContext` render: Safari restricted
 * OfflineAudioContext to a few sample rates for years, and silently getting a
 * 44.1 kHz buffer back when you asked for 16 kHz would ship a subtly wrong
 * upload. Linear interpolation is more than adequate for speech headed to a
 * transcription model, and it behaves identically everywhere.
 */
export function resampleTo(samples, sourceRate, targetRate = TARGET_SAMPLE_RATE) {
  if (!(sourceRate > 0) || sourceRate === targetRate) return samples;
  const ratio = sourceRate / targetRate;
  const outLength = Math.max(1, Math.floor(samples.length / ratio));
  const out = new Float32Array(outLength);
  for (let i = 0; i < outLength; i += 1) {
    const position = i * ratio;
    const left = Math.floor(position);
    const right = Math.min(left + 1, samples.length - 1);
    const weight = position - left;
    out[i] = samples[left] * (1 - weight) + samples[right] * weight;
  }
  return out;
}

/**
 * Write mono float samples as a 16-bit PCM WAV file.
 *
 * Standard 44-byte canonical header; samples are clamped before scaling so a
 * decoder that hands back values slightly outside [-1, 1] cannot wrap around
 * into loud noise.
 */
export function encodeWav(samples, sampleRate = TARGET_SAMPLE_RATE) {
  const bytesPerSample = 2;
  const buffer = new ArrayBuffer(44 + samples.length * bytesPerSample);
  const view = new DataView(buffer);

  const writeAscii = (offset, text) => {
    for (let i = 0; i < text.length; i += 1) view.setUint8(offset + i, text.charCodeAt(i));
  };

  const dataBytes = samples.length * bytesPerSample;
  writeAscii(0, "RIFF");
  view.setUint32(4, 36 + dataBytes, true);
  writeAscii(8, "WAVE");
  writeAscii(12, "fmt ");
  view.setUint32(16, 16, true); // PCM chunk size
  view.setUint16(20, 1, true); // format: PCM
  view.setUint16(22, 1, true); // channels: mono
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * bytesPerSample, true); // byte rate
  view.setUint16(32, bytesPerSample, true); // block align
  view.setUint16(34, 16, true); // bits per sample
  writeAscii(36, "data");
  view.setUint32(40, dataBytes, true);

  let offset = 44;
  for (let i = 0; i < samples.length; i += 1) {
    const clamped = Math.max(-1, Math.min(1, samples[i]));
    // Asymmetric scaling matches the 16-bit range: -32768..32767.
    view.setInt16(offset, clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff, true);
    offset += 2;
  }
  return new Blob([buffer], { type: "audio/wav" });
}

/**
 * Decode a recorded clip and re-encode it as 16 kHz mono WAV.
 *
 * Throws if the browser cannot decode its own recording, which the caller
 * surfaces as a retryable recording error rather than uploading something the
 * endpoint would reject anyway.
 */
export async function clipToWav(blob) {
  const AudioContextCtor = globalThis.AudioContext || globalThis.webkitAudioContext;
  if (typeof AudioContextCtor !== "function") {
    throw new Error("this browser cannot decode recorded audio");
  }
  const context = new AudioContextCtor();
  try {
    const arrayBuffer = await blob.arrayBuffer();
    // Promise form; Safari also supports it (the callback form is only needed
    // for very old WebKit, which fails the capability probe already).
    const decoded = await context.decodeAudioData(arrayBuffer);
    const channels = [];
    for (let c = 0; c < decoded.numberOfChannels; c += 1) {
      channels.push(decoded.getChannelData(c));
    }
    const mono = downmixToMono(channels, decoded.length);
    const resampled = resampleTo(mono, decoded.sampleRate, TARGET_SAMPLE_RATE);
    return encodeWav(resampled, TARGET_SAMPLE_RATE);
  } finally {
    // Release the hardware context; leaking one per recording eventually trips
    // the browser's per-page AudioContext cap.
    if (typeof context.close === "function") await context.close().catch(() => {});
  }
}
