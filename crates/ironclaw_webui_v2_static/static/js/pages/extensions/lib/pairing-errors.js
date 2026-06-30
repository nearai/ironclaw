export function pairingErrorMessage(error, fallback) {
  return error?.payload?.error || error?.payload?.message || error?.message || fallback;
}
