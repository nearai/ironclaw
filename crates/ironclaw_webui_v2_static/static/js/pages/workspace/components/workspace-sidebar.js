import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { Panel } from "../../../design-system/primitives.js";
import { WorkspaceSearchResults, WorkspaceTree } from "./workspace-tree.js";

export function WorkspaceSidebar({
  search,
  onSearchChange,
  rootEntries,
  selectedPath,
  expandedPaths,
  searchResults,
  isLoadingTree,
  isSearching,
  onToggleDirectory,
  onSelectFile,
}) {
  const t = useT();
  const hasSearch = search.trim().length > 0;

  return html`
    <${Panel} className="flex min-h-[420px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="border-b border-white/10 p-3">
        <input
          value=${search}
          onInput=${(event) => onSearchChange(event.target.value)}
          placeholder=${t("workspace.searchPlaceholder")}
          className="h-11 w-full rounded-md border border-white/10 bg-iron-950/80 px-3 text-sm text-white outline-none placeholder:text-iron-400 focus:border-signal/45"
        />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        ${hasSearch
          ? html`
              <${WorkspaceSearchResults}
                results=${searchResults}
                query=${search}
                onSelectFile=${onSelectFile}
                isSearching=${isSearching}
              />
            `
          : html`
              <${WorkspaceTree}
                entries=${rootEntries}
                selectedPath=${selectedPath}
                expandedPaths=${expandedPaths}
                onToggleDirectory=${onToggleDirectory}
                onSelectFile=${onSelectFile}
                isLoading=${isLoadingTree}
              />
            `}
      </div>
    <//>
  `;
}
