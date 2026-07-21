// @ts-nocheck
import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { Modal, ModalBody, ModalFooter } from "../../../design-system/modal";
import { useT } from "../../../lib/i18n";
import { useGatewayRestart } from "../hooks/useGatewayRestart";

export function RestartBanner({ visible, gatewayStatus, gatewayStatusQuery }) {
  const t = useT();
  const restart = useGatewayRestart({ gatewayStatus, gatewayStatusQuery });

  if (!visible) return null;

  return (
    <>
    <div className="space-y-3">
      <div
        role="alert"
        className="flex flex-col gap-3 rounded-xl border border-copper/30 bg-copper/10 px-4 py-3 sm:flex-row sm:items-center"
      >
        <div className="flex min-w-0 flex-1 items-start gap-3">
          <Icon name="bolt" className="mt-0.5 h-4 w-4 shrink-0 text-copper" />
          <div className="min-w-0">
            <p className="text-sm text-copper">
              {t("settings.restartRequired")}
            </p>
            {!restart.restartEnabled &&
            (
              <p className="mt-1 text-xs text-[var(--v2-text-muted)]">
                {restart.unavailableReason}
              </p>
            )}
            {restart.isRestarting &&
            (
              <p className="mt-1 text-xs text-[var(--v2-text-muted)]">
                {restart.progressLabel}
              </p>
            )}
          </div>
        </div>

        <Button
          type="button"
          variant="secondary"
          size="sm"
          disabled={!restart.restartEnabled || restart.isRestarting}
          onClick={restart.openConfirm}
          title={!restart.restartEnabled ? restart.unavailableReason : undefined}
          className="w-full sm:w-auto"
        >
          <Icon name={restart.isRestarting ? "pulse" : "bolt"} className="h-4 w-4" />
          {restart.isRestarting ? t("settings.restartStarting") : t("settings.restartNow")}
        </Button>
      </div>

      {restart.error &&
      (
        <div className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">
          {restart.error}
        </div>
      )}

      {restart.message &&
      (
        <div className="rounded-xl border border-[color-mix(in_srgb,var(--v2-positive-text)_35%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] px-4 py-3 text-sm text-[var(--v2-positive-text)]">
          {restart.message}
        </div>
      )}
    </div>

    <Modal
      open={restart.confirmOpen}
      onClose={restart.closeConfirm}
      title={t("restart.title")}
      size="sm"
    >
      <ModalBody className="space-y-3">
        <p className="text-sm text-[var(--v2-text)]">
          {t("restart.description")}
        </p>
        <div className="rounded-xl border border-copper/25 bg-copper/10 px-3 py-2 text-xs text-copper">
          {t("restart.warning")}
        </div>
      </ModalBody>
      <ModalFooter>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          disabled={restart.isRestarting}
          onClick={restart.closeConfirm}
        >
          {t("restart.cancel")}
        </Button>
        <Button
          type="button"
          variant="danger"
          size="sm"
          disabled={restart.isRestarting}
          onClick={restart.confirmRestart}
        >
          <Icon name="bolt" className="h-4 w-4" />
          {t("restart.confirm")}
        </Button>
      </ModalFooter>
    </Modal>

    {restart.isRestarting &&
    (
      <div
        className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4 backdrop-blur-sm"
        role="status"
        aria-live="polite"
      >
        <div className="w-full max-w-sm rounded-[1.5rem] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] p-6 text-center shadow-[0_24px_60px_rgba(0,0,0,0.35)]">
          <div className="mx-auto grid h-12 w-12 place-items-center rounded-full border border-copper/30 bg-copper/10 text-copper">
            <Icon name="pulse" className="h-5 w-5 animate-pulse" />
          </div>
          <p className="mt-4 text-base font-semibold text-[var(--v2-text-strong)]">
            {t("restart.progressTitle")}
          </p>
          <p className="mt-2 text-sm text-[var(--v2-text-muted)]">
            {restart.progressLabel}
          </p>
        </div>
      </div>
    )}
    </>
  );
}
