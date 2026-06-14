import { createFileRoute } from "@tanstack/react-router";
import {
  BookOpen,
  Code,
  Download,
  Edit3,
  FileText,
  Loader2,
  RefreshCw,
  Save,
  Search,
  Tag,
  Trash2,
  X,
  XCircle,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { useApiClient } from "@/app";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";

export const Route = createFileRoute("/_layout/skills")({
  component: SkillsPage,
});

type Skill = Awaited<ReturnType<ReturnType<typeof useApiClient>["ironclaw"]["skills"]["list"]>>["data"][number];

type CatalogItem = {
  name: string;
  description: string;
};

function trustVariant(trust: string): "default" | "secondary" | "destructive" | "outline" {
  if (trust === "high") return "default";
  if (trust === "medium") return "secondary";
  return "outline";
}

function SkillSkeleton() {
  return (
    <Card className="p-5 space-y-3">
      <Skeleton className="h-5 w-3/4" />
      <Skeleton className="h-4 w-full" />
      <Skeleton className="h-4 w-1/2" />
      <div className="flex gap-2">
        <Skeleton className="h-5 w-16" />
        <Skeleton className="h-5 w-16" />
      </div>
    </Card>
  );
}

function SkillsPage() {
  const apiClient = useApiClient();

  const [installed, setInstalled] = useState<Skill[]>([]);
  const [installedLoading, setInstalledLoading] = useState(true);
  const [installedError, setInstalledError] = useState<string | null>(null);

  const [searchQuery, setSearchQuery] = useState("");
  const [catalogResults, setCatalogResults] = useState<CatalogItem[]>([]);
  const [catalogInstalledNames, setCatalogInstalledNames] = useState<string[]>([]);
  const [catalogLoading, setCatalogLoading] = useState(false);
  const [catalogError, setCatalogError] = useState<string | undefined>();

  const [editingSkill, setEditingSkill] = useState<Skill | null>(null);
  const [editContent, setEditContent] = useState("");
  const [editLoading, setEditLoading] = useState(false);
  const [savingEdit, setSavingEdit] = useState(false);
  const [dialogOpen, setDialogOpen] = useState(false);

  const fetchInstalled = useCallback(async () => {
    setInstalledLoading(true);
    setInstalledError(null);
    try {
      const res = await apiClient.ironclaw.skills.list();
      setInstalled(res.data);
    } catch {
      setInstalledError("Failed to load skills");
      setInstalled([]);
    } finally {
      setInstalledLoading(false);
    }
  }, [apiClient]);

  useEffect(() => {
    fetchInstalled();
  }, [fetchInstalled]);

  const handleSearch = useCallback(async () => {
    if (!searchQuery.trim()) return;
    setCatalogLoading(true);
    setCatalogError(undefined);
    try {
      const res = await apiClient.ironclaw.skills.search({ query: searchQuery });
      const installedNames = res.installed.map((s) => s.name);
      setCatalogResults(res.catalog as CatalogItem[]);
      setCatalogInstalledNames(installedNames);
      if (res.catalogError) {
        setCatalogError(res.catalogError);
      }
    } catch {
      setCatalogError("Search failed. Please try again.");
      setCatalogResults([]);
    } finally {
      setCatalogLoading(false);
    }
  }, [apiClient, searchQuery]);

  const handleInstall = useCallback(
    async (name: string) => {
      try {
        const res = await apiClient.ironclaw.skills.install({ name });
        if (res.success) {
          toast.success(res.message);
          setCatalogInstalledNames((prev) => [...prev, name]);
          fetchInstalled();
        } else {
          toast.error(res.message);
        }
      } catch (err: any) {
        toast.error(err.message ?? "Failed to install skill");
      }
    },
    [apiClient, fetchInstalled],
  );

  const handleRemove = useCallback(
    async (name: string) => {
      try {
        const res = await apiClient.ironclaw.skills.remove({ name });
        if (res.success) {
          toast.success(res.message);
          setInstalled((prev) => prev.filter((s) => s.name !== name));
        } else {
          toast.error(res.message);
        }
      } catch (err: any) {
        toast.error(err.message ?? "Failed to remove skill");
      }
    },
    [apiClient],
  );

  const handleViewEdit = useCallback(
    async (skill: Skill) => {
      setEditingSkill(skill);
      setEditLoading(true);
      setDialogOpen(true);
      try {
        const res = await apiClient.ironclaw.skills.get({ name: skill.name });
        setEditContent(res.content);
      } catch {
        toast.error("Failed to load skill content");
        setEditContent("");
      } finally {
        setEditLoading(false);
      }
    },
    [apiClient],
  );

  const handleSaveEdit = useCallback(async () => {
    if (!editingSkill) return;
    setSavingEdit(true);
    try {
      const res = await apiClient.ironclaw.skills.update({
        name: editingSkill.name,
        content: editContent,
      });
      if (res.success) {
        toast.success(res.message);
        setDialogOpen(false);
        setEditingSkill(null);
        setEditContent("");
      } else {
        toast.error(res.message);
      }
    } catch (err: any) {
      toast.error(err.message ?? "Failed to update skill");
    } finally {
      setSavingEdit(false);
    }
  }, [apiClient, editingSkill, editContent]);

  const handleCloseDialog = useCallback(() => {
    setDialogOpen(false);
    setEditingSkill(null);
    setEditContent("");
  }, []);

  return (
    <div className="space-y-6">
      <div className="space-y-1">
        <h2 className="text-lg font-semibold text-foreground">Skills</h2>
        <p className="text-sm text-muted-foreground">
          Manage installed skills and discover new ones from the catalog.
        </p>
      </div>

      <Tabs defaultValue="installed">
        <TabsList>
          <TabsTrigger value="installed" className="flex items-center gap-1.5">
            <BookOpen size={14} />
            Installed
            {!installedLoading && !installedError && (
              <span className="ml-1 rounded-full bg-muted-foreground/20 px-1.5 py-0 text-[10px] font-medium">
                {installed.length}
              </span>
            )}
          </TabsTrigger>
          <TabsTrigger value="catalog" className="flex items-center gap-1.5">
            <Search size={14} />
            Catalog
          </TabsTrigger>
        </TabsList>

        <TabsContent value="installed" className="space-y-4 mt-4">
          {installedLoading ? (
            <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
              {Array.from({ length: 6 }).map((_, i) => (
                <SkillSkeleton key={i} />
              ))}
            </div>
          ) : installedError ? (
            <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-6 text-center space-y-3">
              <XCircle className="mx-auto h-6 w-6 text-destructive" />
              <p className="text-sm text-destructive">Failed to load installed skills.</p>
              <Button variant="outline" size="sm" onClick={fetchInstalled}>
                <RefreshCw size={14} className="mr-1.5" />
                Retry
              </Button>
            </div>
          ) : installed.length === 0 ? (
            <div className="flex flex-col items-center gap-3 rounded-lg border border-border bg-muted/50 px-4 py-8 text-center">
              <BookOpen size={24} className="text-muted-foreground" />
              <p className="text-sm text-muted-foreground">No skills installed.</p>
              <p className="text-xs text-muted-foreground">
                Discover and install skills from the Catalog tab.
              </p>
            </div>
          ) : (
            <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
              {installed.map((skill) => (
                <Card key={skill.name} className="flex flex-col p-5">
                  <div className="flex items-start justify-between gap-2">
                    <div className="min-w-0 flex-1 space-y-1">
                      <h3 className="truncate text-sm font-semibold text-foreground">
                        {skill.name}
                      </h3>
                      <p className="line-clamp-2 text-xs text-muted-foreground">
                        {skill.description}
                      </p>
                    </div>
                    <Badge variant={trustVariant(skill.trust)} className="shrink-0 text-[10px]">
                      {skill.trust}
                    </Badge>
                  </div>

                  <div className="mt-2 flex items-center gap-2 text-[10px] text-muted-foreground">
                    <FileText size={10} />
                    {skill.version}
                    <Code size={10} className="ml-1" />
                    {skill.source}
                  </div>

                  {skill.keywords.length > 0 && (
                    <div className="mt-2 flex flex-wrap gap-1">
                      {skill.keywords.map((kw) => (
                        <span
                          key={kw}
                          className="inline-flex items-center gap-0.5 rounded-full bg-secondary px-2 py-0.5 text-[10px] text-secondary-foreground"
                        >
                          <Tag size={8} />
                          {kw}
                        </span>
                      ))}
                    </div>
                  )}

                  {skill.usageHint && (
                    <p className="mt-2 text-[10px] italic text-muted-foreground">
                      {skill.usageHint}
                    </p>
                  )}
                  {skill.setupHint && (
                    <p className="mt-1 text-[10px] text-muted-foreground">
                      Setup: {skill.setupHint}
                    </p>
                  )}

                  <div className="mt-auto flex items-center gap-1.5 pt-3">
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-7 text-xs"
                      onClick={() => handleViewEdit(skill)}
                    >
                      {skill.canEdit ? <Edit3 size={11} className="mr-1" /> : <Code size={11} className="mr-1" />}
                      {skill.canEdit ? "View / Edit" : "View"}
                    </Button>
                    {skill.canDelete && (
                      <Button
                        variant="outline"
                        size="sm"
                        className="h-7 text-xs text-muted-foreground hover:text-destructive"
                        onClick={() => handleRemove(skill.name)}
                      >
                        <Trash2 size={11} className="mr-1" />
                        Remove
                      </Button>
                    )}
                  </div>
                </Card>
              ))}
            </div>
          )}
        </TabsContent>

        <TabsContent value="catalog" className="space-y-4 mt-4">
          <div className="flex gap-2">
            <div className="relative flex-1">
              <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground" />
              <Input
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleSearch();
                }}
                placeholder="Search skill catalog..."
                className="pl-8"
              />
            </div>
            <Button variant="default" size="sm" onClick={handleSearch} disabled={catalogLoading}>
              {catalogLoading ? (
                <Loader2 size={14} className="animate-spin" />
              ) : (
                <Search size={14} />
              )}
            </Button>
          </div>

          {!searchQuery.trim() && !catalogLoading && !catalogResults.length && (
            <div className="rounded-lg border border-border bg-muted/50 px-4 py-6 text-center">
              <p className="text-sm text-muted-foreground">
                Type a query and press Enter or click Search to find skills.
              </p>
            </div>
          )}

          {catalogError && (
            <div className="rounded-lg border border-border bg-muted/50 px-4 py-3 text-xs text-muted-foreground">
              {catalogError}
            </div>
          )}

          {catalogLoading && (
            <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
              {Array.from({ length: 3 }).map((_, i) => (
                <SkillSkeleton key={i} />
              ))}
            </div>
          )}

          {!catalogLoading && catalogResults.length > 0 && (
            <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
              {catalogResults.map((item) => {
                const isInstalled = catalogInstalledNames.includes(item.name);
                return (
                  <Card key={item.name} className="flex flex-col p-5">
                    <div className="min-w-0 space-y-1">
                      <h3 className="truncate text-sm font-semibold text-foreground">
                        {item.name}
                      </h3>
                      <p className="line-clamp-2 text-xs text-muted-foreground">
                        {item.description}
                      </p>
                    </div>
                    <div className="mt-auto pt-3">
                      <Button
                        variant={isInstalled ? "outline" : "default"}
                        size="sm"
                        className="h-7 text-xs"
                        disabled={isInstalled}
                        onClick={() => handleInstall(item.name)}
                      >
                        <Download size={11} className="mr-1" />
                        {isInstalled ? "Installed" : "Install"}
                      </Button>
                    </div>
                  </Card>
                );
              })}
            </div>
          )}

          {!catalogLoading && !catalogError && searchQuery && catalogResults.length === 0 && (
            <div className="flex flex-col items-center gap-3 rounded-lg border border-border bg-muted/50 px-4 py-8 text-center">
              <BookOpen size={24} className="text-muted-foreground" />
              <p className="text-sm text-muted-foreground">No results found.</p>
              <p className="text-xs text-muted-foreground">
                Try a different search term.
              </p>
            </div>
          )}
        </TabsContent>
      </Tabs>

      <Dialog open={dialogOpen} onOpenChange={handleCloseDialog}>
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Code size={16} />
              {editingSkill?.name}
            </DialogTitle>
          </DialogHeader>
          {editLoading ? (
            <div className="flex items-center justify-center py-8">
              <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
            </div>
          ) : (
            <div className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="skill-content">Content</Label>
                <Textarea
                  id="skill-content"
                  value={editContent}
                  onChange={(e) => setEditContent(e.target.value)}
                  className="min-h-[300px] font-mono text-xs"
                  readOnly={!editingSkill?.canEdit}
                />
              </div>
              {editingSkill?.canEdit && (
                <div className="flex justify-end gap-2">
                  <Button variant="outline" size="sm" onClick={handleCloseDialog}>
                    <X size={14} className="mr-1" />
                    Cancel
                  </Button>
                  <Button variant="default" size="sm" onClick={handleSaveEdit} disabled={savingEdit}>
                    {savingEdit ? (
                      <Loader2 size={14} className="mr-1 animate-spin" />
                    ) : (
                      <Save size={14} className="mr-1" />
                    )}
                    {savingEdit ? "Saving..." : "Save"}
                  </Button>
                </div>
              )}
            </div>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}
