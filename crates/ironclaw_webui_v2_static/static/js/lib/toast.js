/* Minimal toast bus: any module can call `toast(...)` without prop/context
   plumbing; <ToastViewport> (mounted once in the layout) renders them. */
const listeners = new Set();
let seq = 0;

export function toast(message, opts = {}) {
  const item = {
    id: ++seq,
    message,
    tone: opts.tone || "info",
    duration: opts.duration ?? 2600,
  };
  listeners.forEach((listener) => listener(item));
  return item.id;
}

export function subscribeToasts(listener) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
