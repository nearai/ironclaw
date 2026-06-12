import { createFileRoute } from "@tanstack/react-router";
import { Check, Copy, ExternalLink, FileText } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { getAccount, getActiveRuntime, getAppName } from "@/app";
import { Button } from "@/components/ui/button";
import { Markdown } from "@/components/ui/markdown";

const INTENT_REGISTRY_URL = "https://tanstack.com/intent/registry/everything-dev";

export const Route = createFileRoute("/_layout/skill")({
  loader: async ({ context }) => {
    const runtimeConfig = context.runtimeConfig;

    const skill = await fetch("/skill.md")
      .then(async (response) => {
        if (!response.ok) {
          throw new Error(`Failed to load skill: ${response.status}`);
        }

        return response.text();
      })
      .catch(() => null);

    return {
      runtimeConfig,
      skill,
      intentRegistryUrl: INTENT_REGISTRY_URL,
    };
  },
  head: () => ({
    meta: [
      { title: "Skill | app" },
      {
        name: "description",
        content: "Agent-oriented instructions for running, editing, and publishing this runtime.",
      },
    ],
  }),
  component: SkillPage,
});

function SkillPage() {
  const { skill, runtimeConfig, intentRegistryUrl } = Route.useLoaderData();
  const runtime = getActiveRuntime(runtimeConfig);
  const account = getAccount(runtimeConfig);
  const appName = getAppName(runtimeConfig);
  const [copied, setCopied] = useState(false);

  const accountId = runtime?.accountId ?? account;

  const handleCopy = async () => {
    if (!skill) {
      toast.error("Skill prompt unavailable");
      return;
    }

    await navigator.clipboard.writeText(skill);
    setCopied(true);
    toast.success("Skill prompt copied");
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <div className="flex-1 overflow-y-auto">
        <div className="mx-auto w-full max-w-5xl space-y-4 px-4 py-6 sm:px-6 sm:py-10 animate-fade-in">
          <div className="rounded-[12px] border border-border bg-card p-6 space-y-4">
            <div className="flex items-start justify-between gap-4 flex-wrap">
              <div className="flex items-center gap-3 min-w-0">
                <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-[10px] bg-foreground text-background">
                  <FileText size={18} />
                </div>
                <div className="min-w-0">
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="text-xs font-mono text-muted-foreground">{accountId}</span>
                    <span className="text-muted-foreground">/</span>
                    <span className="text-base font-semibold text-foreground">{appName}</span>
                  </div>
                  <p className="mt-1 text-sm text-muted-foreground">
                    Agent-ready prompt for TanStack Intent, local development, UI changes, and
                    publish flow.
                  </p>
                </div>
              </div>

              <div className="flex flex-wrap items-center gap-2">
                <Button variant="outline" onClick={handleCopy} disabled={!skill}>
                  {copied ? <Check size={14} /> : <Copy size={14} />}
                  {copied ? "Copied" : "Copy prompt"}
                </Button>
                <Button variant="outline" asChild>
                  <a href="/skill.md" target="_blank" rel="noopener noreferrer">
                    <ExternalLink size={14} />
                    raw skill.md
                  </a>
                </Button>
                <Button asChild>
                  <a href={intentRegistryUrl} target="_blank" rel="noopener noreferrer">
                    <ExternalLink size={14} />
                    TanStack Intent
                  </a>
                </Button>
              </div>
            </div>

            <div className="rounded-[8px] border border-border bg-muted px-3.5 py-3 text-sm text-muted-foreground">
              Best entry points: `npx @tanstack/intent@latest load everything-dev`, `/skill.md`, and
              the registry page above.
            </div>
          </div>

          {skill ? (
            <div className="rounded-[12px] border border-border bg-card p-8">
              <Markdown content={skill} />
            </div>
          ) : (
            <div className="flex flex-col items-center justify-center gap-3 rounded-[12px] border border-border bg-card px-8 py-16 text-muted-foreground">
              <FileText size={32} className="text-border" />
              <p className="text-sm text-muted-foreground">Skill prompt unavailable.</p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
