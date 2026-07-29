// @ts-nocheck
// Lazy facade for PairingWebCodePanel.
//
// The panel drags the `qrcode` encoder (~10KB gzip) with it, and the chat
// route only renders it during a `web_generated_code` onboarding gate —
// a rare state that shouldn't tax every chat load (see
// scripts/check-bundle-budgets.ts). The extensions page keeps the static
// import; this facade is for always-loaded chat surfaces.
import React from "react";

const PairingWebCodePanelImpl = React.lazy(() =>
  import("./pairing-web-code-panel").then((mod) => ({
    default: mod.PairingWebCodePanel,
  }))
);

export function PairingWebCodePanel(props) {
  return (
    <React.Suspense fallback={null}>
      <PairingWebCodePanelImpl {...props} />
    </React.Suspense>
  );
}
