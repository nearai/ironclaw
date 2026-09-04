// Treat a resolved API response carrying `success: false` as a failed request
// so a react-query `mutationFn` rejects instead of running its `onSuccess`.
// Without this, a stub/handler that resolves `{ success: false }` would still
// flip the optimistic cache and show a fake "Saved" indicator. Returns the
// data unchanged on success so callers can keep chaining.
type ApiFailure = { success: false; message?: string };

export function throwIfApiFailed<T>(
  data: T,
  fallbackMessage = "Request failed",
): Exclude<T, ApiFailure> {
  if (
    typeof data === "object" &&
    data !== null &&
    Reflect.get(data, "success") === false
  ) {
    const message = Reflect.get(data, "message");
    throw new Error(typeof message === "string" ? message : fallbackMessage);
  }
  return data as Exclude<T, ApiFailure>;
}
