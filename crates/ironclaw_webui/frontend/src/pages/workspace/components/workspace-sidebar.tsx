import { Card } from "@ironclaw/ui";
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
        {/*
         * Deliberately NOT the design-system <Input>: this filter is a one-off
         * on main (h-9 / rounded-md / 14px vs Input's sm at rounded-[10px] /
         * 12px), and this PR is a pure extraction — same pixels, tokens
         * underneath. Fold into an Input size when a redesign is on the table.
         */}
        <input
          className="h-9 w-full rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] px-3 text-sm text-[var(--v2-text-strong)] outline-none placeholder:text-[var(--v2-text-faint)] focus:border-[var(--v2-accent)]"
          value={filter}
          onInput={(event) => onFilterChange(event.currentTarget.value)}
          placeholder={t("workspace.filterPlaceholder")}
          aria-label={t("workspace.filterPlaceholder")}
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
