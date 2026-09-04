import { apiFetch, type ApiRecord } from "../../../lib/api";

export interface TraceCreditsResponse extends ApiRecord {
  enrolled: boolean;
  submissions_total: number;
  submissions_submitted: number;
  submissions_accepted: number;
  manual_review_hold_count: number;
  recent_explanations: string[];
  holds: TraceHoldResponse[];
}

interface TraceHoldResponse extends ApiRecord {
  submission_id: string;
  reason: string;
}

interface AccountTraceResponse extends ApiRecord {
  submission_id: string;
  status: string;
}

function invalidTrace(responseName: string): never {
  throw new TypeError(`invalid ${responseName} response`);
}

function arrayField(
  response: ApiRecord,
  field: string,
  responseName: string,
): unknown[] {
  const value = response[field];
  return Array.isArray(value) ? value : invalidTrace(responseName);
}

function optionalArrayField(
  response: ApiRecord,
  field: string,
  responseName: string,
): unknown[] {
  return response[field] === undefined
    ? []
    : arrayField(response, field, responseName);
}

function numberField(response: ApiRecord, field: string): number {
  const value = response[field];
  return typeof value === "number" ? value : invalidTrace("trace credits");
}

function decodeTraceHolds(value: unknown[]): TraceHoldResponse[] {
  return value.map((entry) => {
    if (
      typeof entry !== "object" ||
      entry === null ||
      Array.isArray(entry) ||
      !("submission_id" in entry) ||
      typeof entry.submission_id !== "string" ||
      !("reason" in entry) ||
      typeof entry.reason !== "string"
    ) {
      return invalidTrace("trace credits");
    }
    return entry as TraceHoldResponse;
  });
}

// Trace Commons credits are read-only and scoped server-side to the
// authenticated caller. Serde-defaulted collections may be omitted on the
// wire, but present malformed values remain boundary errors.
export async function fetchTraceCredits(): Promise<TraceCreditsResponse> {
  const response = await apiFetch("/api/webchat/v2/traces/credit");
  if (typeof response.enrolled !== "boolean") invalidTrace("trace credits");
  const recentExplanations = optionalArrayField(
    response,
    "recent_explanations",
    "trace credits",
  );
  if (!recentExplanations.every((line) => typeof line === "string")) {
    invalidTrace("trace credits");
  }
  return {
    ...response,
    enrolled: response.enrolled,
    submissions_total: numberField(response, "submissions_total"),
    submissions_submitted: numberField(response, "submissions_submitted"),
    submissions_accepted: numberField(response, "submissions_accepted"),
    manual_review_hold_count: numberField(response, "manual_review_hold_count"),
    recent_explanations: recentExplanations as string[],
    holds: decodeTraceHolds(
      optionalArrayField(response, "holds", "trace credits"),
    ),
  };
}

export async function fetchAccountTraces() {
  const response = await apiFetch("/api/webchat/v2/traces/account");
  if (typeof response.enrolled !== "boolean") invalidTrace("account traces");
  const traces = arrayField(response, "traces", "account traces").map(
    (trace): AccountTraceResponse => {
      if (
        typeof trace !== "object" ||
        trace === null ||
        Array.isArray(trace) ||
        !("submission_id" in trace) ||
        typeof trace.submission_id !== "string" ||
        !("status" in trace) ||
        typeof trace.status !== "string"
      ) {
        return invalidTrace("account traces");
      }
      return trace as AccountTraceResponse;
    },
  );
  return { ...response, enrolled: response.enrolled, traces };
}

export function mintAccountLoginLink() {
  return apiFetch("/api/webchat/v2/traces/account-login-link", { method: "POST" });
}

export function authorizeTraceHold(submissionId) {
  return apiFetch(
    `/api/webchat/v2/traces/holds/${encodeURIComponent(submissionId)}/authorize`,
    { method: "POST" },
  );
}
