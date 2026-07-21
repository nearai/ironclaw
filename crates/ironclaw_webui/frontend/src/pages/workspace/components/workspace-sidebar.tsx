import { useT } from "../../../lib/i18n";
import { Panel } from "@ironclaw/design-system";
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
      <div className="border-b border-[var(--v2-panel-border)] p-3">
        <input
          value={filter}
          onInput={(event) => onFilterChange(event.currentTarget.value)}
          placeholder={t("workspace.filterPlaceholder")}
          aria-label={t("workspace.filterPlaceholder")}
          className="h-9 w-full rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] px-3 text-sm text-[var(--v2-text-strong)] outline-none placeholder:text-[var(--v2-text-faint)] focus:border-[var(--v2-accent)]"
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
