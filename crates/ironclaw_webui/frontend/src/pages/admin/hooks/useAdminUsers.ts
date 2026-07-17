// @ts-nocheck
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

export function useAdminUsers() {
  const queryClient = useQueryClient();

  const query = useQuery({
    queryKey: ["admin", "users"],
    queryFn: fetchAdminUsers,
    refetchInterval: 10_000,
  });

  const rawUsers = query.data;
  const users = Array.isArray(rawUsers) ? rawUsers : rawUsers?.users || [];
  // Detect the forbidden state from the structured `ApiError` (see
  // `lib/api.ts`), not the humanized message: a non-admin caller gets HTTP 403
  // whose body kind is humanized to "Participant denied", so a string match on
  // "403"/"Forbidden" would miss it and never render the admin-required panel.
  // Prefer the numeric status; fall back to the parsed error/kind code.
  const err = query.error;
  const errorCode = err?.payload?.kind || err?.payload?.error;
  const isForbidden =
    err?.status === 403 ||
    errorCode === "forbidden" ||
    errorCode === "participant_denied";

  const invalidate = () => queryClient.invalidateQueries({ queryKey: ["admin", "users"] });

  const createMut = useMutation({ mutationFn: createAdminUser, onSuccess: invalidate });
  const updateMut = useMutation({
    mutationFn: ({ id, payload }) => updateAdminUser(id, payload),
    onSuccess: invalidate,
  });
  const deleteMut = useMutation({
    mutationFn: (id) => deleteAdminUser(id),
    onSuccess: invalidate,
  });
  const suspendMut = useMutation({
    mutationFn: (id) => suspendAdminUser(id),
    onSuccess: invalidate,
  });
  const activateMut = useMutation({
    mutationFn: (id) => activateAdminUser(id),
    onSuccess: invalidate,
  });

  return {
    users,
    query,
    isForbidden,
    createUser: createMut.mutateAsync,
    isCreating: createMut.isPending,
    createError: createMut.error,
    updateUser: (id, payload) => updateMut.mutateAsync({ id, payload }),
    deleteUser: deleteMut.mutateAsync,
    suspendUser: suspendMut.mutateAsync,
    activateUser: activateMut.mutateAsync,
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

export function useAdminUserDetail(userId) {
  return useQuery({
    queryKey: ["admin", "user", userId],
    queryFn: () => fetchAdminUser(userId),
    enabled: Boolean(userId),
    refetchInterval: 10_000,
  });
}

export function useAdminUserSecrets(userId) {
  const queryClient = useQueryClient();
  const queryKey = ["admin", "user", userId, "secrets"];
  const query = useQuery({
    queryKey,
    queryFn: () => fetchUserSecrets(userId),
    enabled: Boolean(userId),
  });

  const invalidate = () => queryClient.invalidateQueries({ queryKey });
  const putMutation = useMutation({
    mutationFn: ({ handle, value }) => putUserSecret(userId, handle, value),
    onSuccess: invalidate,
  });
  const deleteMutation = useMutation({
    mutationFn: (handle) => deleteUserSecret(userId, handle),
    onSuccess: invalidate,
  });

  return {
    secrets: Array.isArray(query.data) ? query.data : [],
    query,
    putSecret: (handle, value) => putMutation.mutateAsync({ handle, value }),
    deleteSecret: deleteMutation.mutateAsync,
    isSaving: putMutation.isPending,
    isDeleting: deleteMutation.isPending,
    putError: putMutation.error,
    deleteError: deleteMutation.error,
    resetPut: putMutation.reset,
    resetDelete: deleteMutation.reset,
  };
}
