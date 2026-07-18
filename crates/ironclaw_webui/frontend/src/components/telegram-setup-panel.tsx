// @ts-nocheck
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../design-system/button";
import React from "react";
import { useT } from "../lib/i18n";
import {
  clearTelegramSetup,
  getTelegramSetup,
  saveTelegramSetup,
  telegramSetupError,
} from "../lib/telegram-setup-api";

const QUERY_KEY = ["telegram-setup"];
const FIELD_HELP = {
  botToken: {
    bodyKey: "telegramSetup.help.botToken",
    exampleKey: "telegramSetup.example.botToken",
  },
  webhookUrl: {
    bodyKey: "telegramSetup.help.webhookUrl",
    exampleKey: "telegramSetup.example.webhookUrl",
  },
};

export function TelegramAdminManagedSection({ action }) {
  const setupQuery = useQuery({
    queryKey: QUERY_KEY,
    queryFn: getTelegramSetup,
  });

  return (<TelegramSetupPanel action={action} setupQuery={setupQuery} />);
}

export function TelegramSetupPanel({ action, setupQuery }) {
  const t = useT();
  const queryClient = useQueryClient();
  const [form, setForm] = React.useState(emptyForm());
  const adoptedRevisionRef = React.useRef(-1);
  const dirtyRef = React.useRef(false);
  const status = setupQuery.data;
  const copy = telegramSetupCopy(action, t);

  React.useEffect(() => {
    if (!status || dirtyRef.current) return;
    const revision = setupStatusRevision(status);
    if (revision < adoptedRevisionRef.current) return;
    setForm(formFromStatus(status));
    adoptedRevisionRef.current = revision;
  }, [status]);

  const refreshConnectionQueries = () => {
    queryClient.invalidateQueries({ queryKey: QUERY_KEY });
    queryClient.invalidateQueries({ queryKey: ["connectable-channels"] });
    queryClient.invalidateQueries({ queryKey: ["extensions"] });
  };

  const saveMutation = useMutation({
    mutationFn: saveTelegramSetup,
    onSuccess: (data) => {
      dirtyRef.current = false;
      setForm(formFromStatus(data));
      adoptedRevisionRef.current = setupStatusRevision(data);
      queryClient.setQueryData(QUERY_KEY, data);
      refreshConnectionQueries();
    },
  });

  const removeMutation = useMutation({
    mutationFn: clearTelegramSetup,
    onSuccess: () => {
      dirtyRef.current = false;
      // Let the refetched (now unconfigured) status re-initialize the form.
      adoptedRevisionRef.current = -1;
      setForm(emptyForm());
      refreshConnectionQueries();
    },
  });

  const update = (field) => (event) => {
    const value = event.currentTarget.value;
    dirtyRef.current = true;
    // A prior save's success note must not survive edits: UI success follows
    // backend evidence, and the edited values have none yet.
    saveMutation.reset();
    setForm((current) => ({ ...current, [field]: value }));
  };

  // Save and removal are mutually exclusive: concurrent PUT/DELETE would make
  // the persisted setup depend on completion order.
  const mutationPending = saveMutation.isPending || removeMutation.isPending;
  const save = () => {
    if (mutationPending) return;
    saveMutation.mutate(form);
  };
  const remove = () => {
    if (mutationPending) return;
    if (!window.confirm(t("telegramSetup.removeConfirm"))) return;
    removeMutation.mutate();
  };
  // A blank token is only submittable when a saved one already exists
  // ("leave blank to keep"); the webhook override is always optional.
  const canSave = Boolean(status?.bot_token_configured || form.bot_token.trim());

  return (
    <div className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4">
      <div className="mb-3 flex items-start justify-between gap-3">
        <div>
          <h4 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            {copy.title}
          </h4>
          <p className="mt-2 text-xs leading-5 text-iron-300">
            {copy.instructions}
          </p>
        </div>
        {status?.configured &&
        (<span className="shrink-0 rounded-md border border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] px-2 py-1 text-[10px] text-[var(--v2-positive-text)]">
          {t("common.configured")}
        </span>)}
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        {secretInput(
          t("telegramSetup.field.botToken"),
          form.bot_token,
          update("bot_token"),
          status?.bot_token_configured,
          FIELD_HELP.botToken,
          t,
        )}
        {textInput(
          t("telegramSetup.field.webhookUrl"),
          form.webhook_url,
          update("webhook_url"),
          t("common.optional"),
          FIELD_HELP.webhookUrl,
          t,
        )}
      </div>

      {status?.configured && status?.bot_username &&
      (<p className="mt-3 text-xs text-iron-300">
        {t("telegramSetup.connectedAs")}{" "}
        <span data-testid="telegram-bot-username" className="font-mono text-iron-100">
          @{status.bot_username}
        </span>
      </p>)}

      <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-2">
          <Button
            variant="primary"
            size="sm"
            className="shrink-0"
            onClick={save}
            disabled={!canSave || mutationPending}
          >
            {saveMutation.isPending ? t("common.saving") : copy.submitLabel}
          </Button>
          {status?.configured &&
          (
            <Button
              variant="danger"
              size="sm"
              className="shrink-0"
              onClick={remove}
              loading={removeMutation.isPending}
              disabled={mutationPending}
              data-testid="telegram-remove-bot"
            >
              {t("telegramSetup.remove")}
            </Button>
          )}
        </div>
        {setupQuery.isError &&
        (<p className="text-xs text-red-300">
          {telegramSetupError(setupQuery.error, copy.errorMessage)}
        </p>)}
        {saveMutation.isError &&
        (<p className="text-xs text-red-300">
          {telegramSetupError(saveMutation.error, copy.errorMessage)}
        </p>)}
        {removeMutation.isError &&
        (<p className="text-xs text-red-300">
          {telegramSetupError(removeMutation.error, t("telegramSetup.removeFailed"))}
        </p>)}
        {saveMutation.isSuccess &&
        (<p className="text-xs text-[var(--v2-positive-text)]">{copy.successMessage}</p>)}
      </div>
    </div>
  );
}

// The saved bot token is write-only: the wire status only carries
// `bot_token_configured`, so the form's secret field always starts blank and a
// blank submit keeps the stored value.
function formFromStatus(status) {
  return {
    bot_token: "",
    webhook_url: status.webhook_url || "",
  };
}

function emptyForm() {
  return {
    bot_token: "",
    webhook_url: "",
  };
}

function setupStatusRevision(status) {
  const revision = Number(status?.revision);
  return Number.isSafeInteger(revision) && revision >= 0 ? revision : 0;
}

function translateOptional(t, key, fallback) {
  return typeof t === "function" ? t(key) : fallback;
}

function textInput(label, value, onChange, placeholder = "", help = null, t = null) {
  return (
    <label className="min-w-0">
      <span className="mb-1 block text-[11px] text-[var(--v2-text-muted)]">{label}</span>
      <input
        type="text"
        value={value}
        onChange={onChange}
        placeholder={placeholder}
        className="h-9 w-full min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
      />
      <FieldHint help={help} t={t} />
    </label>
  );
}

function secretInput(label, value, onChange, configured, help = null, t = null) {
  return (
    <label className="min-w-0">
      <span className="mb-1 block text-[11px] text-[var(--v2-text-muted)]">{label}</span>
      <input
        type="password"
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        value={value}
        onChange={onChange}
        placeholder={configured ? translateOptional(t, "telegramSetup.placeholder.keepSecret", "") : ""}
        className="h-9 w-full min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
      />
      <FieldHint help={help} t={t} />
    </label>
  );
}

function FieldHint({ help, t }) {
  if (!help) return null;
  const body = help.bodyKey ? translateOptional(t, help.bodyKey, help.body) : help.body;
  const example = help.exampleKey ? translateOptional(t, help.exampleKey, help.example) : help.example;
  return (
    <p className="mt-1.5 min-h-8 text-[11px] leading-4 text-iron-400">
      <span className="block">{body}</span>
      {example &&
      (<span className="mt-0.5 block font-mono text-iron-300">{example}</span>)}
    </p>
  );
}

function telegramSetupCopy(action, t) {
  return {
    title: action?.title || t("telegramSetup.title"),
    instructions: action?.instructions || t("telegramSetup.instructions"),
    submitLabel: action?.submit_label || t("telegramSetup.save"),
    successMessage: action?.success_message || t("telegramSetup.saved"),
    errorMessage: action?.error_message || t("telegramSetup.saveFailed"),
  };
}
