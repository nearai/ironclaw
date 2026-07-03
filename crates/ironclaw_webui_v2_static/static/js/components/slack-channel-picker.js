import { React, html } from "../lib/html.js";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../design-system/button.js";
import { useT } from "../lib/i18n.js";
import {
  listSlackAllowedChannels,
  listSlackRoutableSubjects,
  normalizeSlackChannelIds,
  saveSlackAllowedChannels,
  slackChannelPickerError,
} from "../lib/slack-channels-api.js";

const QUERY_KEY = ["slack-allowed-channels"];

export function SlackChannelPicker({ action }) {
  const t = useT();
  const queryClient = useQueryClient();
  const [draftChannelId, setDraftChannelId] = React.useState("");
  const [draftSubjectUserId, setDraftSubjectUserId] = React.useState("");
  const [channels, setChannels] = React.useState([]);
  const copy = slackChannelPickerCopy(action, t);

  const channelsQuery = useQuery({
    queryKey: QUERY_KEY,
    queryFn: listSlackAllowedChannels,
  });
  const subjectsQuery = useQuery({
    queryKey: ["slack-routable-subjects"],
    queryFn: listSlackRoutableSubjects,
  });
  const subjects = subjectsQuery.data?.subjects || [];
  const subjectOptions = subjectOptionsForChannel(subjects);
  const subjectsSettled = subjectsQuery.isSuccess || subjectsQuery.isError;
  const hasRoutableSubjects = subjects.length > 0;

  React.useEffect(() => {
    if (!channelsQuery.data) return;
    setChannels(normalizeSlackChannels(channelsQuery.data.channels || []));
  }, [channelsQuery.data]);

  const saveMutation = useMutation({
    mutationFn: ({ channels }) => saveSlackAllowedChannels(channels),
    onSuccess: (data) => {
      setChannels(normalizeSlackChannels(data.channels || []));
      queryClient.invalidateQueries({ queryKey: QUERY_KEY });
      queryClient.invalidateQueries({ queryKey: ["slack-routable-subjects"] });
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
      queryClient.invalidateQueries({ queryKey: ["connectable-channels"] });
    },
  });

  const addChannel = () => {
    const nextId = draftChannelId.trim();
    if (!nextId || !subjectsQuery.isSuccess) return;
    setChannels((channels) =>
      normalizeSlackChannels([
        ...channels,
        { channel_id: nextId, subject_user_id: draftSubjectUserId },
      ]),
    );
    setDraftChannelId("");
  };

  const removeChannel = (channelId) => {
    setChannels((channels) => channels.filter((channel) => channel.channel_id !== channelId));
  };

  const updateChannelSubject = (channelId, subjectUserId) => {
    setChannels((channels) =>
      channels.map((channel) =>
        channel.channel_id === channelId
          ? { ...channel, subject_user_id: subjectUserId }
          : channel,
      ),
    );
  };

  const saveChannels = () => {
    saveMutation.mutate({ channels: persistedSlackChannels(channels) });
  };
  const hasBlankSubjectDuringCatalogError =
    subjectsQuery.isError && channels.some((channel) => !channel.subject_user_id);

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
        ${channelsQuery.data?.team_id &&
        html`<span className="shrink-0 rounded-md border border-white/[0.08] px-2 py-1 font-mono text-[10px] text-iron-500">
          ${channelsQuery.data.team_id}
        </span>`}
      </div>

      <div className="mb-3 flex flex-col gap-2 sm:flex-row sm:items-center">
        <input
          type="text"
          value=${draftChannelId}
          onChange=${(event) => setDraftChannelId(event.target.value)}
          onKeyDown=${(event) => event.key === "Enter" && addChannel()}
          placeholder=${copy.inputPlaceholder}
          className="h-9 min-w-0 flex-1 rounded-md border border-white/12 bg-white/[0.04] px-3 font-mono text-sm text-iron-100 outline-none placeholder:text-iron-700 focus:border-signal/45"
        />
        <select
          value=${draftSubjectUserId}
          onChange=${(event) => setDraftSubjectUserId(event.target.value)}
          disabled=${!hasRoutableSubjects}
          className="h-9 min-w-0 rounded-md border border-white/12 bg-white/[0.04] px-3 text-sm text-iron-100 outline-none focus:border-signal/45"
        >
          ${!hasRoutableSubjects &&
          html`<option value="">${copy.noSubjectsLabel}</option>`}
          ${hasRoutableSubjects &&
          html`<option value="">${copy.autoSubjectLabel}</option>`}
          ${subjectOptions.map(
            (subject) => html`
              <option key=${subject.subject_user_id} value=${subject.subject_user_id}>
                ${subject.display_name}
              </option>
            `,
          )}
        </select>
        <${Button}
          variant="secondary"
          size="sm"
          className="shrink-0"
          onClick=${addChannel}
          disabled=${!draftChannelId.trim() || !subjectsQuery.isSuccess}
        >
          ${copy.addLabel}
        <//>
      </div>

      <div className="mb-3 rounded-lg border border-white/[0.06] bg-black/10">
        ${channelsQuery.isLoading &&
        html`<div className="px-3 py-2 text-xs text-iron-400">${copy.loadingMessage}</div>`}
        ${!channelsQuery.isLoading &&
        channels.length === 0 &&
        html`<div className="px-3 py-2 text-xs text-iron-500">
          ${copy.emptyMessage}
        </div>`}
        ${channels.map(
          (channel) => html`
            <label
              key=${channel.channel_id}
              className="flex min-h-10 items-center justify-between gap-3 border-t border-white/[0.05] px-3 first:border-t-0"
            >
              <span className="min-w-0">
                <span className="block truncate font-mono text-xs text-iron-200">
                  ${channel.channel_id}
                </span>
              </span>
              <div className="flex shrink-0 items-center gap-2">
                ${hasRoutableSubjects
                  ? html`
                    <select
                      value=${channel.subject_user_id}
                      onChange=${(event) =>
                        updateChannelSubject(channel.channel_id, event.target.value)}
                      className="h-8 rounded-md border border-white/10 bg-white/[0.04] px-2 text-xs text-iron-100 outline-none focus:border-signal/45"
                    >
                      <option value="">${copy.autoSubjectLabel}</option>
                      ${subjectOptionsForChannel(subjects, channel).map(
                        (subject) => html`
                          <option key=${subject.subject_user_id} value=${subject.subject_user_id}>
                            ${subject.display_name}
                          </option>
                        `,
                      )}
                    </select>
                  `
                  : html`<span className="max-w-40 truncate text-xs text-iron-500">
                    ${channel.subject_user_id
                      ? channel.subject_display_name || channel.subject_user_id
                      : copy.autoSubjectLabel}
                  </span>`}
                <input
                  type="checkbox"
                  checked=${true}
                  aria-label=${copy.allowLabel(channel.channel_id)}
                  onChange=${() => removeChannel(channel.channel_id)}
                  className="h-4 w-4 rounded border-white/20 bg-white/[0.04] text-signal"
                />
              </div>
            </label>
          `,
        )}
      </div>

      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <${Button}
          variant="primary"
          size="sm"
          className="shrink-0"
          onClick=${saveChannels}
          disabled=${!channelsQuery.isSuccess ||
          !subjectsSettled ||
          saveMutation.isPending ||
          hasBlankSubjectDuringCatalogError}
        >
          ${saveMutation.isPending ? copy.savingLabel : copy.submitLabel}
        <//>
        ${saveMutation.isSuccess &&
        html`<p className="text-xs text-emerald-300">
          ${copy.successMessage}
        </p>`}
        ${(channelsQuery.isError || subjectsQuery.isError || saveMutation.isError) &&
        html`<p className="text-xs text-red-300">
          ${slackChannelPickerError(
            saveMutation.error || channelsQuery.error || subjectsQuery.error,
            copy.errorMessage,
          )}
        </p>`}
      </div>
    </div>
  `;
}

function subjectOptionsForChannel(subjects = [], channel = {}) {
  const bySubjectUserId = new Map();
  for (const subject of subjects) {
    const subjectUserId = String(subject.subject_user_id || "").trim();
    if (!subjectUserId) continue;
    bySubjectUserId.set(subjectUserId, {
      subject_user_id: subjectUserId,
      display_name: subject.display_name || subjectUserId,
    });
  }
  const subjectUserId = String(channel.subject_user_id || "").trim();
  if (subjectUserId && !bySubjectUserId.has(subjectUserId)) {
    bySubjectUserId.set(subjectUserId, {
      subject_user_id: subjectUserId,
      display_name: channel.subject_display_name || subjectUserId,
    });
  }
  return Array.from(bySubjectUserId.values()).sort((left, right) =>
    left.display_name.localeCompare(right.display_name) ||
    left.subject_user_id.localeCompare(right.subject_user_id),
  );
}

function normalizeSlackChannels(channels = []) {
  const byChannelId = new Map();
  for (const channel of channels) {
    const channelId = String(channel.channel_id || "").trim();
    if (!channelId) continue;
    const normalized = {
      channel_id: channelId,
      subject_user_id: String(channel.subject_user_id || "").trim(),
    };
    const subjectDisplayName = String(channel.subject_display_name || "").trim();
    if (subjectDisplayName) {
      normalized.subject_display_name = subjectDisplayName;
    }
    byChannelId.set(channelId, normalized);
  }
  return normalizeSlackChannelIds(Array.from(byChannelId.keys())).map((channelId) =>
    byChannelId.get(channelId),
  );
}

function persistedSlackChannels(channels = []) {
  return channels.map((channel) => ({
    channel_id: channel.channel_id,
    subject_user_id: channel.subject_user_id,
  }));
}

function slackChannelPickerCopy(action, t) {
  return {
    title: action?.title || t("channels.slackAccessTitle"),
    instructions:
      action?.instructions || t("channels.slackAccessInstructions"),
    inputPlaceholder: action?.input_placeholder || action?.code_placeholder || "C0123456789",
    addLabel: t("channels.slackAccessAdd"),
    loadingMessage: t("channels.slackAccessLoading"),
    emptyMessage: t("channels.slackAccessEmpty"),
    submitLabel: action?.submit_label || t("channels.slackAccessSave"),
    savingLabel: t("channels.slackAccessSaving"),
    successMessage: action?.success_message || t("channels.slackAccessSuccess"),
    errorMessage: action?.error_message || t("channels.slackAccessError"),
    autoSubjectLabel: t("channels.slackAccessAutoSubject"),
    noSubjectsLabel: t("channels.slackAccessNoSubjects"),
    allowLabel: (channelId) => t("channels.slackAccessAllow", { channelId }),
  };
}
