// @ts-nocheck
// Lazy facade for the KeyboardShortcuts dialog.
//
// The dialog sits on DS Modal (Radix Dialog + react-remove-scroll), and
// chat renders it on every load even though it only appears after the
// user presses "?". Loading it on first open keeps the dialog stack out
// of the chat route's initial JS (see scripts/check-bundle-budgets.ts).
import React from "react";

const KeyboardShortcutsImpl = React.lazy(() =>
  import("./keyboard-shortcuts").then((mod) => ({
    default: mod.KeyboardShortcuts,
  }))
);

export function KeyboardShortcuts(props) {
  if (!props.open) return null;
  return (
    <React.Suspense fallback={null}>
      <KeyboardShortcutsImpl {...props} />
    </React.Suspense>
  );
}
