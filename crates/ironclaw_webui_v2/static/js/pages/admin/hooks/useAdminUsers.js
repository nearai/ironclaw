import { React } from "../../../lib/html.js";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  fetchAdminUsers,
  fetchAdminUser,
  createAdminUser,
  updateAdminUser,
  deleteAdminUser,
  suspendAdminUser,
  activateAdminUser,
  createUserToken,
} from "../lib/admin-api.js";

export function useAdminUsers() {
  const queryClient = useQueryClient();

  const query = useQuery({
    queryKey: ["admin", "users"],
    queryFn: fetchAdminUsers,
    refetchInterval: 10_000,
  });

  const rawUsers = query.data;
  const users = Array.isArray(rawUsers) ? rawUsers : rawUsers?.users || [];
  const isForbidden = query.error?.message?.includes("403") || query.error?.message?.includes("Forbidden");

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
  const tokenMut = useMutation({
    mutationFn: ({ userId, name }) => createUserToken(userId, name),
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
    createToken: (userId, name) => tokenMut.mutateAsync({ userId, name }),
    newToken: tokenMut.data,
    clearToken: () => tokenMut.reset(),
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
