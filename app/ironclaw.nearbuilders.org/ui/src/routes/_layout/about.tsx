import { createFileRoute, Link } from "@tanstack/react-router";
import { BookOpen, ExternalLink, FileText, GitFork, Sparkles } from "lucide-react";
import { getAccount, getActiveRuntime, getAppName, getRepository } from "@/app";
import { Markdown } from "@/components/ui/markdown";
import { fetchRepositoryReadme } from "@/lib/repository-content";

export const Route = createFileRoute("/_layout/about")({
  loader: async ({ context }) => {
    const repository = getRepository(context.runtimeConfig);
    const description =
      ((context.runtimeConfig as Record<string, unknown>)?.description as string | null) ?? null;
    let readme: string | null = null;
    if (repository) {
      readme = await fetchRepositoryReadme(repository).catch(() => null);
    }
    return { repository, readme, description, runtimeConfig: context.runtimeConfig };
  },
  head: () => ({
    meta: [
      { title: "About | app" },
      { name: "description", content: "About this runtime-composed app on NEAR." },
    ],
  }),
  component: About,
});

function isGithubUrl(url: string) {
  return /github\.com/i.test(url);
}

function GithubIcon({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d="M12 0C5.37 0 0 5.373 0 12c0 5.303 3.438 9.8 8.205 11.387.6.113.82-.258.82-.577v-2.165c-3.338.726-4.042-1.61-4.042-1.61-.546-1.387-1.333-1.756-1.333-1.756-1.09-.745.083-.729.083-.729 1.205.085 1.84 1.237 1.84 1.237 1.07 1.834 2.807 1.304 3.492.997.108-.775.418-1.305.76-1.605-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23A11.52 11.52 0 0 1 12 6.803c1.02.005 2.047.138 3.006.404 2.29-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.91 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.218.694.825.576C20.565 21.796 24 17.3 24 12c0-6.627-5.373-12-12-12z" />
    </svg>
  );
}

function parseGithubRepo(url: string): { owner: string; repo: string } | null {
  const match = url.match(/github\.com\/([^/]+)\/([^/]+?)(?:\.git)?(?:\/.*)?$/i);
  if (!match) return null;
  return { owner: match[1], repo: match[2] };
}

function About() {
  const { repository, readme, description, runtimeConfig } = Route.useLoaderData();
  const runtime = getActiveRuntime(runtimeConfig);
  const account = getAccount(runtimeConfig);
  const appName = getAppName(runtimeConfig);

  const accountId = runtime?.accountId ?? account;
  const githubRepo = repository && isGithubUrl(repository) ? parseGithubRepo(repository) : null;

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <div className="flex-1 overflow-y-auto">
        <div className="w-full max-w-4xl mx-auto px-4 sm:px-6 py-6 sm:py-10 space-y-4 animate-fade-in">
          <div className="rounded-[12px] border border-border bg-card p-6 space-y-4">
            <div className="flex items-start justify-between gap-4">
              <div className="flex items-center gap-3 min-w-0">
                <div className="w-10 h-10 rounded-[10px] bg-foreground flex items-center justify-center shrink-0">
                  <BookOpen size={18} className="text-background" />
                </div>
                <div className="min-w-0">
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="text-xs text-muted-foreground font-mono">{accountId}</span>
                    <span className="text-muted-foreground">/</span>
                    <span className="text-base font-semibold text-foreground">{appName}</span>
                  </div>
                  {githubRepo && (
                    <div className="flex items-center gap-1.5 mt-0.5 text-xs text-muted-foreground font-mono">
                      <GitFork size={11} />
                      <span>
                        {githubRepo.owner}/{githubRepo.repo}
                      </span>
                    </div>
                  )}
                </div>
              </div>

              <div className="flex shrink-0 flex-wrap items-center gap-2">
                <Link
                  to="/skill"
                  preload="intent"
                  className="h-9 rounded-[12px] px-4 text-sm font-bold inline-flex items-center gap-2 no-underline transition-colors duration-150 bg-foreground text-background hover:opacity-90"
                >
                  <Sparkles size={14} />
                  Skill
                </Link>
                <a
                  href="/skill.md"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="h-9 rounded-[12px] px-4 text-sm font-bold inline-flex items-center gap-2 no-underline transition-colors duration-150 bg-secondary text-foreground hover:bg-border"
                >
                  <FileText size={14} />
                  skill.md
                </a>
                {repository && (
                  <a
                    href={repository}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="h-9 rounded-[12px] px-4 text-sm font-bold inline-flex items-center gap-2 no-underline transition-colors duration-150 bg-secondary text-foreground hover:bg-border"
                  >
                    {isGithubUrl(repository) ? (
                      <GithubIcon size={14} />
                    ) : (
                      <ExternalLink size={14} />
                    )}
                    {isGithubUrl(repository) ? "GitHub" : "Repository"}
                  </a>
                )}
              </div>
            </div>

            {description && (
              <p className="text-sm text-muted-foreground leading-relaxed">{description}</p>
            )}

            {repository && (
              <div className="rounded-[8px] border border-border bg-muted px-3.5 py-2.5 flex items-center gap-2">
                <span className="text-[11px] font-bold uppercase tracking-wider text-muted-foreground shrink-0 min-w-[64px]">
                  repo
                </span>
                <a
                  href={repository}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-sm font-mono text-foreground hover:underline truncate"
                >
                  {repository}
                </a>
              </div>
            )}

            <div className="rounded-[8px] border border-border bg-muted px-3.5 py-3 flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
              <div>
                <div className="text-[11px] font-bold uppercase tracking-wider text-muted-foreground">
                  for agents and builders
                </div>
                <p className="mt-1 text-sm text-muted-foreground">
                  Open the skill page for the essential TanStack Intent, local dev, and publish
                  instructions.
                </p>
              </div>
              <div className="flex flex-wrap gap-2">
                <Link
                  to="/skill"
                  preload="intent"
                  className="h-9 rounded-[12px] px-4 text-sm font-bold inline-flex items-center gap-2 no-underline transition-colors duration-150 bg-card text-foreground border border-border hover:bg-background"
                >
                  <Sparkles size={14} />
                  Open skill
                </Link>
                <a
                  href="/skill.md"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="h-9 rounded-[12px] px-4 text-sm font-bold inline-flex items-center gap-2 no-underline transition-colors duration-150 bg-card text-foreground border border-border hover:bg-background"
                >
                  <FileText size={14} />
                  Raw markdown
                </a>
              </div>
            </div>
          </div>

          {readme ? (
            <div className="rounded-[12px] border border-border bg-card p-8">
              <Markdown content={readme} />
            </div>
          ) : (
            <div className="flex flex-col items-center justify-center gap-3 px-8 py-16 rounded-[12px] border border-border bg-card text-muted-foreground">
              <FileText size={32} className="text-border" />
              <p className="text-sm text-muted-foreground">No README available.</p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
