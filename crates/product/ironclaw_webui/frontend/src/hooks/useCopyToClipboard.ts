// @ts-nocheck
import React from "react";

const COPIED_RESET_MS = 1500;

export function useCopyToClipboard(resetMs = COPIED_RESET_MS) {
  const [copied, setCopied] = React.useState(false);
  const timerRef = React.useRef(null);

  React.useEffect(() => () => clearTimeout(timerRef.current), []);

  const copy = React.useCallback(
    async (text) => {
      const clipboard = typeof navigator === "undefined" ? null : navigator.clipboard;
      if (!clipboard?.writeText || !text) return false;

      try {
        await clipboard.writeText(text);
      } catch (_) {
        return false;
      }

      setCopied(true);
      clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => setCopied(false), resetMs);
      return true;
    },
    [resetMs],
  );

  return { copied, copy };
}
