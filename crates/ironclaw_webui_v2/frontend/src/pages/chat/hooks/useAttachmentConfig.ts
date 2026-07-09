import { useQuery } from "@tanstack/react-query";
import { fetchSession, readStoredToken } from "../../../lib/api";
import { FALLBACK_ATTACHMENT_LIMITS } from "../lib/attachments";

// Map the server's `session.attachments` contract (snake_case wire shape from
// `WebUiAttachmentCapabilities`) into the camelCase limits `stageFiles`
// consumes. Falls back to the conservative client defaults until the session
// resolves; the server re-validates regardless, so the fallback only changes
// how early the picker warns.
export function attachmentLimitsFromSession(session) {
  const a = session?.attachments;
  if (!a) return FALLBACK_ATTACHMENT_LIMITS;
  return {
    // Keep only string tokens: a non-string would throw in `isAcceptedFile`'s
    // `token.trim()` and break staging for the whole picker.
    accept: Array.isArray(a.accept)
      ? a.accept.filter((token) => typeof token === "string")
      : FALLBACK_ATTACHMENT_LIMITS.accept,
    maxCount: Number.isFinite(a.max_count)
      ? a.max_count
      : FALLBACK_ATTACHMENT_LIMITS.maxCount,
    maxFileBytes: Number.isFinite(a.max_file_bytes)
      ? a.max_file_bytes
      : FALLBACK_ATTACHMENT_LIMITS.maxFileBytes,
    maxTotalBytes: Number.isFinite(a.max_total_bytes)
      ? a.max_total_bytes
      : FALLBACK_ATTACHMENT_LIMITS.maxTotalBytes,
  };
}

// The composer's view of the inline-attachment contract. Reads the shared
// `["session"]` query (deduped with the auth-layer fetch) so the file picker's
// `accept` set and size budgets come straight from the server registry.
export function useAttachmentConfig() {
  const token = readStoredToken();
  const query = useQuery({
    enabled: Boolean(token),
    queryKey: ["session"],
    queryFn: fetchSession,
    staleTime: 5 * 60_000,
  });
  return attachmentLimitsFromSession(query.data);
}
