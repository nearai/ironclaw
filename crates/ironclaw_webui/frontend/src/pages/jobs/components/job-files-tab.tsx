import { EmptyPanel, Panel } from "@ironclaw/design-system";

function TreeNodes({ nodes, depth = 0, selectedPath, expandingPath, onToggleDirectory, onSelectPath }) {
  return (
    <>
    {nodes.map((node) => (
      <div key={node.path}>
        <button
          onClick={() => (node.isDir ? onToggleDirectory(node.path) : onSelectPath(node.path))}
          className={[
            "flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-sm",
            selectedPath === node.path ? "bg-[var(--v2-accent-soft)] text-[var(--v2-text-strong)]" : "text-[var(--v2-text)] hover:bg-[var(--v2-surface-soft)]",
          ].join(" ")}
          style={{ paddingLeft: `${depth * 18 + 12}px` }}
        >
          <span className="w-4 text-center text-[var(--v2-text-muted)]">
            {node.isDir ? (expandingPath === node.path ? "..." : node.expanded ? "v" : ">") : "·"}
          </span>
          <span className={node.isDir ? "font-medium" : ""}>{node.name}</span>
        </button>
        {node.isDir && node.expanded && node.children?.length
          ? (<TreeNodes
              nodes={node.children}
              depth={depth + 1}
              selectedPath={selectedPath}
              expandingPath={expandingPath}
              onToggleDirectory={onToggleDirectory}
              onSelectPath={onSelectPath}
            />)
          : null}
      </div>
    ))}
    </>
  );
}

export function JobFilesTab({
  canBrowse,
  tree,
  selectedPath,
  selectedFile,
  fileError,
  isLoadingTree,
  isLoadingFile,
  expandingPath,
  treeError,
  onToggleDirectory,
  onSelectPath,
}) {
  if (!canBrowse) {
    return (
      <EmptyPanel
        title="No project workspace"
        description="File browsing is only available for sandbox jobs that produced a mounted project directory."
      />
    );
  }

  return (
    <div className="grid gap-5 xl:grid-cols-[320px_minmax(0,1fr)]">
      <Panel className="min-h-[440px] p-4">
        <div className="border-b border-[var(--v2-panel-border)] px-2 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[var(--v2-tracking-caps)] text-[var(--v2-text-muted)]">Workspace tree</div>
          <p className="mt-2 text-sm leading-6 text-[var(--v2-text-muted)]">Browse the sandbox output and inspect generated files inline.</p>
        </div>

        <div className="mt-3 max-h-[60vh] overflow-y-auto">
          {treeError && (<div className="mx-2 mb-3 rounded-md border border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] px-3 py-2 text-sm text-[var(--v2-danger-text)]">{treeError}</div>)}
          {isLoadingTree
            ? (<div className="space-y-2 px-2">{[1, 2, 3, 4].map((i) => (<div key={i} className="v2-skeleton h-8 rounded-md" />))}</div>)
            : tree.length
              ? (
                  <TreeNodes
                    nodes={tree}
                    selectedPath={selectedPath}
                    expandingPath={expandingPath}
                    onToggleDirectory={onToggleDirectory}
                    onSelectPath={onSelectPath}
                  />
                )
              : (<div className="px-2 py-6 text-sm text-[var(--v2-text-muted)]">No files were recorded for this workspace.</div>)}
        </div>
      </Panel>

      <Panel className="min-h-[440px] p-5 sm:p-6">
        <div className="border-b border-[var(--v2-panel-border)] pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[var(--v2-tracking-caps)] text-[var(--v2-text-muted)]">File preview</div>
          <p className="mt-2 break-all text-sm leading-6 text-[var(--v2-text-muted)]">{selectedFile?.path || selectedPath || "Select a file from the tree to inspect its contents."}</p>
        </div>

        {fileError && !isLoadingFile
          ? (<div className="mt-5 rounded-md border border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] px-4 py-3 text-sm text-[var(--v2-danger-text)]">{fileError}</div>)
          : isLoadingFile
          ? (<div className="mt-5 space-y-3">{[1, 2, 3, 4, 5].map((i) => (<div key={i} className="v2-skeleton h-4 rounded" />))}</div>)
          : selectedFile
            ? (<pre className="mt-5 max-h-[60vh] overflow-auto whitespace-pre-wrap rounded-[18px] border border-[var(--v2-panel-border)] bg-[var(--v2-code-bg)] p-4 font-mono text-xs leading-6 text-[var(--v2-text-strong)]">{selectedFile.content}</pre>)
            : (
                <EmptyPanel
                  title="No file selected"
                  description="Pick a concrete file from the workspace tree to render it here."
                />
              )}
      </Panel>
    </div>
  );
}
