// @ts-nocheck
import { useQuery } from "@tanstack/react-query";
import React from "react";
import { getAutomation } from "../../../lib/api";
import { useI18n } from "../../../lib/i18n";

import { normalizeAutomations } from "../lib/automations-presenters";

// Resolve a single automation by id for the full-screen detail view / deep
// link. The list query (`useAutomations`) is capped and excludes completed
// one-shots, so a deep link to a completed automation — or one past the list
// page — is not resolvable from it. This hook fetches the automation directly
// by id (`GET /api/webchat/v2/automations/:id`), which has no such filter.
//
// `seed` is the matching row from the already-loaded list, if any; it lets the
// detail view paint instantly (e.g. when popping out of the list modal) while
// the authoritative by-id fetch settles. A 404 (missing or not caller-owned)
// surfaces as `error`, so the detail page renders its not-found state.
export function useAutomation(automationId, { seed = null } = {}) {
  const { t, lang } = useI18n();
  const query = useQuery({
    queryKey: ["automation", automationId],
    queryFn: () => getAutomation({ automationId }),
    enabled: Boolean(automationId),
  });

  const automation = React.useMemo(() => {
    if (query.data?.automation) {
      // Reuse the list normalizer (filter + normalize + sort) on a single-row
      // response so an unsupported source type resolves to not-found, exactly
      // as it would be omitted from the list.
      return (
        normalizeAutomations({ automations: [query.data.automation] }, t, lang)[0] ||
        null
      );
    }
    return seed;
  }, [query.data, seed, t, lang]);

  return {
    automation,
    // Only show the loading state when there is nothing to paint yet; a seeded
    // row keeps the view populated while the by-id fetch refreshes it.
    isLoading: query.isLoading && seed == null,
    error: query.isError ? query.error : null,
    schedulerEnabled: query.data?.scheduler_enabled !== false,
  };
}
