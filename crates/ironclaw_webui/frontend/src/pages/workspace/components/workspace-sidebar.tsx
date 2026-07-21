import { useT } from "../../../lib/i18n";
import { Panel } from "../../../design-system/primitives";
import { WorkspaceTree } from "./workspace-tree";

// Read-only navigation rail. The tree is rooted at the mount list (memory,
// workspace, …), so its top level is the mount picker; expanding a mount
// drills into its directories. The filter narrows the loaded tree by name.
export function WorkspaceSidebar({
  rootEntries,
  selectedPath,
  expandedPaths,
  filter,
  onFilterChange,
  isLoadingTree,
  onToggleDirectory,
  onSelectFile,
}) {
  const t = useT();

  return (
    <Panel className="flex min-h-[420px] flex-col overflow-hidden p-0 xl:min-h-0">
      <div className="border-b border-white/10 p-3">
        <input
          value={filter}
          onInput={(event) => onFilterChange(event.currentTarget.value)}
          placeholder={t("workspace.filterPlaceholder")}
          className="h-9 w-full rounded-md border border-white/10 bg-iron-950/80 px-3 text-sm text-white outline-none placeholder:text-iron-400 focus:border-signal/45"
        />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <WorkspaceTree
          entries={rootEntries}
          selectedPath={selectedPath}
          expandedPaths={expandedPaths}
          filter={filter}
          onToggleDirectory={onToggleDirectory}
          onSelectFile={onSelectFile}
          isLoading={isLoadingTree}
        />
      </div>
    </Panel>
  );
}
