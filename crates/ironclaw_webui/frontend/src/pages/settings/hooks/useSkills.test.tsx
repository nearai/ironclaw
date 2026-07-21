import assert from "node:assert/strict";
import React from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, test, vi } from "vitest";

const settingsApi = vi.hoisted(() => ({
  fetchSkillContent: vi.fn(),
  fetchSkills: vi.fn(),
  installSkill: vi.fn(),
  removeSkill: vi.fn(),
  setAutoActivateLearned: vi.fn(),
  setSkillAutoActivate: vi.fn(),
  updateSkill: vi.fn(),
}));

vi.mock("../lib/settings-api", () => settingsApi);

import { useSkills } from "./useSkills";

const skillsQueryKey = ["skills"];

function renderUseSkills(queryClient) {
  let hookResult;
  function Harness() {
    hookResult = useSkills();
    return null;
  }

  renderToStaticMarkup(
    <QueryClientProvider client={queryClient}>
      <Harness />
    </QueryClientProvider>,
  );
  assert.ok(hookResult, "useSkills should render inside QueryClientProvider");
  return hookResult;
}

beforeEach(() => {
  vi.clearAllMocks();
  settingsApi.fetchSkills.mockResolvedValue({
    skills: [],
    auto_activate_learned: true,
  });
  settingsApi.setAutoActivateLearned.mockResolvedValue({ success: true });
});

test("global auto-activation mutation keeps its confirmed value in the active cache", async () => {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false, staleTime: Infinity },
    },
  });
  queryClient.setQueryData(skillsQueryKey, {
    skills: [],
    auto_activate_learned: true,
  });
  const invalidateQueries = vi.spyOn(queryClient, "invalidateQueries");

  const skills = renderUseSkills(queryClient);
  await skills.setAutoActivateLearned(false);

  assert.deepEqual(settingsApi.setAutoActivateLearned.mock.calls, [[false]]);
  assert.equal(
    queryClient.getQueryData<{ auto_activate_learned: boolean }>(skillsQueryKey)
      ?.auto_activate_learned,
    false,
  );
  assert.deepEqual(invalidateQueries.mock.calls[0]?.[0], {
    queryKey: skillsQueryKey,
    refetchType: "none",
  });
  queryClient.clear();
});
