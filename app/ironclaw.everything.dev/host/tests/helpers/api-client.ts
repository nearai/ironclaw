import type { ApiClient } from "../../../ui/src/lib/api";

export function createTestApiClient<TBase extends Record<string, unknown>>(
  baseClient: TBase,
  plugins: Record<string, unknown> = {},
): ApiClient & TBase {
  return {
    ...baseClient,
    ...plugins,
  } as unknown as ApiClient & TBase;
}
