// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../../../test-support/vm-module-harness";

function visit(node, fn) {
  if (Array.isArray(node)) {
    for (const item of node) visit(item, fn);
    return;
  }
  if (node == null) return;
  fn(node);
  if (typeof node === "object") {
    for (const value of Object.values(node)) visit(value, fn);
  }
}

function findByTestId(root, testId) {
  let found = null;
  visit(root, (node) => {
    if (!found && typeof node === "object" && node.props?.["data-testid"] === testId) {
      found = node;
    }
  });
  return found;
}

function findByType(root, type) {
  let found = null;
  visit(root, (node) => {
    if (!found && typeof node === "object" && node.type === type) found = node;
  });
  return found;
}

function collectScalars(root) {
  const scalars = [];
  visit(root, (value) => {
    if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
      scalars.push(value);
    }
  });
  return scalars;
}

function createReactHarness() {
  const state = [];
  let cursor = 0;
  const React = {
    useState(initial) {
      const index = cursor;
      cursor += 1;
      if (!(index in state)) state[index] = typeof initial === "function" ? initial() : initial;
      return [
        state[index],
        (next) => {
          state[index] = typeof next === "function" ? next(state[index]) : next;
        },
      ];
    },
    useMemo(factory) {
      return factory();
    },
    useEffect(effect) {
      effect();
    },
  };
  return {
    React,
    render(component, props) {
      cursor = 0;
      return component(props);
    },
  };
}

function translate(key, params = {}) {
  return params.message ? `${key}:${params.message}` : key;
}

function baseAdminState(overrides = {}) {
  return {
    users: [{
      id: "user-1",
      display_name: "Owner",
      role: "admin",
      status: "active",
      job_count: 0,
      total_cost: 0,
    }],
    query: { isLoading: false, error: null },
    isForbidden: false,
    createUser: async () => {},
    isCreating: false,
    createError: null,
    resetCreate: () => {},
    updateUser: async () => {},
    isUpdating: false,
    updateError: null,
    updatingUserId: null,
    resetUpdate: () => {},
    deleteUser: async () => {},
    isDeleting: false,
    deleteError: null,
    deletingUserId: null,
    resetDelete: () => {},
    suspendUser: async () => {},
    isSuspending: false,
    suspendError: null,
    suspendingUserId: null,
    resetSuspend: () => {},
    activateUser: async () => {},
    isActivating: false,
    activateError: null,
    activatingUserId: null,
    resetActionErrors: () => {},
    createManagedAgent: async () => {},
    isCreatingManagedAgent: false,
    createManagedAgentError: null,
    resetCreateManagedAgent: () => {},
    ...overrides,
  };
}

function loadUsersView(harness) {
  return runVmModuleForTest(
    "./users-tab.tsx",
    ["AdminUsersTabView", "ConfirmModal", "UserRow"],
    {
      React: harness.React,
      useT: () => translate,
      Panel: function Panel() {},
      StatusPill: function StatusPill() {},
      EmptyPanel: function EmptyPanel() {},
      Button: function Button() {},
      Icon: function Icon() {},
      SelectMenu: function SelectMenu() {},
      useAdminUsers: () => baseAdminState(),
      formatRelativeTime: () => "never",
      formatCost: () => "$0",
      truncateId: (id) => id,
      statusTone: () => "muted",
      roleTone: () => "muted",
      formatUserRole: (role) => role,
      formatUserStatus: (status) => status,
      filterUsers: (users) => users,
      buildRoleOptions: () => [],
      adminUserActionErrorMessage: (error, t) => error?.payload?.field === "last_admin"
        ? t("admin.users.lastAdminRequired")
        : t("admin.users.actionFailed", { message: error.message }),
      navigator: {},
      setTimeout: () => {},
    },
    import.meta.url,
  );
}

function loadDetailModule(harness) {
  return runVmModuleForTest(
    "./user-detail.tsx",
    ["UserDetail", "UserDetailView"],
    {
      React: harness.React,
      useT: () => translate,
      Panel: function Panel() {},
      StatCard: function StatCard() {},
      StatusPill: function StatusPill() {},
      Button: function Button() {},
      SelectMenu: function SelectMenu() {},
      useAdminUserDetail: () => ({}),
      useAdminUsers: () => baseAdminState(),
      useUsage: () => ({}),
      UserSecretsPanel: function UserSecretsPanel() {},
      formatRelativeTime: () => "never",
      formatCost: () => "$0",
      formatTokenCount: () => "0",
      truncateId: (id) => id,
      statusTone: () => "muted",
      roleTone: () => "muted",
      formatUserRole: (role) => role,
      formatUserStatus: (status) => status,
      buildRoleOptions: () => [],
      adminUserActionErrorMessage: (error, t) => error?.payload?.field === "last_admin"
        ? t("admin.users.lastAdminRequired")
        : t("admin.users.actionFailed", { message: error.message }),
    },
    import.meta.url,
  );
}

function loadDetailView(harness) {
  return loadDetailModule(harness).UserDetailView;
}

test("user detail view is keyed by user id so local state resets between users", () => {
  const harness = createReactHarness();
  const { UserDetail } = loadDetailModule(harness);

  const rendered = UserDetail({ userId: "user-2", onBack: () => {} });

  assert.equal(rendered.type.name, "UserDetailView");
  assert.equal(rendered.props.key, "user-2");
});

test("users list shows activate and role failures and disables actions while pending", () => {
  const harness = createReactHarness();
  const { AdminUsersTabView: View, UserRow } = loadUsersView(harness);

  for (const errorState of [
    { activateError: new Error("activate denied") },
    { updateError: new Error("last admin") },
  ]) {
    const rendered = harness.render(View, {
      onSelectUser: () => {},
      adminState: baseAdminState(errorState),
    });
    assert.ok(findByTestId(rendered, "admin-user-action-error"));
    assert.ok(collectScalars(rendered).some((value) => String(value).includes("admin.users.actionFailed")));
  }

  const pending = harness.render(View, {
    onSelectUser: () => {},
    adminState: baseAdminState({
      isUpdating: true,
      updatingUserId: "user-1",
    }),
  });
  const pendingRow = UserRow(findByType(pending, UserRow).props);
  assert.equal(findByTestId(pendingRow, "admin-user-role").props.disabled, true);
  assert.equal(findByTestId(pendingRow, "admin-user-role").props["aria-busy"], true);
  assert.ok(collectScalars(findByTestId(pendingRow, "admin-user-role")).includes("common.saving"));
});

test("suspend failure stays in the confirmation dialog with retry context", async () => {
  const harness = createReactHarness();
  const { AdminUsersTabView: View, ConfirmModal, UserRow } = loadUsersView(harness);
  const lastAdminError = Object.assign(new Error("Conflict (last_admin)"), {
    payload: { field: "last_admin" },
  });
  const suspendedUserIds = [];
  const adminState = baseAdminState({
    suspendError: lastAdminError,
    suspendUser: async (userId) => {
      suspendedUserIds.push(userId);
      throw new Error("cannot suspend last admin");
    },
  });

  let rendered = harness.render(View, { onSelectUser: () => {}, adminState });
  const row = UserRow(findByType(rendered, UserRow).props);
  findByTestId(row, "admin-user-suspend").props.onClick();
  rendered = harness.render(View, { onSelectUser: () => {}, adminState });
  let modal = ConfirmModal(findByType(rendered, ConfirmModal).props);
  assert.ok(findByTestId(modal, "admin-user-confirm-dialog"));
  assert.ok(collectScalars(modal).includes("admin.users.lastAdminRequired"));

  await findByTestId(modal, "admin-user-confirm-submit").props.onClick();
  assert.deepEqual(suspendedUserIds, ["user-1"]);
  rendered = harness.render(View, { onSelectUser: () => {}, adminState });
  modal = ConfirmModal(findByType(rendered, ConfirmModal).props);
  assert.ok(findByTestId(modal, "admin-user-confirm-dialog"));
});

test("opening and cancelling suspend preserves unrelated action errors", () => {
  const harness = createReactHarness();
  const { AdminUsersTabView: View, ConfirmModal, UserRow } = loadUsersView(harness);
  let resetActionCalls = 0;
  let resetSuspendCalls = 0;
  const adminState = baseAdminState({
    updateError: new Error("cannot demote last admin"),
    resetActionErrors: () => { resetActionCalls += 1; },
    resetSuspend: () => { resetSuspendCalls += 1; },
  });

  let rendered = harness.render(View, { onSelectUser: () => {}, adminState });
  const row = UserRow(findByType(rendered, UserRow).props);
  findByTestId(row, "admin-user-suspend").props.onClick();
  assert.equal(resetActionCalls, 0);
  assert.equal(resetSuspendCalls, 1);

  rendered = harness.render(View, { onSelectUser: () => {}, adminState });
  assert.ok(findByTestId(rendered, "admin-user-action-error"));
  findByType(rendered, ConfirmModal).props.onCancel();
  assert.equal(resetActionCalls, 0);
  assert.equal(resetSuspendCalls, 2);

  rendered = harness.render(View, { onSelectUser: () => {}, adminState });
  assert.ok(findByTestId(rendered, "admin-user-action-error"));
  assert.equal(findByType(rendered, ConfirmModal), null);
});

test("opening and cancelling delete preserves unrelated action errors", () => {
  const harness = createReactHarness();
  const View = loadDetailView(harness);
  let resetActionCalls = 0;
  let resetDeleteCalls = 0;
  const adminState = baseAdminState({
    updateError: new Error("cannot demote last admin"),
    resetActionErrors: () => { resetActionCalls += 1; },
    resetDelete: () => { resetDeleteCalls += 1; },
  });
  const props = {
    onBack: () => {},
    userQuery: { isLoading: false, error: null, data: baseAdminState().users[0] },
    usageQuery: { data: { usage: [] } },
    adminState,
  };

  let rendered = harness.render(View, props);
  assert.ok(findByTestId(rendered, "admin-user-detail-role-error"));
  findByTestId(rendered, "admin-user-detail-delete").props.onClick();
  assert.equal(resetActionCalls, 0);
  assert.equal(resetDeleteCalls, 1);

  rendered = harness.render(View, props);
  assert.ok(findByTestId(rendered, "admin-user-detail-role-error"));
  findByTestId(rendered, "admin-user-delete-cancel").props.onClick();
  assert.equal(resetActionCalls, 0);
  assert.equal(resetDeleteCalls, 2);

  rendered = harness.render(View, props);
  assert.ok(findByTestId(rendered, "admin-user-detail-role-error"));
  assert.equal(findByTestId(rendered, "admin-user-delete-dialog"), null);
});

test("user detail surfaces status and role failures", () => {
  const harness = createReactHarness();
  const View = loadDetailView(harness);
  const props = (adminState) => ({
    onBack: () => {},
    userQuery: { isLoading: false, error: null, data: baseAdminState().users[0] },
    usageQuery: { data: { usage: [] } },
    adminState,
  });

  const statusFailure = harness.render(View, props(baseAdminState({
    suspendError: new Error("cannot suspend last admin"),
  })));
  assert.ok(findByTestId(statusFailure, "admin-user-detail-status-error"));
  assert.ok(collectScalars(statusFailure).includes("admin.users.actionFailed:cannot suspend last admin"));

  const roleFailure = harness.render(View, props(baseAdminState({
    updateError: new Error("cannot demote last admin"),
  })));
  assert.ok(findByTestId(roleFailure, "admin-user-detail-role-error"));
  assert.ok(collectScalars(roleFailure).includes("admin.users.actionFailed:cannot demote last admin"));
});

test("delete failure keeps the dialog open and does not navigate away", async () => {
  const harness = createReactHarness();
  const View = loadDetailView(harness);
  let backCalls = 0;
  const adminState = baseAdminState({
    deleteError: new Error("cannot delete last admin"),
    deleteUser: async () => { throw new Error("cannot delete last admin"); },
  });
  const props = {
    onBack: () => { backCalls += 1; },
    userQuery: { isLoading: false, error: null, data: baseAdminState().users[0] },
    usageQuery: { data: { usage: [] } },
    adminState,
  };

  let rendered = harness.render(View, props);
  findByTestId(rendered, "admin-user-detail-delete").props.onClick();
  rendered = harness.render(View, props);
  assert.ok(findByTestId(rendered, "admin-user-delete-dialog"));
  assert.ok(collectScalars(rendered).includes("admin.users.actionFailed:cannot delete last admin"));

  await findByTestId(rendered, "admin-user-delete-confirm").props.onClick();
  rendered = harness.render(View, props);
  assert.ok(findByTestId(rendered, "admin-user-delete-dialog"));
  assert.equal(backCalls, 0);

  adminState.isDeleting = true;
  rendered = harness.render(View, props);
  assert.equal(findByTestId(rendered, "admin-user-delete-confirm").props.disabled, true);
  assert.equal(findByTestId(rendered, "admin-user-delete-confirm").props.loading, true);
});
