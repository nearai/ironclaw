// @ts-nocheck
// Lazy facade for AttachmentPreviewModal.
//
// The preview modal (and the DS Modal / Radix Dialog stack under it) is
// only needed after the user clicks an attachment, but message-bubble
// sits in the chat route's initial import graph — a static import would
// charge every chat load ~13KB gzip for a dialog most sessions never
// open (see scripts/check-bundle-budgets.ts). The facade renders nothing
// until an attachment is selected, then lazy-loads the real modal.
import React from "react";

const AttachmentPreviewModalImpl = React.lazy(() =>
  import("./attachment-preview").then((mod) => ({
    default: mod.AttachmentPreviewModal,
  }))
);

export function AttachmentPreviewModal(props) {
  if (props.attachment == null) return null;
  return (
    <React.Suspense fallback={null}>
      <AttachmentPreviewModalImpl {...props} />
    </React.Suspense>
  );
}
