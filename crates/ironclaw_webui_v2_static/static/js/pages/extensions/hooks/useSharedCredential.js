import { useMutation } from "@tanstack/react-query";
import { setSharedCredential } from "../lib/extensions-api.js";

// Operator-only mutation for setting a tenant-shared credential (#5459 P3).
//
// Wraps the write-only POST in the same react-query shape the extensions
// surface uses. The raw secret lives only in the caller's local form state and
// in the request body — this hook never stores it, so there is no client-side
// persistence of the value beyond the in-flight request. The resolved data
// carries only the confirmed handle.
//
// `apiFetch` rejects with an `ApiError` (carrying `.status`) on a non-2xx
// response, so the caller can distinguish a 403 (non-admin) from other
// failures. A 200 that still reports `success: false` is treated as a failed
// save so the caller never shows a fake confirmation for a write that did not
// persist.
export function useSharedCredential() {
  const mutation = useMutation({
    mutationFn: async ({ handle, value }) => {
      const result = await setSharedCredential({ handle, value });
      if (result.success === false) {
        throw new Error("Save failed");
      }
      return result;
    },
  });

  // The section owns its own saved/error state, so only the mutation trigger
  // and the in-flight flag are exposed. `setCredential` rejects with the
  // `ApiError` (carrying `.status`) on failure so the caller can branch on 403.
  return {
    setCredential: (handle, value) => mutation.mutateAsync({ handle, value }),
    isSaving: mutation.isPending,
  };
}
