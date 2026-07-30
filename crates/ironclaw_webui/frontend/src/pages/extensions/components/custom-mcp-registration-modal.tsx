import React from "react";
import { Button } from "../../../design-system/button";
import { Modal, ModalBody, ModalFooter } from "../../../design-system/modal";
import { useT } from "../../../lib/i18n";

export type CustomMcpAuthKind = "auto";

export type CustomMcpAuthSelection = { kind: CustomMcpAuthKind };

export type CustomMcpRegistrationPayload = {
  desiredId: string;
  desiredName: string;
  endpoint: string;
  authSelection: CustomMcpAuthSelection;
  onRegistered: () => void;
  onRegistrationError: (message: string) => void;
};

type CustomMcpRegistrationModalProps = {
  open: boolean;
  onClose: () => void;
  onRegister: (payload: CustomMcpRegistrationPayload) => void;
  isRegistering: boolean;
};

type FieldProps = {
  label: string;
  value: string;
  onChange: (value: string) => void;
  type?: React.HTMLInputTypeAttribute;
  placeholder?: string;
  error?: string;
  hint?: string;
};

const EXTENSION_ID_PATTERN = /^[a-z0-9][a-z0-9_.-]{0,127}$/;
const CONTROL_CHARACTER_PATTERN = /[\u0000-\u001F\u007F]/;
const CREDENTIAL_QUERY_KEYS = new Set([
  "access_token", "token", "api_key", "apikey", "key", "secret", "authorization", "auth", "bearer",
]);

export function customMcpIdFromName(name: string) {
  const slug = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 128);
  return slug || "extension";
}

function connectionErrors(name: string, id: string, endpoint: string, t: (key: string) => string) {
  let endpointError = "";
  const trimmedEndpoint = endpoint.trim();
  if (!trimmedEndpoint) {
    endpointError = t("extensions.customMcpEndpointRequired");
  } else {
    try {
      const parsed = new URL(trimmedEndpoint);
      const host = parsed.hostname.toLowerCase();
      const isIpLiteral = /^\d{1,3}(?:\.\d{1,3}){3}$/.test(host) || host.includes(":");
      const hasCredentialQuery = Array.from(parsed.searchParams.keys()).some((key) =>
        CREDENTIAL_QUERY_KEYS.has(key.toLowerCase()),
      );
      if (
        parsed.protocol !== "https:" || !host || parsed.username || parsed.password || parsed.hash ||
        host === "localhost" || isIpLiteral || hasCredentialQuery ||
        trimmedEndpoint.length > 2048 || CONTROL_CHARACTER_PATTERN.test(trimmedEndpoint)
      ) throw new Error("invalid hosted MCP endpoint");
    } catch {
      endpointError = t("extensions.customMcpEndpointHttps");
    }
  }
  return {
    name: !name.trim()
      ? t("extensions.customMcpNameRequired")
      : name.length > 256
        ? t("extensions.customMcpNameTooLong")
        : CONTROL_CHARACTER_PATTERN.test(name)
          ? t("extensions.customMcpNameControls")
          : "",
    id: !id.trim()
      ? t("extensions.customMcpIdRequired")
      : !EXTENSION_ID_PATTERN.test(id) || id.split(".").some((segment) => !segment)
        ? t("extensions.customMcpIdInvalid")
        : "",
    endpoint: endpointError,
  };
}

export function CustomMcpRegistrationModal({
  open,
  onClose,
  onRegister,
  isRegistering,
}: CustomMcpRegistrationModalProps) {
  const t = useT();
  const [step, setStep] = React.useState<1 | 2 | 3>(1);
  const [desiredName, setDesiredName] = React.useState("");
  const [desiredId, setDesiredId] = React.useState("");
  const [isIdAdvanced, setIsIdAdvanced] = React.useState(false);
  const [endpoint, setEndpoint] = React.useState("");
  const [error, setError] = React.useState("");
  const [showConnectionErrors, setShowConnectionErrors] = React.useState(false);
  const [registered, setRegistered] = React.useState(false);

  React.useEffect(() => {
    if (!open) {
      setStep(1);
      setError("");
      setShowConnectionErrors(false);
      setRegistered(false);
    }
  }, [open]);

  const effectiveDesiredId = isIdAdvanced ? desiredId : customMcpIdFromName(desiredName);
  const errors = connectionErrors(desiredName, effectiveDesiredId, endpoint, t);

  const updateName = (name: string) => {
    setDesiredName(name);
    if (!isIdAdvanced) setDesiredId(customMcpIdFromName(name));
  };

  const continueToReview = () => {
    setShowConnectionErrors(true);
    if (errors.name || errors.id || errors.endpoint) {
      return;
    }
    setError("");
    setStep(2);
  };

  const submit = () => {
    setError("");
    onRegister({
      desiredName: desiredName.trim(),
      desiredId: effectiveDesiredId.trim(),
      endpoint: endpoint.trim(),
      authSelection: { kind: "auto" },
      onRegistered: () => {
        setRegistered(true);
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
              ? t("extensions.customMcpPhase.review")
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
            {t("extensions.customMcpPhase.reviewLabel")}
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
              onChange={updateName}
              hint={t("extensions.customMcpNameHint")}
              error={showConnectionErrors ? errors.name : ""}
            />
            <p className="-mt-2 mb-3 text-sm text-[var(--v2-text-muted)]">
              {t("extensions.customMcpIdGenerated", { id: effectiveDesiredId })}
            </p>
            <details className="mb-3 rounded-md border border-[var(--v2-panel-border)] p-3">
              <summary className="cursor-pointer text-sm font-medium text-[var(--v2-text-strong)]">
                {t("extensions.customMcpAdvanced")}
              </summary>
              <div className="mt-3">
                <Field
                  label={t("extensions.customMcpId")}
                  value={effectiveDesiredId}
                  onChange={(value) => {
                    setIsIdAdvanced(true);
                    setDesiredId(value);
                  }}
                  hint={t("extensions.customMcpIdHint")}
                  error={showConnectionErrors ? errors.id : ""}
                />
              </div>
            </details>
            <Field
              label={t("extensions.customMcpEndpoint")}
              value={endpoint}
              onChange={setEndpoint}
              type="url"
              placeholder="https://mcp.example.com"
              error={showConnectionErrors ? errors.endpoint : ""}
            />
          </>
        ) : step === 2 ? (
          <>
            <p className="mb-3 text-sm text-[var(--v2-text-muted)]">
              {t("extensions.customMcpReviewHint")}
            </p>
            <dl className="rounded-md border border-[var(--v2-panel-border)] p-3 text-sm">
              <div className="flex gap-3 py-1">
                <dt className="w-28 shrink-0 text-[var(--v2-text-muted)]">{t("extensions.customMcpName")}</dt>
                <dd className="break-all text-[var(--v2-text-strong)]">{desiredName.trim()}</dd>
              </div>
              <div className="flex gap-3 py-1">
                <dt className="w-28 shrink-0 text-[var(--v2-text-muted)]">{t("extensions.customMcpId")}</dt>
                <dd className="break-all text-[var(--v2-text-strong)]">{effectiveDesiredId}</dd>
              </div>
              <div className="flex gap-3 py-1">
                <dt className="w-28 shrink-0 text-[var(--v2-text-muted)]">{t("extensions.customMcpEndpoint")}</dt>
                <dd className="break-all text-[var(--v2-text-strong)]">{endpoint.trim()}</dd>
              </div>
            </dl>
          </>
        ) : registered ? (
          <div role="status" className="rounded-md border border-[var(--v2-panel-border)] p-4">
            <h3 className="font-semibold text-[var(--v2-text-strong)]">
              {t("extensions.customMcpReady")}
            </h3>
            <p className="mt-2 text-sm text-[var(--v2-text-muted)]">
              {t("extensions.customMcpReadyHint")}
            </p>
          </div>
        ) : null}
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
            onClick={step === 1 ? continueToReview : submit}
            loading={isRegistering}
          >
            {step === 1 ? t("common.continue") : t("extensions.customMcpRegister")}
          </Button>
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
  error = "",
  hint = "",
}: FieldProps) {
  const hintId = React.useId();
  const errorId = React.useId();
  return (
    <label className="mb-3 block text-sm font-medium">
      {label}
      <input
        type={type}
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.currentTarget.value)}
        aria-invalid={Boolean(error)}
        aria-describedby={error ? errorId : hint ? hintId : undefined}
        className="mt-1 h-10 w-full rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3"
      />
      {hint && !error && <span id={hintId} className="mt-1 block text-xs font-normal text-[var(--v2-text-muted)]">{hint}</span>}
      {error && <span id={errorId} role="alert" className="mt-1 block text-xs font-normal text-[var(--v2-danger-text)]">{error}</span>}
    </label>
  );
}
