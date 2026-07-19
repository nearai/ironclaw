import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../design-system/button";
import React from "react";
import { useT } from "../lib/i18n";
import { getSlackSetup, saveSlackSetup, slackSetupError } from "../lib/slack-setup-api";
import { SlackChannelPicker } from "./slack-channel-picker";

const QUERY_KEY = ["slack-setup"];
const FIELD_HELP = {
  installationId: {
    bodyKey: "slackSetup.help.installationId",
    exampleKey: "slackSetup.example.localSlack",
  },
  teamId: {
    bodyKey: "slackSetup.help.teamId",
    exampleKey: "slackSetup.example.teamId",
  },
  appId: {
    bodyKey: "slackSetup.help.appId",
    exampleKey: "slackSetup.example.appId",
  },
  botUser: {
    bodyKey: "slackSetup.help.botUser",
    exampleKey: "slackSetup.example.botUser",
  },
  sharedSubject: {
    bodyKey: "slackSetup.help.sharedSubject",
    exampleKey: "slackSetup.example.sharedSubject",
  },
  botToken: {
    bodyKey: "slackSetup.help.botToken",
    exampleKey: "slackSetup.example.botToken",
  },
  signingSecret: {
    bodyKey: "slackSetup.help.signingSecret",
    exampleKey: "",
  },
  oauthClientId: {
    body: "Slack app OAuth & Permissions > App Credentials > Client ID. Required for personal (user-token) OAuth.",
    example: "Example: 123456789012.123456789012",
  },
  oauthClientSecret: {
    body: "Slack app OAuth & Permissions > App Credentials > Client Secret. Required for personal (user-token) OAuth.",
    example: "",
  },
};

export function SlackAdminManagedSection({ action }) {
  const setupQuery = useQuery({
    queryKey: QUERY_KEY,
    queryFn: getSlackSetup,
  });
  const configured = setupQuery.data?.configured === true;

  return (
    <div className="space-y-3">
      <SlackSetupPanel action={action} setupQuery={setupQuery} />
      {configured && (<SlackChannelPicker action={action} />)}
    </div>
  );
}

export function SlackSetupPanel({ action, setupQuery }) {
  const t = useT();
  const queryClient = useQueryClient();
  const [form, setForm] = React.useState(emptyForm());
  const initializedRef = React.useRef(false);
  const dirtyRef = React.useRef(false);
  const status = setupQuery.data;
  const copy = slackSetupCopy(action, t);

  React.useEffect(() => {
    if (!status || initializedRef.current || dirtyRef.current) return;
    setForm(formFromStatus(status));
    initializedRef.current = true;
  }, [status]);

  const saveMutation = useMutation({
    mutationFn: saveSlackSetup,
    onSuccess: (data) => {
      dirtyRef.current = false;
      setForm(formFromStatus(data));
      initializedRef.current = true;
      queryClient.setQueryData(QUERY_KEY, data);
      queryClient.invalidateQueries({ queryKey: QUERY_KEY });
      queryClient.invalidateQueries({ queryKey: ["slack-allowed-channels"] });
      queryClient.invalidateQueries({ queryKey: ["slack-routable-subjects"] });
      queryClient.invalidateQueries({ queryKey: ["connectable-channels"] });
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
    },
  });

  const update = (field) => (event) => {
    const value = event.currentTarget.value;
    dirtyRef.current = true;
    setForm((current) => ({ ...current, [field]: value }));
  };

  const save = () => saveMutation.mutate(form);
  const canSave =
    form.installation_id.trim() &&
    form.team_id.trim() &&
    form.api_app_id.trim() &&
    (status?.bot_token_configured || form.bot_token.trim()) &&
    (status?.signing_secret_configured || form.signing_secret.trim()) &&
    (status?.oauth_client_secret_configured || !form.oauth_client_id.trim() || form.oauth_client_secret.trim());

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

      <div className="grid gap-3 sm:grid-cols-3">
        {textInput(
          t("slackSetup.field.installationId"),
          form.installation_id,
          update("installation_id"),
          "",
          FIELD_HELP.installationId,
          t,
        )}
        {textInput(
          t("slackSetup.field.teamId"),
          form.team_id,
          update("team_id"),
          "",
          FIELD_HELP.teamId,
          t,
        )}
        {textInput(
          t("slackSetup.field.appId"),
          form.api_app_id,
          update("api_app_id"),
          "",
          FIELD_HELP.appId,
          t,
        )}
        {textInput(
          t("slackSetup.field.botUser"),
          form.user_id,
          update("user_id"),
          t("slackSetup.placeholder.defaultOperator"),
          FIELD_HELP.botUser,
          t,
        )}
        {textInput(
          t("slackSetup.field.sharedSubject"),
          form.shared_subject_user_id,
          update("shared_subject_user_id"),
          t("common.optional"),
          FIELD_HELP.sharedSubject,
          t,
        )}
      </div>

      <div className="mt-3 grid gap-3 sm:grid-cols-2">
        {secretInput(
          t("slackSetup.field.botToken"),
          form.bot_token,
          update("bot_token"),
          status?.bot_token_configured,
          FIELD_HELP.botToken,
          t,
        )}
        {secretInput(
          t("slackSetup.field.signingSecret"),
          form.signing_secret,
          update("signing_secret"),
          status?.signing_secret_configured,
          FIELD_HELP.signingSecret,
          t,
        )}
        {textInput(
          "OAuth client ID",
          form.oauth_client_id,
          update("oauth_client_id"),
          "optional",
          FIELD_HELP.oauthClientId,
        )}
        {secretInput(
          "OAuth client secret",
          form.oauth_client_secret,
          update("oauth_client_secret"),
          status?.oauth_client_secret_configured,
          FIELD_HELP.oauthClientSecret,
        )}
      </div>

      <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <Button
          variant="primary"
          size="sm"
          className="shrink-0"
          onClick={save}
          disabled={!canSave || saveMutation.isPending}
        >
          {saveMutation.isPending ? t("common.saving") : copy.submitLabel}
        </Button>
        {setupQuery.isError &&
        (<p className="text-xs text-red-300">
          {slackSetupError(setupQuery.error, copy.errorMessage)}
        </p>)}
        {saveMutation.isError &&
        (<p className="text-xs text-red-300">
          {slackSetupError(saveMutation.error, copy.errorMessage)}
        </p>)}
        {saveMutation.isSuccess &&
        (<p className="text-xs text-[var(--v2-positive-text)]">{copy.successMessage}</p>)}
      </div>
    </div>
  );
}

function formFromStatus(status) {
  return {
    installation_id: status.installation_id || "",
    team_id: status.team_id || "",
    api_app_id: status.api_app_id || "",
    user_id: status.user_id || "",
    shared_subject_user_id: status.shared_subject_user_id || "",
    bot_token: "",
    signing_secret: "",
    oauth_client_id: status.oauth_client_id || "",
    oauth_client_secret: "",
  };
}

function emptyForm() {
  return {
    installation_id: "",
    team_id: "",
    api_app_id: "",
    user_id: "",
    shared_subject_user_id: "",
    bot_token: "",
    signing_secret: "",
    oauth_client_id: "",
    oauth_client_secret: "",
  };
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
        placeholder={configured ? translateOptional(t, "slackSetup.placeholder.keepSecret", "") : ""}
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

function slackSetupCopy(action, t) {
  return {
    title: action?.title || t("slackSetup.title"),
    instructions: action?.instructions || t("slackSetup.instructions"),
    submitLabel: action?.submit_label || t("slackSetup.save"),
    successMessage: action?.success_message || t("slackSetup.saved"),
    errorMessage: action?.error_message || t("slackSetup.saveFailed"),
  };
}
