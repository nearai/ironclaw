// @ts-nocheck
import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { InlineNotice } from "../../../design-system/inline-notice";
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
      <InlineNotice
        tone="warning"
        role="alert"
        action={(
          <Button
            type="button"
            variant="secondary"
            size="sm"
            disabled={!restart.restartEnabled || restart.isRestarting}
            onClick={restart.openConfirm}
            title={!restart.restartEnabled ? restart.unavailableReason : undefined}
          >
            <Icon name={restart.isRestarting ? "pulse" : "bolt"} className="h-4 w-4" />
            {restart.isRestarting ? t("settings.restartStarting") : t("settings.restartNow")}
          </Button>
        )}
      >
        <p>{t("settings.restartRequired")}</p>
        {!restart.restartEnabled &&
        (
          <p className="mt-1 text-ui-sm text-[var(--v2-text-muted)]">
            {restart.unavailableReason}
          </p>
        )}
        {restart.isRestarting &&
        (
          <p className="mt-1 text-ui-sm text-[var(--v2-text-muted)]">
            {restart.progressLabel}
          </p>
        )}
      </InlineNotice>

      {restart.error &&
      (
        <InlineNotice tone="danger" role="alert">
          {restart.error}
        </InlineNotice>
      )}

      {restart.message &&
      (
        <InlineNotice tone="success" role="status">
          {restart.message}
        </InlineNotice>
      )}
    </div>

    <Modal
      open={restart.confirmOpen}
      onClose={restart.closeConfirm}
      title={t("restart.title")}
      size="sm"
    >
      <ModalBody className="space-y-3">
        <p className="text-ui text-[var(--v2-text)]">
          {t("restart.description")}
        </p>
        <div className="rounded-xl border border-copper/25 bg-[var(--v2-warning-soft)] px-3 py-2 text-ui-sm text-[var(--v2-warning-text)]">
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
          <div className="mx-auto grid h-12 w-12 place-items-center rounded-full border border-[color-mix(in_srgb,var(--v2-warning-text)_35%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]">
            <Icon name="pulse" className="h-5 w-5 animate-pulse" />
          </div>
          <p className="mt-4 text-ui-lg font-semibold text-[var(--v2-text-strong)]">
            {t("restart.progressTitle")}
          </p>
          <p className="mt-2 text-ui text-[var(--v2-text-muted)]">
            {restart.progressLabel}
          </p>
        </div>
      </div>
    )}
    </>
  );
}
