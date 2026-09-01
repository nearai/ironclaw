import { useT } from "../../../lib/i18n";
import { Panel } from "../../../design-system/primitives";
import { SearchField } from "../../../design-system/search-field";
import { WorkspaceTree } from "./workspace-tree";

// Read-only navigation rail. The tree is rooted at the mount list (memory,
// workspace, …), so its top level is the mount picker; expanding a mount
// drills into its directories. The filter narrows the loaded tree by name.
export function WorkspaceSidebar({
  rootEntries,
  selectedPath,
  expandedPaths,
  filter,
  scopeKey,
  listDirectory,
  onFilterChange,
  isLoadingTree,
  onToggleDirectory,
  onSelectFile,
}) {
  const t = useT();

  return (
    <Panel className="flex min-h-[420px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="border-b border-white/10 p-3">
        <SearchField
          value={filter}
          onChange={onFilterChange}
          onClear={() => onFilterChange("")}
          placeholder={t("workspace.filterPlaceholder")}
          aria-label={t("workspace.filterPlaceholder")}
          clearLabel={t("settings.clearSearch")}
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <WorkspaceTree
          entries={rootEntries}
          selectedPath={selectedPath}
          expandedPaths={expandedPaths}
          filter={filter}
          scopeKey={scopeKey}
          listDirectory={listDirectory}
          onToggleDirectory={onToggleDirectory}
          onSelectFile={onSelectFile}
          isLoading={isLoadingTree}
        />
      </div>
    </Panel>
  );
}
