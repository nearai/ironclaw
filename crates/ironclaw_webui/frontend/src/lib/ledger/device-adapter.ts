// @ts-nocheck
/**
 * Where a real device port comes from (attested-signing §D4).
 *
 * The ceremony is written against the device port; this is the one place that
 * decides whether a real one can be built. Today it cannot: the
 * `@ledgerhq/device-management-kit` + `device-transport-kit-web-hid` adapter is
 * not wired yet, pending dependency approval. Until it is, this returns `null`
 * and the ceremony reports `unsupported` — a visible, fail-closed state rather
 * than a button that appears to work.
 *
 * `null` is also the correct answer for a permanent, non-transitional reason:
 * **WebHID exists only in Chromium browsers.** A user on Safari or Firefox will
 * always land here, so this is not scaffolding to be deleted when the adapter
 * arrives — it stays as the browser-support check, and the adapter slots in
 * behind it.
 */

/** Whether this browser can talk to a USB HID device at all. */
export function webHidSupported(navigatorLike = globalThis.navigator) {
  return Boolean(navigatorLike && navigatorLike.hid);
}

/**
 * Build a device port, or `null` when one cannot exist here.
 *
 * Returning `null` rather than throwing keeps the caller on the ordinary
 * outcome path: a browser without WebHID is an expected condition to render,
 * not an exception to catch.
 *
 * The DMK adapter is imported dynamically so its weight lives in the review
 * route's chunk and never loads for ordinary chat users. A failure to load it
 * is also `null` — a bundle that did not arrive is not a reason to pretend a
 * device is reachable.
 */
export async function createDevicePort(navigatorLike = globalThis.navigator, options = {}) {
  if (!webHidSupported(navigatorLike)) {
    return null;
  }
  try {
    const { createDmkDevicePort } = await import("./dmk-adapter");
    return await createDmkDevicePort(options);
  } catch (_) {
    return null;
  }
}
