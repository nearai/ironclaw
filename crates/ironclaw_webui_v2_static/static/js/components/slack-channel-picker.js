import { React, html } from "../lib/html.js";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../design-system/button.js";
import { useT } from "../lib/i18n.js";
import {
  listSlackAllowedChannels,
  normalizeSlackChannelIds,
  saveSlackAllowedChannels,
  slackChannelPickerError,
} from "../lib/slack-channels-api.js";

const QUERY_KEY = ["slack-allowed-channels"];

export function SlackChannelPicker({ action }) {
  const t = useT();
  const queryClient = useQueryClient();
  const [draftChannelId, setDraftChannelId] = React.useState("");
  const [channelIds, setChannelIds] = React.useState([]);
  const copy = slackChannelPickerCopy(action, t);

  const channelsQuery = useQuery({
    queryKey: QUERY_KEY,
    queryFn: listSlackAllowedChannels,
  });

  React.useEffect(() => {
    if (!channelsQuery.data) return;
    setChannelIds(
      normalizeSlackChannelIds(
        channelsQuery.data.channels?.map((channel) => channel.channel_id) || [],
      ),
    );
  }, [channelsQuery.data]);

  const saveMutation = useMutation({
    mutationFn: ({ ids }) => saveSlackAllowedChannels(ids),
    onSuccess: (data) => {
      setChannelIds(
        normalizeSlackChannelIds(data.channels?.map((channel) => channel.channel_id) || []),
      );
      queryClient.invalidateQueries({ queryKey: QUERY_KEY });
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
      queryClient.invalidateQueries({ queryKey: ["connectable-channels"] });
    },
  });

  const addChannel = () => {
    const nextId = draftChannelId.trim();
    if (!nextId) return;
    setChannelIds((ids) => normalizeSlackChannelIds([...ids, nextId]));
    setDraftChannelId("");
  };

  const removeChannel = (channelId) => {
    setChannelIds((ids) => ids.filter((id) => id !== channelId));
  };

  const saveChannels = () => {
    saveMutation.mutate({ ids: channelIds });
  };

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
        <${Button}
          variant="secondary"
          size="sm"
          className="shrink-0"
          onClick=${addChannel}
          disabled=${!draftChannelId.trim()}
        >
          ${copy.addLabel}
        <//>
      </div>

      <div className="mb-3 rounded-lg border border-white/[0.06] bg-black/10">
        ${channelsQuery.isLoading &&
        html`<div className="px-3 py-2 text-xs text-iron-400">${copy.loadingMessage}</div>`}
        ${!channelsQuery.isLoading &&
        channelIds.length === 0 &&
        html`<div className="px-3 py-2 text-xs text-iron-500">
          ${copy.emptyMessage}
        </div>`}
        ${channelIds.map(
          (channelId) => html`
            <label
              key=${channelId}
              className="flex min-h-10 items-center justify-between gap-3 border-t border-white/[0.05] px-3 first:border-t-0"
            >
              <span className="min-w-0 truncate font-mono text-xs text-iron-200">
                ${channelId}
              </span>
              <input
                type="checkbox"
                checked=${true}
                aria-label=${copy.allowLabel(channelId)}
                onChange=${() => removeChannel(channelId)}
                className="h-4 w-4 rounded border-white/20 bg-white/[0.04] text-signal"
              />
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
          disabled=${!channelsQuery.isSuccess || saveMutation.isPending}
        >
          ${saveMutation.isPending ? copy.savingLabel : copy.submitLabel}
        <//>
        ${saveMutation.isSuccess &&
        html`<p className="text-xs text-emerald-300">
          ${copy.successMessage}
        </p>`}
        ${(channelsQuery.isError || saveMutation.isError) &&
        html`<p className="text-xs text-red-300">
          ${slackChannelPickerError(
            saveMutation.error || channelsQuery.error,
            copy.errorMessage,
          )}
        </p>`}
      </div>
    </div>
  `;
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
    allowLabel: (channelId) => t("channels.slackAccessAllow", { channelId }),
  };
}
