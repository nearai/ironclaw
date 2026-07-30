import { Card, SearchInput } from "@ironclaw/ui";
import { useT } from "../../../lib/i18n";
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
    <Card className="flex min-h-[420px] flex-col overflow-hidden xl:min-h-0">
      <div className="border-b border-[var(--v2-panel-border)] p-3">
        <SearchInput
          label={t("workspace.filterPlaceholder")}
          value={filter}
          onInput={(event) => onFilterChange(event.currentTarget.value)}
          placeholder={t("workspace.filterPlaceholder")}
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
    </Card>
  );
}
