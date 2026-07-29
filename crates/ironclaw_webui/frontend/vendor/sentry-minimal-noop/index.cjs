/**
 * No-op replacement for `@sentry/minimal` (attested-signing Phase D).
 *
 * `@ledgerhq/device-transport-kit-web-hid` calls `captureException` on every
 * device error path — getDevices, promptDeviceAccess, startDiscovering,
 * updateTransportDiscoveredDevices. In `@sentry/minimal` those calls delegate
 * to whatever Sentry hub is current, so the transport carries a live route out
 * of the process that activates the moment anything in the page initializes
 * Sentry.
 *
 * We do not want that route to exist at all in a transaction-signing bundle.
 * The exceptions in question carry device and connection context, and the
 * signing path is the last place to leave an implicit "phone home" that a
 * future unrelated dependency could switch on. Errors are not lost: the
 * ceremony surfaces every device failure as its own outcome, which is where
 * they belong.
 *
 * Only `captureException` is currently reached (verified against the shipped
 * bundles). The rest of the v6 surface is stubbed so a future Ledger version
 * calling one cannot throw a TypeError inside the ceremony.
 */

const noop = () => undefined;
/** Sentry returns an event id string; callers may use it as one. */
const noopEventId = () => "";

const captureException = noopEventId;
const captureMessage = noopEventId;
const captureEvent = noopEventId;
const addBreadcrumb = noop;
const configureScope = noop;
const withScope = (callback) =>
  typeof callback === "function" ? callback(scopeStub) : undefined;
const setContext = noop;
const setExtra = noop;
const setExtras = noop;
const setTag = noop;
const setTags = noop;
const setUser = noop;
const startTransaction = () => undefined;
const getCurrentHub = () => hubStub;

/** A scope that accepts every mutation and keeps none. */
const scopeStub = {
  setTag: noop,
  setTags: noop,
  setExtra: noop,
  setExtras: noop,
  setUser: noop,
  setContext: noop,
  setLevel: noop,
  setFingerprint: noop,
  addBreadcrumb: noop,
  clear: noop,
};

const hubStub = {
  captureException: noopEventId,
  captureMessage: noopEventId,
  captureEvent: noopEventId,
  addBreadcrumb: noop,
  configureScope: noop,
  withScope,
  getScope: () => scopeStub,
  getClient: () => undefined,
};



module.exports = { captureException, captureMessage, captureEvent, addBreadcrumb, configureScope, withScope, setContext, setExtra, setExtras, setTag, setTags, setUser, startTransaction, getCurrentHub };
