import { createFileRoute } from "@tanstack/react-router";
import {
  CheckCircle,
  ExternalLink,
  Globe,
  Loader2,
  Package,
  Plug,
  Puzzle,
  RefreshCw,
  Search,
  Settings,
  Wrench,
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
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

export const Route = createFileRoute("/_layout/extensions")({
  component: ExtensionsPage,
});

type Extension = Awaited<
  ReturnType<ReturnType<typeof useApiClient>["ironclaw"]["extensions"]["list"]>
>["data"][number];

type RegistryEntry = Awaited<
  ReturnType<ReturnType<typeof useApiClient>["ironclaw"]["extensions"]["listRegistry"]>
>["data"][number];

type ConnectableChannel = Awaited<
  ReturnType<ReturnType<typeof useApiClient>["ironclaw"]["channels"]["listConnectable"]>
>["data"][number];

function ExtensionSkeleton() {
  return (
    <Card className="p-5 space-y-3">
      <div className="flex items-start justify-between">
        <div className="space-y-2 flex-1">
          <Skeleton className="h-5 w-40" />
          <Skeleton className="h-3 w-20" />
        </div>
        <Skeleton className="h-6 w-16 rounded-full" />
      </div>
      <Skeleton className="h-3 w-full" />
      <Skeleton className="h-3 w-3/4" />
      <div className="flex items-center gap-3 pt-2">
        <Skeleton className="h-4 w-24" />
        <Skeleton className="h-4 w-20" />
      </div>
      <div className="flex gap-2 pt-1">
        <Skeleton className="h-8 w-24 rounded-md" />
        <Skeleton className="h-8 w-24 rounded-md" />
      </div>
    </Card>
  );
}

function RegistryEntryCard({
  entry,
  onInstall,
  installing,
}: {
  entry: RegistryEntry;
  onInstall: (entry: RegistryEntry) => void;
  installing: boolean;
}) {
  return (
    <Card className="p-5 space-y-3">
      <div className="flex items-start justify-between gap-4">
        <div className="space-y-1.5 min-w-0">
          <div className="flex items-center gap-2">
            <h3 className="font-medium text-foreground truncate">{entry.displayName}</h3>
            <Badge variant="secondary" className="shrink-0 text-[10px] px-1.5 py-0">
              {entry.kind}
            </Badge>
          </div>
          {entry.version && (
            <p className="text-xs text-muted-foreground">v{entry.version}</p>
          )}
        </div>
        <Button
          size="sm"
          variant={entry.installed ? "outline" : "default"}
          disabled={entry.installed || installing}
          onClick={() => onInstall(entry)}
          className="shrink-0"
        >
          {installing ? (
            <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
          ) : (
            <Package className="mr-1.5 h-3.5 w-3.5" />
          )}
          {installing ? "Installing..." : entry.installed ? "Installed" : "Install"}
        </Button>
      </div>

      <p className="text-sm text-muted-foreground leading-relaxed">{entry.description}</p>

      {entry.keywords.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {entry.keywords.map((kw) => (
            <span
              key={kw}
              className="inline-flex items-center rounded-full bg-muted px-2 py-0.5 text-[10px] font-medium text-muted-foreground"
            >
              {kw}
            </span>
          ))}
        </div>
      )}
    </Card>
  );
}

function InstalledExtensionCard({
  ext,
  onToggleActive,
  onRemove,
  onConfigure,
  toggling,
  removing,
}: {
  ext: Extension;
  onToggleActive: (ext: Extension) => void;
  onRemove: (ext: Extension) => void;
  onConfigure: (ext: Extension) => void;
  toggling: boolean;
  removing: boolean;
}) {
  const kindIcon =
    ext.kind === "channel" ? (
      <Cable size={14} />
    ) : ext.kind === "provider" ? (
      <Globe size={14} />
    ) : (
      <Wrench size={14} />
    );

  return (
    <Card className="p-5 space-y-3">
      <div className="flex items-start justify-between gap-4">
        <div className="space-y-1.5 min-w-0">
          <div className="flex items-center gap-2">
            <h3 className="font-medium text-foreground truncate">{ext.displayName}</h3>
            <Badge variant="secondary" className="shrink-0 text-[10px] px-1.5 py-0">
              {kindIcon}
              <span className="ml-1">{ext.kind}</span>
            </Badge>
          </div>
          {ext.version && (
            <p className="text-xs text-muted-foreground">v{ext.version}</p>
          )}
        </div>
        {ext.active ? (
          <Badge variant="default" className="shrink-0 gap-1 text-[10px] px-2 py-0.5">
            <CheckCircle size={10} />
            Active
          </Badge>
        ) : (
          <Badge variant="outline" className="shrink-0 gap-1 text-[10px] px-2 py-0.5">
            <XCircle size={10} />
            Inactive
          </Badge>
        )}
      </div>

      <p className="text-sm text-muted-foreground leading-relaxed">{ext.description}</p>

      <div className="flex flex-wrap items-center gap-x-4 gap-y-1.5 text-xs text-muted-foreground">
        <span className="inline-flex items-center gap-1">
          <Wrench size={12} />
          {ext.tools.length} tool{ext.tools.length !== 1 ? "s" : ""}
        </span>
        {ext.authenticated ? (
          <span className="inline-flex items-center gap-1 text-amber-600 dark:text-amber-400">
            <Plug size={12} />
            Auth required
          </span>
        ) : (
          <span className="inline-flex items-center gap-1">
            <Plug size={12} />
            No auth
          </span>
        )}
        {ext.needsSetup && (
          <span className="inline-flex items-center gap-1 text-amber-600 dark:text-amber-400">
            <Settings size={12} />
            Needs setup
          </span>
        )}
        {ext.activationError && (
          <span className="inline-flex items-center gap-1 text-destructive" title={ext.activationError}>
            <XCircle size={12} />
            Error
          </span>
        )}
      </div>

      <div className="flex flex-wrap gap-2 pt-1">
        <Button
          size="sm"
          variant={ext.active ? "outline" : "default"}
          onClick={() => onToggleActive(ext)}
          disabled={toggling}
        >
          {toggling ? (
            <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
          ) : ext.active ? (
            <XCircle className="mr-1.5 h-3.5 w-3.5" />
          ) : (
            <CheckCircle className="mr-1.5 h-3.5 w-3.5" />
          )}
          {ext.active ? "Deactivate" : "Activate"}
        </Button>
        {ext.needsSetup && (
          <Button size="sm" variant="secondary" onClick={() => onConfigure(ext)}>
            <Settings className="mr-1.5 h-3.5 w-3.5" />
            Configure
          </Button>
        )}
        <Button
          size="sm"
          variant="destructive"
          onClick={() => onRemove(ext)}
          disabled={removing}
        >
          {removing ? (
            <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
          ) : (
            <XCircle className="mr-1.5 h-3.5 w-3.5" />
          )}
          Remove
        </Button>
      </div>
    </Card>
  );
}

function SetupDialog({
  ext,
  open,
  onOpenChange,
  onSave,
}: {
  ext: Extension | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSave?: () => void;
}) {
  const apiClient = useApiClient();
  const [setupData, setSetupData] = useState<Record<string, unknown> | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [formPayload, setFormPayload] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!ext || !open) return;
    setLoading(true);
    setError(null);
    apiClient.ironclaw.extensions
      .getSetup({ name: ext.packageRef.id })
      .then((res) => setSetupData(res))
      .catch((err: any) => setError(err.message ?? "Failed to load setup details"))
      .finally(() => setLoading(false));
  }, [ext, open, apiClient]);

  useEffect(() => {
    if (setupData) {
      const initial: Record<string, string> = {};
      for (const secret of setupData.secrets ?? []) {
        initial[secret.name ?? secret.key] = "";
      }
      for (const field of setupData.fields ?? []) {
        initial[field.name] = field.default ?? "";
      }
      setFormPayload(initial);
    }
  }, [setupData]);

  const displayName = ext?.displayName ?? ext?.packageRef.id ?? "Extension";

  const handleSubmit = async () => {
    if (!ext) return;
    setSaving(true);
    try {
      await apiClient.ironclaw.extensions.setup({
        name: ext.packageRef.id,
        action: "save",
        payload: formPayload,
      });
      toast.success(`${displayName} configured`);
      onOpenChange(false);
      onSave?.();
    } catch (err: any) {
      toast.error(err.message ?? "Failed to save configuration");
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Configure {displayName}</DialogTitle>
          <DialogDescription>
            Set up the required credentials and settings for this extension.
          </DialogDescription>
        </DialogHeader>

        {loading ? (
          <div className="flex items-center justify-center py-8">
            <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
          </div>
        ) : error ? (
          <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive">
            {error}
          </div>
        ) : setupData?.onboarding ? (
          <div className="space-y-4">
            {setupData.onboarding.credentialInstructions && (
              <div className="rounded-lg bg-muted p-4 space-y-2">
                <p className="text-xs font-medium text-foreground">Instructions</p>
                <p className="text-sm text-muted-foreground whitespace-pre-wrap">
                  {setupData.onboarding.credentialInstructions}
                </p>
              </div>
            )}

            {setupData.onboarding.setupUrl && (
              <a
                href={setupData.onboarding.setupUrl}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-1.5 text-sm text-primary hover:underline"
              >
                <ExternalLink size={14} />
                Open setup page
              </a>
            )}

            {setupData.onboarding.credentialNextStep && (
              <div className="rounded-lg border border-border bg-card p-4">
                <p className="text-xs font-medium text-foreground mb-1.5">
                  Credential Fields
                </p>
                <p className="text-sm text-muted-foreground">
                  {setupData.onboarding.credentialNextStep}
                </p>
              </div>
            )}

            {(setupData.secrets?.length > 0 || setupData.fields?.length > 0) && (
              <div className="space-y-3">
                {setupData.secrets?.length > 0 && (
                  <div className="space-y-3">
                    {setupData.secrets.map((secret: any) => {
                      const key = secret.name ?? secret.key;
                      return (
                        <div key={key} className="space-y-1.5">
                          <Label htmlFor={key}>
                            {secret.label ?? secret.prompt ?? key}
                          </Label>
                          <Input
                            id={key}
                            placeholder={secret.prompt ?? "Enter value"}
                            value={formPayload[key] ?? ""}
                            onChange={(e) =>
                              setFormPayload((prev) => ({ ...prev, [key]: e.target.value }))
                            }
                            disabled={secret.provided || saving}
                          />
                          {secret.provided && (
                            <p className="text-[11px] text-muted-foreground">Already provided</p>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
                {setupData.fields?.length > 0 && (
                  <div className="space-y-3">
                    {setupData.fields.map((field: any) => (
                      <div key={field.name} className="space-y-1.5">
                        <Label htmlFor={field.name}>
                          {field.prompt ?? field.name}
                          {field.optional ? " (optional)" : ""}
                        </Label>
                        <Input
                          id={field.name}
                          placeholder={field.placeholder ?? field.prompt ?? "Enter value"}
                          value={formPayload[field.name] ?? ""}
                          onChange={(e) =>
                            setFormPayload((prev) => ({ ...prev, [field.name]: e.target.value }))
                          }
                          disabled={saving}
                        />
                      </div>
                    ))}
                  </div>
                )}
                <Button onClick={handleSubmit} disabled={saving} className="w-full">
                  {saving ? (
                    <Loader2 className="mr-1.5 h-4 w-4 animate-spin" />
                  ) : null}
                  {saving ? "Saving..." : "Save Configuration"}
                </Button>
              </div>
            )}
          </div>
        ) : setupData ? (
          <div className="text-sm text-muted-foreground">
            <p>No additional setup required for this extension.</p>
          </div>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

function ExtensionsPage() {
  const apiClient = useApiClient();

  const [installed, setInstalled] = useState<Extension[]>([]);
  const [registry, setRegistry] = useState<RegistryEntry[]>([]);
  const [loadingInstalled, setLoadingInstalled] = useState(true);
  const [loadingRegistry, setLoadingRegistry] = useState(true);
  const [errorInstalled, setErrorInstalled] = useState<string | null>(null);
  const [errorRegistry, setErrorRegistry] = useState<string | null>(null);
  const [togglingExt, setTogglingExt] = useState<string | null>(null);
  const [removingExt, setRemovingExt] = useState<string | null>(null);
  const [installingRef, setInstallingRef] = useState<string | null>(null);
  const [setupExt, setSetupExt] = useState<Extension | null>(null);
  const [setupOpen, setSetupOpen] = useState(false);
  const [connectableChannels, setConnectableChannels] = useState<ConnectableChannel[]>([]);
  const [channelsLoading, setChannelsLoading] = useState(true);
  const [searchQuery, setSearchQuery] = useState("");

  const loadInstalled = useCallback(async () => {
    setLoadingInstalled(true);
    setErrorInstalled(null);
    try {
      const result = await apiClient.ironclaw.extensions.list();
      setInstalled(result.data);
    } catch (err: any) {
      setErrorInstalled(err.message ?? "Failed to load extensions");
    } finally {
      setLoadingInstalled(false);
    }
  }, [apiClient]);

  const loadRegistry = useCallback(async () => {
    setLoadingRegistry(true);
    setErrorRegistry(null);
    try {
      const result = await apiClient.ironclaw.extensions.listRegistry();
      setRegistry(result.data);
    } catch (err: any) {
      setErrorRegistry(err.message ?? "Failed to load registry");
    } finally {
      setLoadingRegistry(false);
    }
  }, [apiClient]);

  useEffect(() => {
    loadInstalled();
    loadRegistry();
  }, [loadInstalled, loadRegistry]);

  useEffect(() => {
    apiClient.ironclaw.channels
      .listConnectable()
      .then((res) => setConnectableChannels(res.data ?? []))
      .catch(() => {})
      .finally(() => setChannelsLoading(false));
  }, [apiClient]);

  const handleToggleActive = useCallback(
    async (ext: Extension) => {
      const name = ext.packageRef.id;
      setTogglingExt(name);
      try {
        if (ext.active) {
          await apiClient.ironclaw.extensions.remove({ name });
          toast.success(`${ext.displayName} deactivated`);
          loadInstalled();
          loadRegistry();
        } else {
          await apiClient.ironclaw.extensions.activate({ name });
          toast.success(`${ext.displayName} activated`);
          loadInstalled();
        }
      } catch (err: any) {
        toast.error(err.message ?? `Failed to ${ext.active ? "deactivate" : "activate"} extension`);
      } finally {
        setTogglingExt(null);
      }
    },
    [apiClient, loadInstalled, loadRegistry],
  );

  const handleRemove = useCallback(
    async (ext: Extension) => {
      const name = ext.packageRef.id;
      setRemovingExt(name);
      try {
        await apiClient.ironclaw.extensions.remove({ name });
        toast.success(`${ext.displayName} removed`);
        loadInstalled();
        loadRegistry();
      } catch (err: any) {
        toast.error(err.message ?? "Failed to remove extension");
      } finally {
        setRemovingExt(null);
      }
    },
    [apiClient, loadInstalled, loadRegistry],
  );

  const handleInstall = useCallback(
    async (entry: RegistryEntry) => {
      setInstallingRef(entry.packageRef.id);
      try {
        const result = await apiClient.ironclaw.extensions.install({
          packageRef: entry.packageRef,
        });
        toast.success(result.message ?? `${entry.displayName} installed`);
        loadInstalled();
        loadRegistry();
      } catch (err: any) {
        toast.error(err.message ?? "Failed to install extension");
      } finally {
        setInstallingRef(null);
      }
    },
    [apiClient, loadInstalled, loadRegistry],
  );

  const handleConfigure = useCallback((ext: Extension) => {
    setSetupExt(ext);
    setSetupOpen(true);
  }, []);

  const filteredRegistry = registry.filter(
    (entry) =>
      entry.displayName.toLowerCase().includes(searchQuery.toLowerCase()) ||
      entry.description.toLowerCase().includes(searchQuery.toLowerCase()) ||
      entry.keywords.some((kw) => kw.toLowerCase().includes(searchQuery.toLowerCase())),
  );

  return (
    <div className="space-y-6 p-6 max-w-5xl mx-auto">
      <div className="flex items-center gap-3">
        <div className="flex h-10 w-10 items-center justify-center rounded-full bg-primary/10">
          <Puzzle className="h-5 w-5 text-primary" />
        </div>
        <div className="space-y-0.5">
          <h1 className="text-lg font-semibold text-foreground">Extensions</h1>
          <p className="text-sm text-muted-foreground">
            Manage tools, channels, and providers that extend IronClaw's capabilities.
          </p>
        </div>
      </div>

      <Tabs defaultValue="installed" className="space-y-4">
        <TabsList>
          <TabsTrigger value="installed" className="gap-1.5">
            <Package size={14} />
            Installed
            {!loadingInstalled && !errorInstalled && (
              <span className="ml-1 rounded-full bg-muted-foreground/20 px-1.5 py-0 text-[10px] font-medium">
                {installed.length}
              </span>
            )}
          </TabsTrigger>
          <TabsTrigger value="registry" className="gap-1.5">
            <Globe size={14} />
            Registry
            {!loadingRegistry && !errorRegistry && (
              <span className="ml-1 rounded-full bg-muted-foreground/20 px-1.5 py-0 text-[10px] font-medium">
                {registry.length}
              </span>
            )}
          </TabsTrigger>
        </TabsList>

        <TabsContent value="installed" className="space-y-4 mt-0">
          {loadingInstalled ? (
            <div className="grid gap-4 sm:grid-cols-2">
              <ExtensionSkeleton />
              <ExtensionSkeleton />
              <ExtensionSkeleton />
            </div>
          ) : errorInstalled ? (
            <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-6 text-center space-y-3">
              <XCircle className="mx-auto h-6 w-6 text-destructive" />
              <p className="text-sm text-destructive">{errorInstalled}</p>
              <Button variant="outline" size="sm" onClick={loadInstalled}>
                <RefreshCw className="mr-1.5 h-3.5 w-3.5" />
                Retry
              </Button>
            </div>
          ) : installed.length === 0 ? (
            <div className="rounded-lg border border-border p-12 text-center space-y-3">
              <div className="flex h-12 w-12 items-center justify-center rounded-full bg-muted mx-auto">
                <Package className="h-6 w-6 text-muted-foreground" />
              </div>
              <div className="space-y-1">
                <p className="text-sm font-medium text-foreground">No extensions installed</p>
                <p className="text-xs text-muted-foreground">
                  Browse the Registry tab to find and install extensions.
                </p>
              </div>
            </div>
          ) : (
            <div className="grid gap-4 sm:grid-cols-2">
              {installed.map((ext) => (
                <InstalledExtensionCard
                  key={ext.packageRef.id}
                  ext={ext}
                  onToggleActive={handleToggleActive}
                  onRemove={handleRemove}
                  onConfigure={handleConfigure}
                  toggling={togglingExt === ext.packageRef.id}
                  removing={removingExt === ext.packageRef.id}
                />
              ))}
            </div>
          )}
        </TabsContent>

        <TabsContent value="registry" className="space-y-4 mt-0">
          <div className="relative">
            <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              placeholder="Search extensions..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="pl-9"
            />
          </div>

          {loadingRegistry ? (
            <div className="grid gap-4 sm:grid-cols-2">
              <ExtensionSkeleton />
              <ExtensionSkeleton />
              <ExtensionSkeleton />
            </div>
          ) : errorRegistry ? (
            <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-6 text-center space-y-3">
              <XCircle className="mx-auto h-6 w-6 text-destructive" />
              <p className="text-sm text-destructive">{errorRegistry}</p>
              <Button variant="outline" size="sm" onClick={loadRegistry}>
                <RefreshCw className="mr-1.5 h-3.5 w-3.5" />
                Retry
              </Button>
            </div>
          ) : registry.length === 0 ? (
            <div className="rounded-lg border border-border p-12 text-center space-y-3">
              <div className="flex h-12 w-12 items-center justify-center rounded-full bg-muted mx-auto">
                <Globe className="h-6 w-6 text-muted-foreground" />
              </div>
              <div className="space-y-1">
                <p className="text-sm font-medium text-foreground">No extensions in registry</p>
                <p className="text-xs text-muted-foreground">
                  The extension registry is empty. Check your configuration.
                </p>
              </div>
            </div>
          ) : filteredRegistry.length === 0 ? (
            <div className="rounded-lg border border-border p-8 text-center space-y-2">
              <p className="text-sm text-muted-foreground">
                No extensions match your search.
              </p>
            </div>
          ) : (
            <div className="grid gap-4 sm:grid-cols-2">
              {filteredRegistry.map((entry) => (
                <RegistryEntryCard
                  key={entry.packageRef.id}
                  entry={entry}
                  onInstall={handleInstall}
                  installing={installingRef === entry.packageRef.id}
                />
              ))}
            </div>
          )}
        </TabsContent>
      </Tabs>

      <section className="space-y-3">
        <div className="flex items-center gap-2">
          <Plug className="size-4 text-muted-foreground" />
          <h2 className="text-sm font-semibold text-foreground">Connectable Channels</h2>
        </div>
        {channelsLoading ? (
          <div className="grid gap-3 sm:grid-cols-2">
            <ExtensionSkeleton />
            <ExtensionSkeleton />
          </div>
        ) : connectableChannels.length === 0 ? (
          <Card className="p-4 text-center text-sm text-muted-foreground">
            No connectable channels available.
          </Card>
        ) : (
          <div className="grid gap-3 sm:grid-cols-2">
            {connectableChannels.map((ch) => (
              <Card key={ch.channel} className="p-4 space-y-2">
                <div className="flex items-center justify-between">
                  <div className="space-y-0.5">
                    <p className="text-sm font-medium text-foreground">{ch.displayName}</p>
                    <p className="text-xs text-muted-foreground">{ch.channel}</p>
                  </div>
                  <Badge variant="secondary">{ch.strategy}</Badge>
                </div>
                <p className="text-xs text-muted-foreground">{ch.action.title}</p>
                {ch.commandAliases.length > 0 && (
                  <div className="flex flex-wrap gap-1">
                    {ch.commandAliases.map((alias) => (
                      <span key={alias} className="rounded bg-muted px-1.5 py-0.5 text-[10px] font-mono text-muted-foreground">
                        /{alias}
                      </span>
                    ))}
                  </div>
                )}
              </Card>
            ))}
          </div>
        )}
      </section>

      <SetupDialog ext={setupExt} open={setupOpen} onOpenChange={setSetupOpen} onSave={loadInstalled} />
    </div>
  );
}
