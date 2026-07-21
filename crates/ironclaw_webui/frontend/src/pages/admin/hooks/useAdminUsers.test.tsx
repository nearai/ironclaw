import assert from "node:assert/strict";
import React from "react";
import {
  QueryClient,
  QueryClientProvider,
  QueryObserver,
} from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, test, vi } from "vitest";

const adminApi = vi.hoisted(() => ({
  activateAdminUser: vi.fn(),
  createAdminUser: vi.fn(),
  deleteAdminUser: vi.fn(),
  deleteUserSecret: vi.fn(),
  fetchAdminUser: vi.fn(),
  fetchAdminUsers: vi.fn(),
  fetchUserSecrets: vi.fn(),
  putUserSecret: vi.fn(),
  suspendAdminUser: vi.fn(),
  updateAdminUser: vi.fn(),
}));

vi.mock("../lib/admin-api", () => adminApi);

import { useAdminUsers } from "./useAdminUsers";

const usersQueryKey = ["admin", "users"];

function createQueryClient() {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false, staleTime: Infinity },
    },
  });
  queryClient.setQueryData(usersQueryKey, { users: [] });
  return queryClient;
}

function renderUseAdminUsers(queryClient) {
  let hookResult;
  function Harness() {
    hookResult = useAdminUsers();
    return null;
  }

  renderToStaticMarkup(
    <QueryClientProvider client={queryClient}>
      <Harness />
    </QueryClientProvider>,
  );
  assert.ok(hookResult, "useAdminUsers should render inside QueryClientProvider");
  return hookResult;
}

function observeUserDetail(queryClient, userId, queryFn) {
  const queryKey = ["admin", "user", userId];
  queryClient.setQueryData(queryKey, { id: userId, role: "member", status: "active" });
  const observer = new QueryObserver(queryClient, { queryKey, queryFn, staleTime: Infinity });
  const unsubscribe = observer.subscribe(() => {});
  return { queryKey, unsubscribe };
}

beforeEach(() => {
  vi.clearAllMocks();
  adminApi.activateAdminUser.mockImplementation(async (id) => ({ id, status: "active" }));
  adminApi.createAdminUser.mockImplementation(async (payload) => ({
    id: "created-user",
    ...payload,
  }));
  adminApi.deleteAdminUser.mockResolvedValue({});
  adminApi.fetchAdminUsers.mockResolvedValue({ users: [] });
  adminApi.suspendAdminUser.mockImplementation(async (id) => ({
    id,
    status: "suspended",
  }));
  adminApi.updateAdminUser.mockImplementation(async (id, payload) => ({ id, ...payload }));
});

test("role and status mutations refetch the matching active admin user detail", async () => {
  const queryClient = createQueryClient();
  const details = [
    {
      id: "user-role",
      fetch: vi.fn().mockResolvedValue({ id: "user-role", role: "admin", status: "active" }),
    },
    {
      id: "user-suspended",
      fetch: vi.fn().mockResolvedValue({
        id: "user-suspended",
        role: "member",
        status: "suspended",
      }),
    },
    {
      id: "user-active",
      fetch: vi.fn().mockResolvedValue({ id: "user-active", role: "member", status: "active" }),
    },
  ];
  const observations = details.map(({ id, fetch }) =>
    observeUserDetail(queryClient, id, fetch),
  );

  try {
    const adminUsers = renderUseAdminUsers(queryClient);
    await adminUsers.updateUser("user-role", { role: "admin" });
    await adminUsers.suspendUser("user-suspended");
    await adminUsers.activateUser("user-active");

    for (const detail of details) {
      assert.equal(detail.fetch.mock.calls.length, 1);
    }
    assert.equal(
      queryClient.getQueryData<{ role: string }>(observations[0].queryKey)?.role,
      "admin",
    );
    assert.equal(
      queryClient.getQueryData<{ status: string }>(observations[1].queryKey)?.status,
      "suspended",
    );
    assert.equal(
      queryClient.getQueryData<{ status: string }>(observations[2].queryKey)?.status,
      "active",
    );
    assert.equal(queryClient.getQueryState(usersQueryKey)?.isInvalidated, true);
  } finally {
    observations.forEach(({ unsubscribe }) => unsubscribe());
    queryClient.clear();
  }
});

test("create and delete mutations leave active admin user details untouched", async () => {
  const queryClient = createQueryClient();
  const fetchDetail = vi.fn().mockResolvedValue({ id: "existing-user" });
  const observation = observeUserDetail(queryClient, "existing-user", fetchDetail);

  try {
    const adminUsers = renderUseAdminUsers(queryClient);
    await adminUsers.createUser({ display_name: "Created User" });
    await adminUsers.deleteUser("deleted-user");

    assert.equal(fetchDetail.mock.calls.length, 0);
    assert.equal(queryClient.getQueryState(observation.queryKey)?.isInvalidated, false);
    assert.equal(queryClient.getQueryState(usersQueryKey)?.isInvalidated, true);
  } finally {
    observation.unsubscribe();
    queryClient.clear();
  }
});

test("a missing user id never creates or invalidates an admin user detail query", async () => {
  const queryClient = createQueryClient();
  const adminUsers = renderUseAdminUsers(queryClient);

  await adminUsers.suspendUser();

  assert.equal(queryClient.getQueryState(["admin", "user", undefined]), undefined);
  assert.equal(queryClient.getQueryState(usersQueryKey)?.isInvalidated, true);
  queryClient.clear();
});
