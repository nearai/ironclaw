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
    useRef(initial) {
      const index = cursor;
      cursor += 1;
      if (!(index in state)) state[index] = { current: initial };
      return state[index];
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
  if (params.message) return `${key}:${params.message}`;
  return params.name ? `${key}:${params.name}` : key;
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
    hasMore: false,
    isLoadingMore: false,
    loadMoreError: null,
    loadMore: async () => {},
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
    newToken: null,
    clearToken: () => {},
    ...overrides,
  };
}

function loadUsersView(harness) {
  function ConfirmDialog() {}
  const module = runVmModuleForTest(
    "./users-tab.tsx",
    ["AdminUsersTabView", "UserRow"],
    {
      React: harness.React,
      useT: () => translate,
      Panel: function Panel() {},
      StatusPill: function StatusPill() {},
      EmptyPanel: function EmptyPanel() {},
      Button: function Button() {},
      ConfirmDialog,
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
  return { ...module, ConfirmDialog };
}

function loadDetailModule(harness) {
  function ConfirmDialog() {}
  function ThreadScrapingPanel() {}
  const module = runVmModuleForTest(
    "./user-detail.tsx",
    ["UserDetail", "UserDetailView"],
    {
      React: harness.React,
      useT: () => translate,
      Panel: function Panel() {},
      StatCard: function StatCard() {},
      StatusPill: function StatusPill() {},
      Button: function Button() {},
      ConfirmDialog,
      SelectMenu: function SelectMenu() {},
      useAdminUserDetail: () => ({}),
      useAdminUsers: () => baseAdminState(),
      useUsage: () => ({}),
      UserSecretsPanel: function UserSecretsPanel() {},
      ThreadScrapingPanel,
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
  return { ...module, ConfirmDialog, ThreadScrapingPanel };
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
  assert.equal(rendered.props.threadScrapingEnabled, false);
  assert.equal(
    UserDetail({ userId: "user-2", onBack: () => {}, threadScrapingEnabled: true })
      .props.threadScrapingEnabled,
    true,
  );
});

test("user detail exposes thread scraping only when the deployment gate is enabled", () => {
  const harness = createReactHarness();
  const { UserDetailView: View, ThreadScrapingPanel } = loadDetailModule(harness);
  const props = {
    onBack: () => {},
    userQuery: { isLoading: false, error: null, data: baseAdminState().users[0] },
    usageQuery: { data: { usage: [] } },
    adminState: baseAdminState(),
  };

  assert.equal(findByType(harness.render(View, props), ThreadScrapingPanel), null);
  assert.ok(findByType(harness.render(View, { ...props, threadScrapingEnabled: true }), ThreadScrapingPanel));
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

test("users list renders load-more progress, retry, and final-page states", async () => {
  const harness = createReactHarness();
  const { AdminUsersTabView: View } = loadUsersView(harness);
  let loadMoreCalls = 0;
  const loadMore = async () => {
    loadMoreCalls += 1;
  };

  const available = harness.render(View, {
    onSelectUser: () => {},
    adminState: baseAdminState({ hasMore: true, loadMore }),
  });
  const availableButton = findByTestId(available, "admin-users-load-more");
  assert.ok(availableButton);
  assert.equal(availableButton.props.disabled, false);
  assert.ok(collectScalars(availableButton).includes("common.loadMore"));
  await availableButton.props.onClick();
  assert.equal(loadMoreCalls, 1);

  const loading = harness.render(View, {
    onSelectUser: () => {},
    adminState: baseAdminState({
      hasMore: true,
      isLoadingMore: true,
      loadMore,
    }),
  });
  const loadingButton = findByTestId(loading, "admin-users-load-more");
  assert.equal(loadingButton.props.disabled, true);
  assert.equal(loadingButton.props.loading, true);
  assert.ok(collectScalars(loadingButton).includes("common.loading"));

  const failed = harness.render(View, {
    onSelectUser: () => {},
    adminState: baseAdminState({
      hasMore: true,
      loadMoreError: new Error("next page failed"),
      loadMore,
    }),
  });
  assert.ok(findByTestId(failed, "admin-users-load-more-error"));
  assert.ok(findByTestId(failed, "admin-users-load-more"));

  const finalPage = harness.render(View, {
    onSelectUser: () => {},
    adminState: baseAdminState({ hasMore: false }),
  });
  assert.equal(findByTestId(finalPage, "admin-users-load-more"), null);
});

test("suspend failure stays in the confirmation dialog with retry context", async () => {
  const harness = createReactHarness();
  const { AdminUsersTabView: View, ConfirmDialog, UserRow } = loadUsersView(harness);
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
  const trigger = { isConnected: true, focus() {} };
  findByTestId(row, "admin-user-suspend").props.onClick({ currentTarget: trigger });
  rendered = harness.render(View, { onSelectUser: () => {}, adminState });
  let dialog = findByType(rendered, ConfirmDialog);
  assert.equal(dialog.props.open, true);
  assert.equal(dialog.props.returnFocusTo, trigger);
  assert.ok(collectScalars(dialog.props.description).includes("admin.users.suspendDesc:Owner"));
  assert.ok(collectScalars(dialog.props.description).includes("admin.users.lastAdminRequired"));

  await dialog.props.onConfirm();
  assert.deepEqual(suspendedUserIds, ["user-1"]);
  rendered = harness.render(View, { onSelectUser: () => {}, adminState });
  dialog = findByType(rendered, ConfirmDialog);
  assert.equal(dialog.props.open, true);
});

test("opening and cancelling suspend preserves unrelated action errors", () => {
  const harness = createReactHarness();
  const { AdminUsersTabView: View, ConfirmDialog, UserRow } = loadUsersView(harness);
  let resetActionCalls = 0;
  let resetSuspendCalls = 0;
  const adminState = baseAdminState({
    updateError: new Error("cannot demote last admin"),
    resetActionErrors: () => { resetActionCalls += 1; },
    resetSuspend: () => { resetSuspendCalls += 1; },
  });

  let rendered = harness.render(View, { onSelectUser: () => {}, adminState });
  const row = UserRow(findByType(rendered, UserRow).props);
  findByTestId(row, "admin-user-suspend").props.onClick({
    currentTarget: { isConnected: true, focus() {} },
  });
  assert.equal(resetActionCalls, 0);
  assert.equal(resetSuspendCalls, 1);

  rendered = harness.render(View, { onSelectUser: () => {}, adminState });
  assert.ok(findByTestId(rendered, "admin-user-action-error"));
  findByType(rendered, ConfirmDialog).props.onCancel();
  assert.equal(resetActionCalls, 0);
  assert.equal(resetSuspendCalls, 2);

  rendered = harness.render(View, { onSelectUser: () => {}, adminState });
  assert.ok(findByTestId(rendered, "admin-user-action-error"));
  assert.equal(findByType(rendered, ConfirmDialog).props.open, false);
});

test("admin confirmations ignore repeated submissions while requests are in flight", async () => {
  const suspendHarness = createReactHarness();
  const {
    AdminUsersTabView,
    ConfirmDialog: SuspendDialog,
    UserRow,
  } = loadUsersView(suspendHarness);
  let resolveSuspend;
  let suspendCalls = 0;
  const suspendState = baseAdminState({
    suspendUser: () => {
      suspendCalls += 1;
      return new Promise((resolve) => { resolveSuspend = resolve; });
    },
  });
  let rendered = suspendHarness.render(AdminUsersTabView, {
    onSelectUser: () => {},
    adminState: suspendState,
  });
  const row = UserRow(findByType(rendered, UserRow).props);
  findByTestId(row, "admin-user-suspend").props.onClick({
    currentTarget: { isConnected: true, focus() {} },
  });
  rendered = suspendHarness.render(AdminUsersTabView, {
    onSelectUser: () => {},
    adminState: suspendState,
  });
  const suspendDialog = findByType(rendered, SuspendDialog);
  const firstSuspend = suspendDialog.props.onConfirm();
  const secondSuspend = suspendDialog.props.onConfirm();
  assert.equal(suspendCalls, 1);
  resolveSuspend();
  await Promise.all([firstSuspend, secondSuspend]);

  const deleteHarness = createReactHarness();
  const {
    UserDetailView,
    ConfirmDialog: DeleteDialog,
  } = loadDetailModule(deleteHarness);
  let resolveDelete;
  let deleteCalls = 0;
  const deleteState = baseAdminState({
    deleteUser: () => {
      deleteCalls += 1;
      return new Promise((resolve) => { resolveDelete = resolve; });
    },
  });
  const detailProps = {
    onBack: () => {},
    userQuery: { isLoading: false, error: null, data: baseAdminState().users[0] },
    usageQuery: { data: { usage: [] } },
    adminState: deleteState,
  };
  rendered = deleteHarness.render(UserDetailView, detailProps);
  findByTestId(rendered, "admin-user-detail-delete").props.onClick({
    currentTarget: { isConnected: true, focus() {} },
  });
  rendered = deleteHarness.render(UserDetailView, detailProps);
  const deleteDialog = findByType(rendered, DeleteDialog);
  const firstDelete = deleteDialog.props.onConfirm();
  const secondDelete = deleteDialog.props.onConfirm();
  assert.equal(deleteCalls, 1);
  resolveDelete();
  await Promise.all([firstDelete, secondDelete]);
});

test("opening and cancelling delete preserves unrelated action errors", () => {
  const harness = createReactHarness();
  const { UserDetailView: View, ConfirmDialog } = loadDetailModule(harness);
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
  const trigger = { isConnected: true, focus() {} };
  findByTestId(rendered, "admin-user-detail-delete").props.onClick({ currentTarget: trigger });
  assert.equal(resetActionCalls, 0);
  assert.equal(resetDeleteCalls, 1);

  rendered = harness.render(View, props);
  assert.ok(findByTestId(rendered, "admin-user-detail-role-error"));
  const dialog = findByType(rendered, ConfirmDialog);
  assert.equal(dialog.props.returnFocusTo, trigger);
  dialog.props.onCancel();
  assert.equal(resetActionCalls, 0);
  assert.equal(resetDeleteCalls, 2);

  rendered = harness.render(View, props);
  assert.ok(findByTestId(rendered, "admin-user-detail-role-error"));
  assert.equal(findByType(rendered, ConfirmDialog).props.open, false);
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
  const { UserDetailView: View, ConfirmDialog } = loadDetailModule(harness);
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
  let dialog = findByType(rendered, ConfirmDialog);
  assert.equal(dialog.props.open, true);
  assert.ok(
    collectScalars(dialog.props.description).includes(
      "admin.users.actionFailed:cannot delete last admin",
    ),
  );

  await dialog.props.onConfirm();
  rendered = harness.render(View, props);
  dialog = findByType(rendered, ConfirmDialog);
  assert.equal(dialog.props.open, true);
  assert.equal(backCalls, 0);

  adminState.isDeleting = true;
  rendered = harness.render(View, props);
  dialog = findByType(rendered, ConfirmDialog);
  assert.equal(dialog.props.isConfirming, true);
});
