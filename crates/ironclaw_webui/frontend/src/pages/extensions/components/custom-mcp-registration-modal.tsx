import React from "react";
import { Button } from "../../../design-system/button";
import { Modal, ModalBody, ModalFooter } from "../../../design-system/modal";
import { useT } from "../../../lib/i18n";

export type CustomMcpAuthKind = "no_auth" | "bearer" | "oauth";

export type CustomMcpAuthSelection = { kind: CustomMcpAuthKind };

// This is the projection shape consumed by the shared ConfigureModal. Keep it
// deliberately structural: the registration modal does not own lifecycle
// state, it merely passes the authoritative extension projection along.
export type CustomMcpSetupExtension = {
  packageRef?: string | { id?: string };
  package_ref?: { id?: string };
  displayName?: string;
  display_name?: string;
  installation_state?: string;
};

export type CustomMcpRegistrationPayload = {
  desiredId: string;
  desiredName: string;
  endpoint: string;
  authSelection: CustomMcpAuthSelection;
  onNeedsSetup: (extension: CustomMcpSetupExtension) => void;
  onRegistered: (extension: CustomMcpSetupExtension | null) => void;
  onRegistrationError: (message: string) => void;
};

type CustomMcpRegistrationModalProps = {
  open: boolean;
  onClose: () => void;
  onRegister: (payload: CustomMcpRegistrationPayload) => void;
  isRegistering: boolean;
  onSetup: (extension: CustomMcpSetupExtension) => void;
};

type RegistrationResult =
  | { kind: "active" }
  | { kind: "setup_needed"; extension: CustomMcpSetupExtension };

type FieldProps = {
  label: string;
  value: string;
  onChange: (value: string) => void;
  type?: React.HTMLInputTypeAttribute;
  placeholder?: string;
};

const AUTH_OPTIONS: CustomMcpAuthKind[] = ["no_auth", "bearer", "oauth"];

export function CustomMcpRegistrationModal({
  open,
  onClose,
  onRegister,
  isRegistering,
  onSetup,
}: CustomMcpRegistrationModalProps) {
  const t = useT();
  const [step, setStep] = React.useState<1 | 2 | 3>(1);
  const [desiredName, setDesiredName] = React.useState("");
  const [desiredId, setDesiredId] = React.useState("");
  const [endpoint, setEndpoint] = React.useState("");
  const [authKind, setAuthKind] = React.useState<CustomMcpAuthKind>("no_auth");
  const [error, setError] = React.useState("");
  const [result, setResult] = React.useState<RegistrationResult | null>(null);

  React.useEffect(() => {
    if (!open) {
      setStep(1);
      setError("");
      setResult(null);
    }
  }, [open]);

  const continueToAuthentication = () => {
    if (!desiredName.trim() || !desiredId.trim() || !endpoint.trim()) {
      setError(t("extensions.customMcpRequired"));
      return;
    }
    try {
      if (new URL(endpoint).protocol !== "https:") throw new Error("not HTTPS");
    } catch {
      setError(t("extensions.customMcpHttpsOnly"));
      return;
    }
    setError("");
    setStep(2);
  };

  const submit = () => {
    setError("");
    onRegister({
      desiredName: desiredName.trim(),
      desiredId: desiredId.trim(),
      endpoint: endpoint.trim(),
      authSelection: { kind: authKind },
      onNeedsSetup: (extension) => {
        setResult({ kind: "setup_needed", extension });
        setStep(3);
      },
      onRegistered: () => {
        setResult({ kind: "active" });
        setStep(3);
      },
      onRegistrationError: setError,
    });
  };

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={t("extensions.addCustomMcp")}
      closeLabel={t("common.close")}
      size="lg"
    >
      <ModalBody>
        <p className="mb-4 text-sm text-[var(--v2-text-muted)]">
          {step === 1
            ? t("extensions.customMcpPhase.connection")
            : step === 2
              ? t("extensions.customMcpPhase.authentication")
              : t("extensions.customMcpPhase.result")}
        </p>
        <div
          className="mb-5 flex gap-2"
          aria-label={t("extensions.customMcpProgress")}
        >
          {[1, 2, 3].map((value) => (
            <span
              key={value}
              className={`h-1.5 flex-1 rounded-full ${
                value <= step ? "bg-signal" : "bg-[var(--v2-panel-border)]"
              }`}
            />
          ))}
        </div>
        <ol className="mb-5 grid grid-cols-3 gap-2 text-xs text-[var(--v2-text-muted)]">
          <li className={step === 1 ? "text-[var(--v2-text-strong)]" : ""}>
            {t("extensions.customMcpPhase.connectionLabel")}
          </li>
          <li className={step === 2 ? "text-[var(--v2-text-strong)]" : ""}>
            {t("extensions.customMcpPhase.authenticationLabel")}
          </li>
          <li className={step === 3 ? "text-[var(--v2-text-strong)]" : ""}>
            {t("extensions.customMcpPhase.resultLabel")}
          </li>
        </ol>

        {step === 1 ? (
          <>
            <Field
              label={t("extensions.customMcpName")}
              value={desiredName}
              onChange={setDesiredName}
            />
            <Field
              label={t("extensions.customMcpId")}
              value={desiredId}
              onChange={setDesiredId}
            />
            <Field
              label={t("extensions.customMcpEndpoint")}
              value={endpoint}
              onChange={setEndpoint}
              type="url"
              placeholder="https://mcp.example.com"
            />
          </>
        ) : step === 2 ? (
          <>
            <p className="mb-3 text-sm text-[var(--v2-text-muted)]">
              {t("extensions.customMcpAuthHint")}
            </p>
            {AUTH_OPTIONS.map((kind) => (
              <label
                key={kind}
                className="mb-2 flex cursor-pointer items-center gap-3 rounded-md border border-[var(--v2-panel-border)] p-3"
              >
                <input
                  type="radio"
                  name="custom-mcp-auth"
                  checked={authKind === kind}
                  onChange={() => setAuthKind(kind)}
                />
                {t(`extensions.customMcpAuth.${kind}`)}
              </label>
            ))}
          </>
        ) : result?.kind === "setup_needed" ? (
          <div role="status" className="rounded-md border border-[var(--v2-panel-border)] p-4">
            <h3 className="font-semibold text-[var(--v2-text-strong)]">
              {t("extensions.customMcpSetupRequired")}
            </h3>
            <p className="mt-2 text-sm text-[var(--v2-text-muted)]">
              {t("extensions.customMcpSetupRequiredHint")}
            </p>
          </div>
        ) : (
          <div role="status" className="rounded-md border border-[var(--v2-panel-border)] p-4">
            <h3 className="font-semibold text-[var(--v2-text-strong)]">
              {t("extensions.customMcpReady")}
            </h3>
            <p className="mt-2 text-sm text-[var(--v2-text-muted)]">
              {t("extensions.customMcpReadyHint")}
            </p>
          </div>
        )}
        {error && (
          <p role="alert" className="mt-3 text-sm text-[var(--v2-danger-text)]">
            {error}
          </p>
        )}
      </ModalBody>
      <ModalFooter>
        {step === 2 && (
          <Button variant="ghost" onClick={() => setStep(1)} disabled={isRegistering}>
            {t("common.back")}
          </Button>
        )}
        {step < 3 && (
          <Button variant="ghost" onClick={onClose} disabled={isRegistering}>
            {t("common.cancel")}
          </Button>
        )}
        {step < 3 ? (
          <Button
            variant="primary"
            onClick={step === 1 ? continueToAuthentication : submit}
            loading={isRegistering}
          >
            {step === 1 ? t("common.continue") : t("extensions.customMcpRegister")}
          </Button>
        ) : result?.kind === "setup_needed" ? (
          <>
            <Button variant="ghost" onClick={onClose}>
              {t("extensions.customMcpFinishLater")}
            </Button>
            <Button variant="primary" onClick={() => onSetup(result.extension)}>
              {t("extensions.customMcpContinueSetup")}
            </Button>
          </>
        ) : (
          <Button variant="primary" onClick={onClose}>
            {t("common.done")}
          </Button>
        )}
      </ModalFooter>
    </Modal>
  );
}

function Field({
  label,
  value,
  onChange,
  type = "text",
  placeholder = "",
}: FieldProps) {
  return (
    <label className="mb-3 block text-sm font-medium">
      {label}
      <input
        type={type}
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.currentTarget.value)}
        className="mt-1 h-10 w-full rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3"
      />
    </label>
  );
}
