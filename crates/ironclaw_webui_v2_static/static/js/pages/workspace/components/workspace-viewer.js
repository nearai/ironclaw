import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { Button } from "../../../design-system/button.js";
import { EmptyPanel, Panel, StatusPill } from "../../../design-system/primitives.js";
import { MarkdownRenderer } from "../../chat/components/markdown-renderer.js";
import {
  formatWorkspaceDate,
  isMarkdownPath,
  parentPath,
  pathSegments,
  routeForWorkspacePath,
} from "../lib/workspace-presenters.js";

function Breadcrumb({ path, onNavigate }) {
  const t = useT();
  const parts = pathSegments(path);
  let current = "";

  return html`
    <div className="flex min-w-0 flex-wrap items-center gap-2 font-mono text-sm">
      <button type="button" onClick=${() => onNavigate("/workspace")} className="text-signal hover:underline">${t("workspace.breadcrumbRoot")}</button>
      ${parts.map((part) => {
        current = current ? `${current}/${part}` : part;
        const target = current;
        return html`
          <span key=${target} className="text-iron-400">/</span>
          <button
            key=${`${target}-button`}
            type="button"
            onClick=${() => onNavigate(routeForWorkspacePath(target))}
            className="max-w-[220px] truncate text-signal hover:underline"
          >
            ${part}
          </button>
        `;
      })}
    </div>
  `;
}

export function WorkspaceViewer({
  path,
  file,
  draft,
  onDraftChange,
  editing,
  onStartEdit,
  onCancelEdit,
  onSave,
  isLoading,
  isSaving,
  onNavigate,
}) {
  const t = useT();
  if (isLoading) {
    return html`
      <div className="space-y-4">
        <div className="v2-skeleton h-16 rounded-xl" />
        <div className="v2-skeleton h-[460px] rounded-xl" />
      </div>
    `;
  }

  if (!file) {
    return html`
      <${EmptyPanel}
        title=${t("workspace.pickFileTitle")}
        description=${t("workspace.pickFileDesc")}
      />
    `;
  }

  return html`
    <${Panel} className="flex min-h-[520px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-white/10 px-4 py-3">
        <${Breadcrumb} path=${path} onNavigate=${onNavigate} />
        <div className="flex items-center gap-2">
          <${StatusPill} tone="muted" label=${formatWorkspaceDate(file.updated_at)} />
          ${editing
            ? html`
                <${Button} variant="ghost" size="sm" onClick=${onCancelEdit} disabled=${isSaving}>${t("workspace.cancel")}<//>
                <${Button} size="sm" onClick=${onSave} disabled=${isSaving}>${isSaving ? t("workspace.saving") : t("workspace.save")}<//>
              `
            : html`<${Button} variant="secondary" size="sm" onClick=${onStartEdit}>${t("workspace.edit")}<//>`}
        </div>
      </div>

      ${editing
        ? html`
            <div className="min-h-0 flex-1 p-4">
              <textarea
                value=${draft}
                onInput=${(event) => onDraftChange(event.target.value)}
                className="h-full min-h-[460px] w-full resize-none rounded-xl border border-white/10 bg-iron-950/80 p-4 font-mono text-sm leading-6 text-white outline-none focus:border-signal/45"
                spellCheck=${false}
              />
            </div>
          `
        : html`
            <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3 sm:px-6 sm:py-4">
              ${isMarkdownPath(path)
                ? html`<${MarkdownRenderer} content=${file.content} className="max-w-4xl text-base leading-7" />`
                : html`<pre className="overflow-x-auto whitespace-pre-wrap font-mono text-sm leading-6 text-iron-200">${file.content}</pre>`}
            </div>
          `}

      ${parentPath(path) && html`
        <div className="border-t border-white/10 px-4 py-3 text-xs text-iron-400">
          ${t("workspace.parent", { path: parentPath(path) })}
        </div>
      `}
    <//>
  `;
}
