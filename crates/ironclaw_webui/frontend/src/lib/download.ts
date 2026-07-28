// Trigger a browser "save as" for an in-memory Blob.
//
// Canonical home for the object-URL + transient-anchor dance so data-fetching
// layers (e.g. `lib/api.ts`) stay free of DOM side effects and call sites do
// not re-implement (and drift on) the revoke/cleanup steps.
export function saveBlob(blob, filename) {
  const url = URL.createObjectURL(blob);
  try {
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = filename || "download";
    document.body.appendChild(anchor);
    anchor.click();
    anchor.remove();
    // Defer revocation: revoking synchronously after click() races the
    // browser's async download manager and breaks downloads in Chrome/
    // Firefox/Safari (the URL is freed before the blob is resolved).
    setTimeout(() => URL.revokeObjectURL(url), 100);
  } catch (e) {
    URL.revokeObjectURL(url);
    throw e;
  }
}
