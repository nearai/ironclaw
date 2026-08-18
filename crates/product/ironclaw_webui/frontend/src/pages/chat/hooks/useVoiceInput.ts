import React from "react";
import { useQuery } from "@tanstack/react-query";

import { fetchSession, readStoredToken } from "../../../lib/api";
import { browserSupportsVoiceCapture, voiceLimitsFromSession } from "../lib/voice";

/**
 * The composer's view of the voice contract: whether this deployment can
 * transcribe at all, and the limits the recorder must respect.
 *
 * Reads the shared `["session"]` query (deduped with the auth-layer fetch), so
 * the server's `features.voice_input` gate and its `voice` budget arrive
 * together with everything else the composer learns at bootstrap.
 */
export function useVoiceConfig() {
  const token = readStoredToken();
  const query = useQuery({
    enabled: Boolean(token),
    queryKey: ["session"],
    queryFn: fetchSession,
    staleTime: 5 * 60_000,
  });
  const limits = voiceLimitsFromSession(query.data);
  // Two independent gates, both required: the deployment must have resolved a
  // transcription backend, and this browser must be able to record (secure
  // context + MediaRecorder). Either one failing means no microphone button
  // rather than a button that cannot work.
  const enabled = Boolean(query.data?.features?.voice_input) && browserSupportsVoiceCapture();
  return { enabled, limits };
}

/**
 * Record a voice clip and transcribe it.
 *
 * States: `idle` → `recording` → `transcribing` → `idle`. `stop()` produces a
 * transcript; `cancel()` discards without uploading, the escape hatch for a
 * mis-started recording.
 *
 * This hook is deliberately thin — state, refs, and lifecycle only. The engine
 * that touches `MediaRecorder` is dynamically imported on first use, so a
 * session that never dictates never loads it (see `lib/voice-recorder.ts` and
 * `scripts/check-bundle-budgets.ts`). The hook itself must stay eager because
 * hooks cannot be called conditionally.
 *
 * The transcript goes to `onTranscript` and is never auto-sent: it lands in the
 * composer as editable text, so a mis-transcription is always correctable.
 */
export function useVoiceInput({ limits, onTranscript, onError }) {
  const [status, setStatus] = React.useState("idle");
  const [elapsedSecs, setElapsedSecs] = React.useState(0);

  const handleRef = React.useRef(null);
  // Bumped by every cancel and by unmount. `start` captures it before awaiting
  // the engine chunk and re-checks after: without this, cancelling during that
  // load would still grab the microphone when the import resolved.
  const generationRef = React.useRef(0);
  // Guards every callback that can fire after the engine's async tail: a
  // composer unmounted mid-recording must discard the clip, not upload it and
  // set state nobody is looking at.
  const mountedRef = React.useRef(true);
  // Read inside callbacks that outlive the render that created them, so a
  // mid-recording refresh does not strand a stale value.
  const limitsRef = React.useRef(limits);
  limitsRef.current = limits;
  const onTranscriptRef = React.useRef(onTranscript);
  onTranscriptRef.current = onTranscript;
  const onErrorRef = React.useRef(onError);
  onErrorRef.current = onError;

  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      generationRef.current += 1;
      // Cancel rather than stop: an unmounted composer has nowhere to put a
      // transcript, and cancelling also releases the microphone tracks.
      handleRef.current?.cancel();
      handleRef.current = null;
    };
  }, []);

  const settle = React.useCallback((result) => {
    handleRef.current = null;
    if (!mountedRef.current) return;
    setStatus("idle");
    if (result.ok) {
      onTranscriptRef.current?.(result.text);
      return;
    }
    // A deliberate cancel is not a failure and gets no error notice.
    if (result.reason === "cancelled") return;
    onErrorRef.current?.({ reason: result.reason, detail: result.detail });
  }, []);

  const start = React.useCallback(async () => {
    if (status !== "idle" || handleRef.current) return;
    const generation = generationRef.current;
    setStatus("recording");
    setElapsedSecs(0);

    const { startVoiceRecording } = await import("../lib/voice-recorder");
    // Cancelled or unmounted while the engine chunk was loading: do not grab
    // the microphone now that it has arrived.
    if (!mountedRef.current || generationRef.current !== generation) return;

    const handle = await startVoiceRecording({
      limits: limitsRef.current,
      onTick: (seconds) => {
        if (mountedRef.current) setElapsedSecs(seconds);
      },
      onTranscribing: () => {
        if (mountedRef.current) setStatus("transcribing");
      },
      onSettled: settle,
    });
    // A cancel can also land during `getUserMedia`, which is itself a prompt
    // the user may dismiss slowly; tear down rather than keep a live recorder
    // nothing is showing.
    if (!mountedRef.current || generationRef.current !== generation) {
      handle?.cancel();
      return;
    }
    // `null` means the engine already reported its failure through `onSettled`
    // (no permission, no supported container) and there is nothing to hold.
    handleRef.current = handle;
  }, [settle, status]);

  const stop = React.useCallback(() => {
    handleRef.current?.stop();
  }, []);

  const cancel = React.useCallback(() => {
    // Bump first: a cancel pressed while the engine chunk (or the permission
    // prompt) is still resolving has no handle to cancel, and the generation
    // check in `start` is what stops that pending recording from beginning.
    generationRef.current += 1;
    const handle = handleRef.current;
    handleRef.current = null;
    if (handle) {
      handle.cancel();
      return;
    }
    setStatus("idle");
  }, []);

  return { status, elapsedSecs, start, stop, cancel };
}
