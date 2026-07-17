// @ts-nocheck
import React from "react";
import { useT } from "../../../lib/i18n";
import { Panel } from "../../../design-system/primitives";
import { Button } from "../../../design-system/button";
import { Input } from "../../../design-system/input";
import { Modal, ModalBody, ModalFooter } from "../../../design-system/modal";
import { useAdminUserSecrets } from "../hooks/useAdminUsers";

export function UserSecretsPanel({ userId }) {
  const secretsState = useAdminUserSecrets(userId);
  return <UserSecretsPanelView {...secretsState} />;
}

export function UserSecretsPanelView({
  secrets,
  query,
  putSecret,
  deleteSecret,
  isSaving,
  isDeleting,
  putError,
  deleteError,
  resetPut,
  resetDelete,
}) {
  const t = useT();
  const [handle, setHandle] = React.useState("");
  const [value, setValue] = React.useState("");
  const [success, setSuccess] = React.useState("");
  const [pendingDelete, setPendingDelete] = React.useState(null);
  const normalizedHandle = handle.trim();
  const isMutating = isSaving || isDeleting;

  const handleSubmit = async (event) => {
    event.preventDefault();
    if (!normalizedHandle || value.length === 0 || isMutating) return;
    setSuccess("");
    resetPut?.();
    resetDelete?.();
    try {
      await putSecret(normalizedHandle, value);
      setHandle("");
      setValue("");
      setSuccess(t("admin.user.secrets.saved", { handle: normalizedHandle }));
    } catch (_) {
      // The mutation exposes its sanitized error through `putError`.
    }
  };

  const handleDelete = async () => {
    if (!pendingDelete || isMutating) return;
    setSuccess("");
    resetPut?.();
    resetDelete?.();
    try {
      await deleteSecret(pendingDelete);
      setSuccess(t("admin.user.secrets.deleted", { handle: pendingDelete }));
      setPendingDelete(null);
    } catch (_) {
      // Keep the confirmation open so the administrator can retry.
    }
  };

  const beginReplace = (secretHandle) => {
    setHandle(secretHandle);
    setValue("");
    setSuccess("");
    resetPut?.();
  };

  const beginDelete = (secretHandle) => {
    setPendingDelete(secretHandle);
    setSuccess("");
    resetDelete?.();
  };

  const closeDelete = () => {
    if (isDeleting) return;
    setPendingDelete(null);
    resetDelete?.();
  };

  return (
    <Panel className="p-5 sm:p-6" data-testid="admin-user-secrets-panel">
      <div className="mb-4">
        <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
          {t("admin.user.secrets.title")}
        </h3>
        <p className="mt-2 text-sm text-iron-300">
          {t("admin.user.secrets.description")}
        </p>
      </div>

      {query.isLoading ? (
        <div className="space-y-2" aria-label={t("admin.user.secrets.loading")}>
          <div className="v2-skeleton h-9 rounded" />
          <div className="v2-skeleton h-9 rounded" />
        </div>
      ) : query.error ? (
        <p className="text-sm text-red-200" role="alert">
          {t("admin.user.secrets.loadFailed", { message: query.error.message })}
        </p>
      ) : (
        <div className="space-y-2">
          {secrets.length === 0 ? (
            <p className="py-2 text-sm text-iron-300">
              {t("admin.user.secrets.empty")}
            </p>
          ) : secrets.map((secret) => (
            <div
              key={secret.handle}
              data-testid="admin-secret-row"
              data-secret-handle={secret.handle}
              className="flex items-center justify-between gap-3 rounded-lg border border-iron-700 bg-iron-800/40 px-3 py-2"
            >
              <code className="min-w-0 truncate text-xs text-iron-100">
                {secret.handle}
              </code>
              <div className="flex shrink-0 items-center gap-1">
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  data-testid="admin-secret-replace"
                  data-secret-handle={secret.handle}
                  disabled={isMutating}
                  onClick={() => beginReplace(secret.handle)}
                >
                  {t("admin.user.secrets.replace")}
                </Button>
                <Button
                  type="button"
                  variant="danger"
                  size="sm"
                  data-testid="admin-secret-delete"
                  data-secret-handle={secret.handle}
                  disabled={isMutating}
                  onClick={() => beginDelete(secret.handle)}
                >
                  {t("admin.user.secrets.delete")}
                </Button>
              </div>
            </div>
          ))}
        </div>
      )}

      <form onSubmit={handleSubmit} className="mt-5 space-y-4">
        <div className="grid gap-4 sm:grid-cols-2">
          <div>
            <label htmlFor="admin-secret-handle" className="mb-1 block text-xs text-iron-300">
              {t("admin.user.secrets.handle")}
            </label>
            <Input
              id="admin-secret-handle"
              data-testid="admin-secret-handle"
              size="sm"
              value={handle}
              onChange={(event) => {
                setHandle(event.currentTarget.value);
                setSuccess("");
                resetPut?.();
              }}
              autoComplete="off"
              spellCheck={false}
              required
            />
          </div>
          <div>
            <label htmlFor="admin-secret-value" className="mb-1 block text-xs text-iron-300">
              {t("admin.user.secrets.value")}
            </label>
            <Input
              id="admin-secret-value"
              data-testid="admin-secret-value"
              size="sm"
              type="password"
              value={value}
              onChange={(event) => {
                setValue(event.currentTarget.value);
                setSuccess("");
                resetPut?.();
              }}
              autoComplete="new-password"
              spellCheck={false}
              required
            />
          </div>
        </div>
        <p className="text-xs text-iron-300">
          {t("admin.user.secrets.writeOnlyHint")}
        </p>
        <Button
          type="submit"
          size="sm"
          loading={isSaving}
          disabled={isMutating || !normalizedHandle || value.length === 0}
          data-testid="admin-secret-save"
        >
          {isSaving
            ? t("admin.user.secrets.saving")
            : t("admin.user.secrets.save")}
        </Button>
      </form>

      {success && (
        <p
          className="mt-4 text-sm text-signal"
          role="status"
          data-testid="admin-secret-status"
        >
          {success}
        </p>
      )}
      {putError && (
        <p className="mt-4 text-sm text-red-200" role="alert">
          {t("admin.user.secrets.actionFailed", { message: putError.message })}
        </p>
      )}

      {pendingDelete && (
        <Modal
          open
          onClose={closeDelete}
          title={t("admin.user.secrets.deleteTitle")}
          size="sm"
        >
          <ModalBody>
            <div data-testid="admin-secret-delete-dialog">
              <p className="mt-2 text-sm text-iron-300">
                {t("admin.user.secrets.deleteDesc", { handle: pendingDelete })}
              </p>
              {deleteError && (
                <p className="mt-4 text-sm text-red-200" role="alert">
                  {t("admin.user.secrets.actionFailed", { message: deleteError.message })}
                </p>
              )}
            </div>
          </ModalBody>
          <ModalFooter>
            <Button
              type="button"
              variant="ghost"
              disabled={isDeleting}
              data-testid="admin-secret-delete-cancel"
              onClick={closeDelete}
            >
              {t("admin.users.cancel")}
            </Button>
            <Button
              type="button"
              variant="danger"
              loading={isDeleting}
              disabled={isMutating}
              data-testid="admin-secret-delete-confirm"
              onClick={handleDelete}
            >
              {isDeleting
                ? t("admin.user.secrets.deleting")
                : t("admin.user.secrets.delete")}
            </Button>
          </ModalFooter>
        </Modal>
      )}
    </Panel>
  );
}
