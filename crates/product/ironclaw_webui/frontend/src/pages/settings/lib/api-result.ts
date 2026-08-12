// Treat a resolved API response carrying `success: false` as a failed request
// so a react-query `mutationFn` rejects instead of running its `onSuccess`.
// Without this, a stub/handler that resolves `{ success: false }` would still
// flip the optimistic cache and show a fake "Saved" indicator. Returns the
// data unchanged on success so callers can keep chaining.
export function throwIfApiFailed(data, fallbackMessage = "Request failed") {
  if (data && data.success === false) {
    throw new Error(data.message || fallbackMessage);
  }
  return data;
}
