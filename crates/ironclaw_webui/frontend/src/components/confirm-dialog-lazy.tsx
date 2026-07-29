// @ts-nocheck
// Lazy facade for the design-system ConfirmDialog.
//
// Used by always-mounted chrome (sidebar thread rows) that sits in the
// initial import graph of every route: a static import would pull the DS
// Modal / Radix Dialog stack into the entry bundle for a dialog that only
// appears on a destructive action (see scripts/check-bundle-budgets.ts).
// Renders nothing until `open`; the exit animation is skipped because the
// facade unmounts with `open`, which is acceptable for this rare path.
import React from "react";

const ConfirmDialogImpl = React.lazy(() =>
  // Deep subpath import (NOT the barrel): dynamically importing the
  // barrel makes every export reachable, which defeats tree-shaking for
  // the whole package and inflates the shared chunks.
  import("@ironclaw/design-system/confirm-dialog").then((mod) => ({
    default: mod.ConfirmDialog,
  }))
);

export function ConfirmDialog(props) {
  if (!props.open) return null;
  return (
    <React.Suspense fallback={null}>
      <ConfirmDialogImpl {...props} />
    </React.Suspense>
  );
}
