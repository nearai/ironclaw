import { Callout, CodePanel, EmptyPanel, Card, Skeleton } from "@ironclaw/ui";

function TreeNodes({ nodes, depth = 0, selectedPath, expandingPath, onToggleDirectory, onSelectPath }) {
  return (
    <>
    {nodes.map((node) => (
      <div key={node.path}>
        <button
          onClick={() => (node.isDir ? onToggleDirectory(node.path) : onSelectPath(node.path))}
          className={[
            "flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-sm transition-colors",
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)]",
            selectedPath === node.path ? "bg-signal/10 text-white" : "text-iron-200 hover:bg-white/[0.05]",
          ].join(" ")}
          style={{ paddingLeft: `${depth * 18 + 12}px` }}
        >
          <span className="w-4 text-center text-iron-300">
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
      <Card className="min-h-[440px] p-4">
        <div className="border-b border-white/10 px-2 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">Workspace tree</div>
          <p className="mt-2 text-sm leading-6 text-iron-300">Browse the sandbox output and inspect generated files inline.</p>
        </div>

        <div className="mt-3 max-h-[60vh] overflow-y-auto">
          {treeError && (<Callout tone="danger" className="mx-2 mb-3">{treeError}</Callout>)}
          {isLoadingTree
            ? (<div className="space-y-2 px-2">{[1, 2, 3, 4].map((i) => (<Skeleton key={i} className="h-8 rounded-md" />))}</div>)
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
              : (<div className="px-2 py-6 text-sm text-iron-300">No files were recorded for this workspace.</div>)}
        </div>
      </Card>

      <Card className="min-h-[440px] p-5 sm:p-6">
        <div className="border-b border-white/10 pb-3">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-iron-300">File preview</div>
          <p className="mt-2 break-all text-sm leading-6 text-iron-300">{selectedFile?.path || selectedPath || "Select a file from the tree to inspect its contents."}</p>
        </div>

        {fileError && !isLoadingFile
          ? (<Callout tone="danger" className="mt-5">{fileError}</Callout>)
          : isLoadingFile
          ? (<div className="mt-5 space-y-3">{[1, 2, 3, 4, 5].map((i) => (<Skeleton key={i} className="h-4 rounded" />))}</div>)
          : selectedFile
            ? (<CodePanel wrap className="mt-5 max-h-[60vh]">{selectedFile.content}</CodePanel>)
            : (
                <EmptyPanel
                  title="No file selected"
                  description="Pick a concrete file from the workspace tree to render it here."
                />
              )}
      </Card>
    </div>
  );
}
