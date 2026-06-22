import { Copy } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "./ui/button";
import { Card, CardContent } from "./ui/card";
import { Input } from "./ui/input";
import { Label } from "./ui/label";

export interface ApiKeyFormValues {
  name: string;
  expiresIn?: number;
}

interface ApiKeyFormProps {
  onCreate: (values: ApiKeyFormValues) => void;
  isPending: boolean;
}

const EXPIRATION_PRESETS = [
  { label: "no expiry", value: 0 },
  { label: "7 days", value: 7 * 24 * 60 * 60 },
  { label: "30 days", value: 30 * 24 * 60 * 60 },
  { label: "90 days", value: 90 * 24 * 60 * 60 },
  { label: "1 year", value: 365 * 24 * 60 * 60 },
] as const;

export function ApiKeyForm({ onCreate, isPending }: ApiKeyFormProps) {
  const [name, setName] = useState("");
  const [expiresInSeconds, setExpiresInSeconds] = useState<number>(0);

  const handleSubmit = () => {
    if (!name.trim()) return;
    onCreate({
      name: name.trim(),
      expiresIn: expiresInSeconds > 0 ? expiresInSeconds : undefined,
    });
    setName("");
    setExpiresInSeconds(0);
  };

  return (
    <div className="space-y-4">
      <div className="space-y-2">
        <Label className="text-xs text-muted-foreground uppercase tracking-wide">name</Label>
        <Input
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="API key name"
        />
      </div>

      <div className="space-y-2">
        <Label className="text-xs text-muted-foreground uppercase tracking-wide">expiration</Label>
        <div className="flex flex-wrap gap-2">
          {EXPIRATION_PRESETS.map((preset) => (
            <Button
              key={preset.value}
              type="button"
              variant={expiresInSeconds === preset.value ? "default" : "outline"}
              size="sm"
              onClick={() => setExpiresInSeconds(preset.value)}
            >
              {preset.label}
            </Button>
          ))}
        </div>
      </div>

      <p className="text-xs text-muted-foreground leading-relaxed">
        Permissions, rate limits, and refill configuration are server-only and cannot be set from
        the browser. Provision them through a server-side endpoint or admin tooling.
      </p>

      <div className="flex gap-2">
        <Button
          onClick={handleSubmit}
          disabled={isPending || !name.trim()}
          variant="outline"
          size="sm"
        >
          {isPending ? "creating..." : "create key"}
        </Button>
      </div>
    </div>
  );
}

export interface ApiKeyRevealProps {
  apiKey: {
    id: string;
    name: string | null;
    prefix: string | null;
    start: string | null;
    key: string;
    createdAt: string | Date;
  };
  onDismiss: () => void;
}

export function ApiKeyReveal({ apiKey, onDismiss }: ApiKeyRevealProps) {
  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(apiKey.key);
      toast.success("API key copied");
    } catch {
      toast.error("Failed to copy API key");
    }
  };

  return (
    <Card>
      <CardContent className="p-6 space-y-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="space-y-1">
            <div className="font-medium">New API key ready</div>
            <p className="text-sm text-muted-foreground leading-relaxed">
              Copy and store this key now. You will only be able to see the full secret once.
            </p>
          </div>
          <Button onClick={onDismiss} variant="outline" size="sm">
            dismiss
          </Button>
        </div>
        <div className="grid gap-3 md:grid-cols-[1fr_auto]">
          <Input
            readOnly
            value={apiKey.key}
            className="font-mono text-xs"
            onFocus={(e) => e.target.select()}
            onClick={(e) => e.currentTarget.select()}
          />
          <Button onClick={handleCopy} variant="outline" size="sm">
            <Copy className="h-3.5 w-3.5 mr-1" />
            copy
          </Button>
        </div>
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3 text-sm">
          <InfoRow label="name" value={apiKey.name ?? "unnamed"} />
          <InfoRow label="prefix" value={`${apiKey.prefix ?? "api_"}...`} mono />
          <InfoRow label="created" value={new Date(apiKey.createdAt).toLocaleString()} />
        </div>
      </CardContent>
    </Card>
  );
}

function InfoRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="border-2 border-outset border-[rgb(51,51,51)] dark:border-[rgb(100,100,100)] bg-muted/30 p-3 grid gap-1 sm:grid-cols-[100px_1fr] sm:gap-4">
      <div className="text-xs uppercase tracking-wide text-muted-foreground">{label}</div>
      <div className={mono ? "text-xs font-mono break-all" : "text-sm break-all"}>{value}</div>
    </div>
  );
}
