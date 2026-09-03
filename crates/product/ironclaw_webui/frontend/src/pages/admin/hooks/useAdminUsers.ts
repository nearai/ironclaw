import React from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  fetchAdminUsers,
  fetchAdminUser,
  createAdminUser,
  updateAdminUser,
  deleteAdminUser,
  suspendAdminUser,
  activateAdminUser,
  fetchUserSecrets,
  putUserSecret,
  deleteUserSecret,
} from "../lib/admin-api";

const ADMIN_USERS_PAGE_SIZE = 20;

type AdminUser = {
  id: string;
  user_id?: string;
  token?: string;
  display_name?: string;
  email?: string;
  role?: string;
  status?: string;
  created_at?: string;
  last_login_at?: string;
  last_active_at?: string;
  created_by?: string;
  job_count?: number;
  total_cost?: number;
  [key: string]: unknown;
};

type AdminUsersPage = {
  users: AdminUser[];
  total: number;
  nextCursor: string | null;
};

type AdminUserPayload = Record<string, unknown>;
type UpdateAdminUserVariables = { id: string; payload: AdminUserPayload };
type PutSecretVariables = { handle: string; value: string };
type AdminSecret = { handle: string; [key: string]: unknown };

function apiErrorField(error: unknown, field: string): unknown {
  return error && typeof error === "object" ? Reflect.get(error, field) : undefined;
}

function apiErrorPayloadField(error: unknown, field: string): unknown {
  const payload = apiErrorField(error, "payload");
  return payload && typeof payload === "object"
    ? Reflect.get(payload, field)
    : undefined;
}

export function useAdminUsers() {
  const queryClient = useQueryClient();
  const requestedMoreRef = React.useRef(false);

  const query = useQuery<AdminUsersPage>({
    queryKey: ["admin", "users"],
    queryFn: ({ signal }) => fetchAdminUsers({
      limit: ADMIN_USERS_PAGE_SIZE,
      signal,
    }),
    // Poll the initial bounded page, but stop once the administrator attempts
    // to load more. Additional pages are fetched directly below, so polling
    // can never multiply traffic by the number of retained pages.
    refetchInterval: (currentQuery) => {
      return currentQuery.state.fetchStatus === "idle" &&
        !requestedMoreRef.current
        ? 10_000
        : false;
    },
  });

  const [additionalPages, setAdditionalPages] = React.useState<AdminUsersPage[]>([]);
  const [isLoadingMore, setIsLoadingMore] = React.useState(false);
  const [loadMoreError, setLoadMoreError] = React.useState(null);
  const initialPageUpdatedAtRef = React.useRef(query.dataUpdatedAt);
  React.useEffect(() => {
    if (initialPageUpdatedAtRef.current === query.dataUpdatedAt) return;
    initialPageUpdatedAtRef.current = query.dataUpdatedAt;
    setAdditionalPages([]);
    setLoadMoreError(null);
    requestedMoreRef.current = false;
  }, [query.dataUpdatedAt]);

  const pages = [query.data, ...additionalPages].filter(
    (page): page is AdminUsersPage => Boolean(page),
  );
  const users = React.useMemo(() => {
    const seen = new Set<string>();
    return pages.flatMap((page) =>
      (page?.users || []).filter((user) => {
        const id = user?.id || user?.user_id;
        if (!id || seen.has(id)) return false;
        seen.add(id);
        return true;
      }),
    );
  }, [query.data, additionalPages]);
  const nextCursor = pages.at(-1)?.nextCursor || null;
  const loadMoreInFlightRef = React.useRef<Promise<AdminUsersPage | null> | null>(null);
  const loadMore = React.useCallback(() => {
    if (!nextCursor) return Promise.resolve();
    requestedMoreRef.current = true;
    if (loadMoreInFlightRef.current) return loadMoreInFlightRef.current;

    setIsLoadingMore(true);
    setLoadMoreError(null);
    // The cursor page load retries once on a transient failure, so a 5xx
    // or rate-limited response from the admin users endpoint surfaces the
    // structured retry state instead of a terminal load-more error. This
    // matches React Query's default retry policy for the initial page query
    // above and keeps pagination resilient under brief backend hiccups.
    // Non-retryable failures (4xx authorization/validation) fail fast.
    const isTransient = (error: unknown) => {
      const status = apiErrorField(error, "status");
      if (typeof status !== "number") return true;
      if (status === 429 || status >= 500) return true;
      return Boolean(apiErrorPayloadField(error, "retryable"));
    };
    const attempt = (retriesLeft: number) =>
      fetchAdminUsers({
        limit: ADMIN_USERS_PAGE_SIZE,
        cursor: nextCursor,
      })
        .then((page) => {
          setAdditionalPages((current) => [...current, page]);
          return page;
        })
        .catch((error) => {
          if (retriesLeft > 0 && isTransient(error)) {
            return attempt(retriesLeft - 1);
          }
          setLoadMoreError(error);
          return null;
        });
    const request = attempt(1)
      .finally(() => {
        setIsLoadingMore(false);
        if (loadMoreInFlightRef.current === request) {
          loadMoreInFlightRef.current = null;
        }
      });
    loadMoreInFlightRef.current = request;
    return request;
  }, [nextCursor]);
  // Detect the forbidden state from the structured `ApiError` (see
  // `lib/api.ts`), not the humanized message: a non-admin caller gets HTTP 403
  // whose body kind is humanized to "Participant denied", so a string match on
  // "403"/"Forbidden" would miss it and never render the admin-required panel.
  // Prefer the numeric status; fall back to the parsed error/kind code.
  const err = query.error;
  const errorCode =
    apiErrorPayloadField(err, "kind") || apiErrorPayloadField(err, "error");
  const isForbidden =
    apiErrorField(err, "status") === 403 ||
    errorCode === "forbidden" ||
    errorCode === "participant_denied";

  const invalidateUsers = () =>
    queryClient.invalidateQueries({ queryKey: ["admin", "users"] });
  const refreshUser = (userId: string, user: AdminUser) => {
    if (userId && user?.id === userId) {
      queryClient.setQueryData<AdminUser>(["admin", "user", userId], (currentUser) =>
        currentUser ? { ...currentUser, ...user } : user,
      );
    }
    const invalidations = [invalidateUsers()];
    if (userId) {
      invalidations.push(
        queryClient.invalidateQueries({
          queryKey: ["admin", "user", userId],
          exact: true,
          refetchType: "active",
        }),
      );
    }
    return Promise.all(invalidations);
  };

  const createMut = useMutation<AdminUser, Error, AdminUserPayload>({
    mutationFn: createAdminUser,
    onSuccess: invalidateUsers,
  });
  const updateMut = useMutation<AdminUser, Error, UpdateAdminUserVariables>({
    mutationFn: ({ id, payload }) => updateAdminUser(id, payload),
    onSuccess: (user, { id }) => refreshUser(id, user),
  });
  const deleteMut = useMutation<unknown, Error, string>({
    mutationFn: (id) => deleteAdminUser(id),
    onSuccess: invalidateUsers,
  });
  const suspendMut = useMutation<AdminUser, Error, string>({
    mutationFn: (id) => suspendAdminUser(id),
    onSuccess: (user, id) => refreshUser(id, user),
  });
  const activateMut = useMutation<AdminUser, Error, string>({
    mutationFn: (id) => activateAdminUser(id),
    onSuccess: (user, id) => refreshUser(id, user),
  });

  const resetActionErrors = () => {
    updateMut.reset();
    deleteMut.reset();
    suspendMut.reset();
    activateMut.reset();
  };

  return {
    users,
    query,
    isForbidden,
    hasMore: Boolean(nextCursor),
    isLoadingMore,
    loadMoreError,
    loadMore,
    createUser: createMut.mutateAsync,
    isCreating: createMut.isPending,
    createError: createMut.error,
    resetCreate: createMut.reset,
    updateUser: (id: string, payload: AdminUserPayload) =>
      updateMut.mutateAsync({ id, payload }),
    isUpdating: updateMut.isPending,
    updateError: updateMut.error,
    updatingUserId: updateMut.variables?.id || null,
    resetUpdate: updateMut.reset,
    deleteUser: deleteMut.mutateAsync,
    isDeleting: deleteMut.isPending,
    deleteError: deleteMut.error,
    deletingUserId: deleteMut.variables || null,
    resetDelete: deleteMut.reset,
    suspendUser: suspendMut.mutateAsync,
    isSuspending: suspendMut.isPending,
    suspendError: suspendMut.error,
    suspendingUserId: suspendMut.variables || null,
    resetSuspend: suspendMut.reset,
    activateUser: activateMut.mutateAsync,
    isActivating: activateMut.isPending,
    activateError: activateMut.error,
    activatingUserId: activateMut.variables || null,
    resetActionErrors,
    // The one-time API bearer is issued ONLY at user creation, so the create
    // result (which carries `.token`) feeds the one-time token banner. There is
    // no re-issue endpoint for existing users, so no `createToken` action is
    // exposed here — see `lib/admin-api.ts::createUserToken`.
    newToken: createMut.data?.token ? createMut.data : null,
    clearToken: () => {
      createMut.reset();
    },
  };
}

export function useAdminUserDetail(userId: string) {
  return useQuery<AdminUser | null>({
    queryKey: ["admin", "user", userId],
    queryFn: () => fetchAdminUser(userId),
    enabled: Boolean(userId),
    refetchInterval: 10_000,
  });
}

export function useAdminUserSecrets(userId: string) {
  const queryClient = useQueryClient();
  const queryKey = ["admin", "user", userId, "secrets"];
  const query = useQuery<AdminSecret[]>({
    queryKey,
    queryFn: () => fetchUserSecrets(userId),
    enabled: Boolean(userId),
  });

  const invalidate = () => queryClient.invalidateQueries({ queryKey });
  const putMutation = useMutation<AdminSecret, Error, PutSecretVariables>({
    mutationFn: ({ handle, value }) => putUserSecret(userId, handle, value),
    onSuccess: invalidate,
  });
  const deleteMutation = useMutation<unknown, Error, string>({
    mutationFn: (handle) => deleteUserSecret(userId, handle),
    onSuccess: invalidate,
  });

  return {
    secrets: Array.isArray(query.data) ? query.data : [],
    query,
    putSecret: (handle: string, value: string) =>
      putMutation.mutateAsync({ handle, value }),
    deleteSecret: deleteMutation.mutateAsync,
    isSaving: putMutation.isPending,
    isDeleting: deleteMutation.isPending,
    putError: putMutation.error,
    deleteError: deleteMutation.error,
    resetPut: putMutation.reset,
    resetDelete: deleteMutation.reset,
  };
}
