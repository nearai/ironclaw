import { authScope } from "../../../lib/auth-scope";
import type { BoundedDiagnosticText } from "./inspector-activity";

export interface DiagnosticMetricTotal {
  known_total: number;
  unavailable_samples: number;
}

export interface SessionDiagnosticStats {
  total_model_calls: number;
  total_tool_calls?: number;
  successful_tool_calls?: number;
  failed_tool_calls?: number;
  calls_per_model: Array<{ model: BoundedDiagnosticText; calls: number }>;
  calls_per_model_truncated: boolean;
  input_tokens: DiagnosticMetricTotal;
  output_tokens: DiagnosticMetricTotal;
  cache_read_input_tokens: DiagnosticMetricTotal;
  cache_creation_input_tokens: DiagnosticMetricTotal;
  total_latency_ms: DiagnosticMetricTotal;
}

/**
 * Per-run breakdown ceiling, mirroring `MAX_MODELS_IN_STATS` in
 * `ironclaw_product_contracts::inspector`. The host truncates to this length
 * and reports `calls_per_model_truncated`, so a longer array cannot be a
 * contract-conforming response.
 */
const MAX_STATS_MODELS = 64;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isCount(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function isMetricTotal(value: unknown): value is DiagnosticMetricTotal {
  return isRecord(value)
    && isCount(value.known_total)
    && isCount(value.unavailable_samples);
}

function isBoundedDiagnosticText(value: unknown): value is BoundedDiagnosticText {
  return isRecord(value)
    && typeof value.content === "string"
    && isCount(value.original_bytes)
    && typeof value.truncated === "boolean";
}

function isCallsPerModelEntry(value: unknown): boolean {
  return isRecord(value)
    && isBoundedDiagnosticText(value.model)
    && isCount(value.calls);
}

/**
 * Accept only a complete authoritative statistics record.
 *
 * A partial or empty `stats` object would otherwise accumulate as real zeros
 * and the Stats tab would report "0 model calls" where the host actually
 * reported nothing. The inspector contract requires an explicit unavailable
 * state instead of a fabricated zero, so an undecodable record keeps
 * `sessionStats` null rather than seeding the accumulator.
 */
export function decodeSessionDiagnosticStats(
  value: unknown,
): SessionDiagnosticStats | null {
  if (!isRecord(value)) return null;
  if (!isCount(value.total_model_calls)) return null;
  if (typeof value.calls_per_model_truncated !== "boolean") return null;
  // Bound the breakdown before scanning or retaining it. The host truncates to
  // MAX_STATS_MODELS and sets `calls_per_model_truncated`, so a longer array is
  // outside the contract by construction — reject it rather than scan every
  // entry and hand the oversized array to `record`, which retains the record
  // for up to MAX_ACTIVE_RUN_STATS runs.
  //
  // Then every entry, not just the array: a negative or non-integer `calls`
  // would otherwise survive decoding and be coerced to 0 during accumulation
  // without marking the breakdown truncated, presenting a fabricated "0 calls"
  // for a model the host never reported that way.
  if (
    !Array.isArray(value.calls_per_model)
    || value.calls_per_model.length > MAX_STATS_MODELS
    || !value.calls_per_model.every(isCallsPerModelEntry)
  ) {
    return null;
  }
  if (
    !isMetricTotal(value.input_tokens)
    || !isMetricTotal(value.output_tokens)
    || !isMetricTotal(value.cache_read_input_tokens)
    || !isMetricTotal(value.cache_creation_input_tokens)
    || !isMetricTotal(value.total_latency_ms)
  ) {
    return null;
  }
  // Optional tool aggregates: an older host may omit them entirely, but a
  // present field must still be a real count rather than a coerced zero.
  for (const field of [
    "total_tool_calls",
    "successful_tool_calls",
    "failed_tool_calls",
  ] as const) {
    if (value[field] !== undefined && !isCount(value[field])) return null;
  }
  return value as unknown as SessionDiagnosticStats;
}

const MAX_ACTIVE_RUN_STATS = 128;
const MAX_RETIRED_RUN_IDS = 1_024;
const MAX_SESSION_MODELS = 64;

function safeCount(value: unknown): number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0
    ? value
    : 0;
}

function add(left: number, right: number): number {
  const total = safeCount(left) + safeCount(right);
  return Math.min(total, Number.MAX_SAFE_INTEGER);
}

function emptyMetric(): DiagnosticMetricTotal {
  return { known_total: 0, unavailable_samples: 0 };
}

function emptyStats(): SessionDiagnosticStats {
  return {
    total_model_calls: 0,
    total_tool_calls: 0,
    successful_tool_calls: 0,
    failed_tool_calls: 0,
    calls_per_model: [],
    calls_per_model_truncated: false,
    input_tokens: emptyMetric(),
    output_tokens: emptyMetric(),
    cache_read_input_tokens: emptyMetric(),
    cache_creation_input_tokens: emptyMetric(),
    total_latency_ms: emptyMetric(),
  };
}

function mergeMetric(
  target: DiagnosticMetricTotal,
  source: DiagnosticMetricTotal | null | undefined,
): void {
  target.known_total = add(target.known_total, source?.known_total);
  target.unavailable_samples = add(
    target.unavailable_samples,
    source?.unavailable_samples,
  );
}

function mergeStats(
  target: SessionDiagnosticStats,
  source: Partial<SessionDiagnosticStats> | null | undefined,
): void {
  if (!source) return;
  target.total_model_calls = add(target.total_model_calls, source.total_model_calls);
  target.total_tool_calls = add(target.total_tool_calls ?? 0, source.total_tool_calls);
  target.successful_tool_calls = add(
    target.successful_tool_calls ?? 0,
    source.successful_tool_calls ?? 0,
  );
  target.failed_tool_calls = add(
    target.failed_tool_calls ?? 0,
    source.failed_tool_calls ?? 0,
  );
  mergeMetric(target.input_tokens, source.input_tokens);
  mergeMetric(target.output_tokens, source.output_tokens);
  mergeMetric(target.cache_read_input_tokens, source.cache_read_input_tokens);
  mergeMetric(target.cache_creation_input_tokens, source.cache_creation_input_tokens);
  mergeMetric(target.total_latency_ms, source.total_latency_ms);

  const models = new Map(
    target.calls_per_model.map((entry) => [entry.model.content, entry]),
  );
  const sourceModels = Array.isArray(source.calls_per_model)
    ? source.calls_per_model
    : [];
  for (const entry of sourceModels) {
    if (!entry || typeof entry.model?.content !== "string") {
      target.calls_per_model_truncated = true;
      continue;
    }
    const existing = models.get(entry.model.content);
    if (existing) {
      existing.calls = add(existing.calls, entry.calls);
    } else if (models.size < MAX_SESSION_MODELS) {
      const retained = { model: entry.model, calls: safeCount(entry.calls) };
      target.calls_per_model.push(retained);
      models.set(entry.model.content, retained);
    } else {
      target.calls_per_model_truncated = true;
    }
  }
  target.calls_per_model_truncated ||= source.calls_per_model_truncated === true;
}

interface AuthSessionStats {
  hasData: boolean;
  carried: SessionDiagnosticStats;
  runs: Map<string, SessionDiagnosticStats>;
  retiredRunIds: Set<string>;
}

/**
 * Accumulates the latest authoritative snapshot for each observed run.
 * Replacing a run prevents SSE reconnects and historical navigation from
 * double-counting. Older runs fold into a bounded carried total.
 */
export class InspectorSessionStatsAccumulator {
  private activeAuthScope = "";
  private state: AuthSessionStats = this.emptyState();

  private emptyState(): AuthSessionStats {
    return {
      hasData: false,
      carried: emptyStats(),
      runs: new Map(),
      retiredRunIds: new Set(),
    };
  }

  private selectScope(scope: string): void {
    if (this.activeAuthScope === scope) return;
    this.activeAuthScope = scope;
    this.state = this.emptyState();
  }

  record(
    callerScope: string,
    runScope: string,
    stats: SessionDiagnosticStats,
  ): SessionDiagnosticStats {
    this.selectScope(callerScope);
    if (!runScope || this.state.retiredRunIds.has(runScope)) {
      return this.snapshot(callerScope) ?? emptyStats();
    }
    this.state.hasData = true;
    this.state.runs.delete(runScope);
    this.state.runs.set(runScope, stats);

    while (this.state.runs.size > MAX_ACTIVE_RUN_STATS) {
      const oldest = this.state.runs.entries().next().value as
        | [string, SessionDiagnosticStats]
        | undefined;
      if (!oldest) break;
      this.state.runs.delete(oldest[0]);
      mergeStats(this.state.carried, oldest[1]);
      this.state.retiredRunIds.add(oldest[0]);
      while (this.state.retiredRunIds.size > MAX_RETIRED_RUN_IDS) {
        const oldestRetired = this.state.retiredRunIds.values().next().value as
          | string
          | undefined;
        if (!oldestRetired) break;
        this.state.retiredRunIds.delete(oldestRetired);
      }
    }
    return this.snapshot(callerScope) ?? emptyStats();
  }

  snapshot(callerScope: string): SessionDiagnosticStats | null {
    this.selectScope(callerScope);
    if (!this.state.hasData) return null;
    const result = emptyStats();
    mergeStats(result, this.state.carried);
    for (const stats of this.state.runs.values()) mergeStats(result, stats);
    return result;
  }

  reset(): void {
    this.activeAuthScope = "";
    this.state = this.emptyState();
  }
}

const pageSessionStats = new InspectorSessionStatsAccumulator();

export function recordInspectorSessionStats(
  runScope: string,
  stats: SessionDiagnosticStats,
): SessionDiagnosticStats {
  return pageSessionStats.record(authScope(), runScope, stats);
}

export function readInspectorSessionStats(): SessionDiagnosticStats | null {
  return pageSessionStats.snapshot(authScope());
}

export function resetInspectorSessionStats(): void {
  pageSessionStats.reset();
}
