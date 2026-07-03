import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../design-system/button.js";
import { React, html } from "../lib/html.js";
import { getSlackSetup, saveSlackSetup, slackSetupError } from "../lib/slack-setup-api.js";
import { SlackChannelPicker } from "./slack-channel-picker.js";

const QUERY_KEY = ["slack-setup"];
const FIELD_HELP = {
  installationId: {
    body: "Local IronClaw name for this Slack install. Choose one and keep it stable.",
    example: "Example: local-slack",
  },
  teamId: {
    body: "Slack workspace/team ID from the workspace that installed the app.",
    example: "Example: T0123456789",
  },
  appId: {
    body: "Slack app Basic Information > App Credentials.",
    example: "Example: A0123456789",
  },
  botUser: {
    body: "Optional Reborn user. Blank uses the current WebUI operator.",
    example: "Example: user:operator",
  },
  sharedSubject: {
    body: "Optional default team agent for shared channel turns. Usually blank.",
    example: "Example: user:slack-shared",
  },
  botToken: {
    body: "Slack app OAuth & Permissions > Bot User OAuth Token.",
    example: "Example: xoxb-...",
  },
  signingSecret: {
    body: "Slack app Basic Information > App Credentials > Signing Secret.",
    example: "",
  },
};

export function SlackAdminManagedSection({ action }) {
  const setupQuery = useQuery({
    queryKey: QUERY_KEY,
    queryFn: getSlackSetup,
  });
  const configured = setupQuery.data?.configured === true;

  return html`
    <div className="space-y-3">
      <${SlackSetupPanel} action=${action} setupQuery=${setupQuery} />
      ${configured && html`<${SlackChannelPicker} action=${action} />`}
    </div>
  `;
}

export function SlackSetupPanel({ action, setupQuery }) {
  const queryClient = useQueryClient();
  const [form, setForm] = React.useState(emptyForm());
  const initializedRef = React.useRef(false);
  const dirtyRef = React.useRef(false);
  const status = setupQuery.data;
  const copy = slackSetupCopy(action);

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
    dirtyRef.current = true;
    setForm((current) => ({ ...current, [field]: event.target.value }));
  };

  const save = () => saveMutation.mutate(form);
  const canSave =
    form.installation_id.trim() &&
    form.team_id.trim() &&
    form.api_app_id.trim() &&
    (status?.bot_token_configured || form.bot_token.trim()) &&
    (status?.signing_secret_configured || form.signing_secret.trim());

  return html`
    <div className="mt-3 rounded-xl border border-white/[0.06] bg-white/[0.02] p-4">
      <div className="mb-3 flex items-start justify-between gap-3">
        <div>
          <h4 className="font-mono text-[11px] uppercase tracking-[0.14em] text-signal">
            ${copy.title}
          </h4>
          <p className="mt-2 text-xs leading-5 text-iron-300">
            ${copy.instructions}
          </p>
        </div>
        ${status?.configured &&
        html`<span className="shrink-0 rounded-md border border-emerald-400/20 px-2 py-1 text-[10px] text-emerald-300">
          Configured
        </span>`}
      </div>

      <div className="grid gap-3 sm:grid-cols-3">
        ${textInput(
          "Installation ID",
          form.installation_id,
          update("installation_id"),
          "",
          FIELD_HELP.installationId,
        )}
        ${textInput("Team ID", form.team_id, update("team_id"), "", FIELD_HELP.teamId)}
        ${textInput("App ID", form.api_app_id, update("api_app_id"), "", FIELD_HELP.appId)}
        ${textInput(
          "Bot user",
          form.user_id,
          update("user_id"),
          "default operator",
          FIELD_HELP.botUser,
        )}
        ${textInput(
          "Shared subject",
          form.shared_subject_user_id,
          update("shared_subject_user_id"),
          "optional",
          FIELD_HELP.sharedSubject,
        )}
      </div>

      <div className="mt-3 grid gap-3 sm:grid-cols-2">
        ${secretInput(
          "Bot token",
          form.bot_token,
          update("bot_token"),
          status?.bot_token_configured,
          FIELD_HELP.botToken,
        )}
        ${secretInput(
          "Signing secret",
          form.signing_secret,
          update("signing_secret"),
          status?.signing_secret_configured,
          FIELD_HELP.signingSecret,
        )}
      </div>

      <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <${Button}
          variant="primary"
          size="sm"
          className="shrink-0"
          onClick=${save}
          disabled=${!canSave || saveMutation.isPending}
        >
          ${saveMutation.isPending ? "Saving..." : copy.submitLabel}
        <//>
        ${setupQuery.isError &&
        html`<p className="text-xs text-red-300">
          ${slackSetupError(setupQuery.error, copy.errorMessage)}
        </p>`}
        ${saveMutation.isError &&
        html`<p className="text-xs text-red-300">
          ${slackSetupError(saveMutation.error, copy.errorMessage)}
        </p>`}
        ${saveMutation.isSuccess &&
        html`<p className="text-xs text-emerald-300">${copy.successMessage}</p>`}
      </div>
    </div>
  `;
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
  };
}

function textInput(label, value, onChange, placeholder = "", help = null) {
  return html`
    <label className="min-w-0">
      <span className="mb-1 block text-[11px] text-iron-500">${label}</span>
      <input
        type="text"
        value=${value}
        onChange=${onChange}
        placeholder=${placeholder}
        className="h-9 w-full min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
      />
      <${FieldHint} help=${help} />
    </label>
  `;
}

function secretInput(label, value, onChange, configured, help = null) {
  return html`
    <label className="min-w-0">
      <span className="mb-1 block text-[11px] text-iron-500">${label}</span>
      <input
        type="password"
        autoComplete="off"
        autoCapitalize="none"
        spellCheck=${false}
        value=${value}
        onChange=${onChange}
        placeholder=${configured ? "Configured; leave blank to keep" : ""}
        className="h-9 w-full min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
      />
      <${FieldHint} help=${help} />
    </label>
  `;
}

function FieldHint({ help }) {
  if (!help) return null;
  return html`
    <p className="mt-1.5 min-h-8 text-[11px] leading-4 text-iron-400">
      <span className="block">${help.body}</span>
      ${help.example &&
      html`<span className="mt-0.5 block font-mono text-iron-300">${help.example}</span>`}
    </p>
  `;
}

function slackSetupCopy(action) {
  return {
    title: "Slack setup",
    instructions: action?.instructions || "Configure the Slack app before assigning channels.",
    submitLabel: "Save setup",
    successMessage: "Slack setup saved.",
    errorMessage: "Slack setup update failed.",
  };
}
