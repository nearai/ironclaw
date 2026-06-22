import { createFileRoute, Link } from "@tanstack/react-router";
import {
  Check,
  ChevronDown,
  Cloud,
  Code2,
  Copy,
  ExternalLink,
  Key,
  MessageCircle,
  Package,
  RefreshCw,
  Terminal,
  Upload,
  UserPlus,
  Zap,
} from "lucide-react";
import { useRef, useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { CommandCopy } from "@/components/ui/command-copy";
import { getRepository } from "@/app";
import { useConnectionMode } from "@/hooks/use-connection-mode";
import { useIronclawStatus } from "@/hooks/use-ironclaw-status";

export const Route = createFileRoute("/_layout/setup")({
  head: () => ({
    meta: [
      { title: "IronClaw Setup | NEAR Builders" },
      {
        name: "description",
        content:
          "Connect to IronClaw: use the hosted agent or run your own binary. Get a NEAR AI API key, build skills, and submit to the hackathon.",
      },
    ],
  }),
  component: IronclawPage,
});

type StepId = "api-key" | "setup" | "skills" | "register" | "contribute" | "submit" | "support";

const steps: Array<{
  id: StepId;
  step: string;
  icon: React.ComponentType<{ size?: number; className?: string }>;
  title: string;
  subtitle: string;
  content: (props: { onNext: () => void }) => React.ReactNode;
  markdown: string;
}> = [
  {
    id: "api-key",
    step: "0",
    icon: Key,
    title: "Get Your NEAR AI API Key",
    subtitle: "Create an account, claim free credits, and generate credentials",
    content: ({ onNext }) => (
      <div className="space-y-4">
        <div className="rounded-lg border border-border bg-muted px-3.5 py-3 text-sm text-muted-foreground">
          You need a NEAR AI API key to run inference through your IronClaw agent. Recommended:
          DeepSeek V4 Flash via NEAR AI (fast, free).
        </div>

        <ol className="space-y-3 text-sm text-muted-foreground list-decimal pl-4">
          <li>
            <strong className="text-foreground">Create your account</strong> at{" "}
            <a
              href="https://cloud.near.ai"
              target="_blank"
              rel="noopener noreferrer"
              className="text-primary underline underline-offset-4"
            >
              cloud.near.ai
            </a>
          </li>
          <li>
            <strong className="text-foreground">Claim $5 of free credits</strong> —{" "}
            <a
              href="https://app.notion.com/p/near-foundation/Claiming-NEAR-AI-Cloud-Credits-2e6da22d7b6483deb74901226d383df2"
              target="_blank"
              rel="noopener noreferrer"
              className="text-primary underline underline-offset-4"
            >
              guide here
            </a>
          </li>
          <li>
            <strong className="text-foreground">Generate an API key</strong> in the &ldquo;API
            Keys&rdquo; section
          </li>
        </ol>

        <div className="rounded-md border border-border bg-muted/50 px-3.5 py-2.5 text-xs text-muted-foreground">
          Never share your API key publicly or commit it to version control.
        </div>

        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" size="sm" asChild>
              <a href="https://cloud.near.ai" target="_blank" rel="noopener noreferrer">
                <ExternalLink size={12} />
                cloud.near.ai
              </a>
            </Button>
            <Button variant="outline" size="sm" asChild>
              <a
                href="https://docs.near.ai/cloud/quickstart#setup"
                target="_blank"
                rel="noopener noreferrer"
              >
                <ExternalLink size={12} />
                Setup guide
              </a>
            </Button>
          </div>
          <Button size="sm" onClick={onNext}>
            Next: Set up binary
            <ChevronDown size={12} className="-rotate-90" />
          </Button>
        </div>
      </div>
    ),
    markdown: `## Step 0: Get Your NEAR AI API Key

1. Create your account at https://cloud.near.ai
2. Claim $5 of free credits
3. Generate an API key in the "API Keys" section

Setup guide: https://docs.near.ai/cloud/quickstart#setup`,
  },
  {
    id: "setup",
    step: "1",
    icon: Terminal,
    title: "Start Reborn with ngrok tunnel",
    subtitle: "Expose a local binary via ngrok for the production dashboard",
    content: ({ onNext }) => (
      <div className="space-y-4">
        <div className="rounded-lg border border-border bg-muted px-3.5 py-3 text-sm text-muted-foreground">
          <strong>scripts/bos-dev.sh --tunnel</strong> starts ironclaw-reborn locally and exposes it
          via ngrok. It prints the Tunnel URL and API Token to paste into Settings.
        </div>

        <div className="rounded-xl border-2 border-primary/30 bg-primary/5 px-5 py-4 space-y-3">
          <p className="text-sm font-bold text-foreground">Quick start (recommended)</p>
          <p className="text-sm text-muted-foreground">
            Clone the repo, export your API key, and run the tunnel script:
          </p>
          <CommandCopy command="git clone https://github.com/NEARBuilders/ironclaw.git && cd ironclaw" />
          <CommandCopy command='export NEARAI_API_KEY="your-key-here"' />
          <CommandCopy command="scripts/bos-dev.sh --tunnel" />
          <p className="text-sm text-muted-foreground">
            Copy the <strong>Tunnel URL</strong> and <strong>API Token</strong> printed by the
            script, then paste them into{" "}
            <Link
              to="/settings/ironclaw"
              className="text-primary underline underline-offset-4"
            >
              Settings → IronClaw
            </Link>
            .
          </p>
        </div>

        <details className="group">
          <summary className="cursor-pointer text-sm font-semibold text-muted-foreground hover:text-foreground transition-colors list-none flex items-center gap-2">
            <ChevronDown
              size={14}
              className="shrink-0 transition-transform duration-200 group-open:rotate-0 -rotate-90"
            />
            Manual setup (expand)
          </summary>
          <div className="mt-4 space-y-3">
            <p className="text-sm text-muted-foreground">
              Build from source or run without ngrok. The repo has full build instructions and CLI
              reference.
            </p>
            <Button variant="outline" size="sm" asChild>
              <a
                href={getRepository() ?? "https://github.com/NEARBuilders/ironclaw"}
                target="_blank"
                rel="noopener noreferrer"
              >
                <Code2 size={14} />
                View source on GitHub
              </a>
            </Button>
            <p className="text-xs text-muted-foreground pt-2">
              Key environment variables for manual runs:
            </p>
            <div className="space-y-1">
              <code className="block rounded bg-secondary px-2 py-1 font-mono text-xs">
                IRONCLAW_REBORN_WEBUI_TOKEN — bearer token for your binary
              </code>
              <code className="block rounded bg-secondary px-2 py-1 font-mono text-xs">
                NEARAI_API_KEY — provider API key (or OPENAI_API_KEY / ANTHROPIC_API_KEY)
              </code>
            </div>
          </div>
        </details>

        <div className="flex flex-wrap items-center justify-between gap-2">
          <Button variant="outline" size="sm" asChild>
            <a
              href={getRepository() ? `${getRepository()}/blob/main/docs/reborn-binary.md` : "https://github.com/nearai/ironclaw/blob/main/docs/reborn-binary.md"}
              target="_blank"
              rel="noopener noreferrer"
            >
              <ExternalLink size={12} />
              Reborn binary docs
            </a>
          </Button>
          <Button size="sm" onClick={onNext}>
            Next: NOVA account
            <ChevronDown size={12} className="-rotate-90" />
          </Button>
        </div>
      </div>
    ),
    markdown: `## Step 1: Start Reborn with ngrok tunnel

### Quick start (recommended)

\`\`\`bash
git clone https://github.com/NEARBuilders/ironclaw.git && cd ironclaw
export NEARAI_API_KEY="your-key-here"
scripts/bos-dev.sh --tunnel
\`\`\`

Copy the printed Tunnel URL and API Token into Settings → IronClaw on the dashboard.

Reborn binary docs: https://github.com/nearai/ironclaw/blob/main/docs/reborn-binary.md`,
  },
  {
    id: "skills",
    step: "2",
    icon: Package,
    title: "Get a NOVA Account",
    subtitle: "Create a NOVA account for encrypted submissions",
    content: ({ onNext }) => (
      <div className="space-y-4">
        <div className="rounded-lg border border-border bg-muted px-3.5 py-3 text-sm text-muted-foreground">
          The hackathon skill and nova-submit extension are built into the agent. You just need a
          NOVA account to submit your entry.
        </div>

        <p className="text-sm font-semibold text-foreground">Get a NOVA account</p>
        <p className="text-sm text-muted-foreground">
          Sign up at{" "}
          <a
            href="https://nova-sdk.com"
            target="_blank"
            rel="noopener noreferrer"
            className="text-primary underline underline-offset-4"
          >
            nova-sdk.com
          </a>{" "}
          for your NOVA account ID and API key. You will need these during registration and
          submission.
        </p>

        <div className="flex flex-wrap items-center justify-between gap-2">
          <Button variant="outline" size="sm" asChild>
            <a href="https://nova-sdk.com" target="_blank" rel="noopener noreferrer">
              <ExternalLink size={12} />
              nova-sdk.com
            </a>
          </Button>
          <Button size="sm" onClick={onNext}>
            Next: Register
            <ChevronDown size={12} className="-rotate-90" />
          </Button>
        </div>
      </div>
    ),
    markdown: `## Step 2: Get a NOVA Account

Sign up at https://nova-sdk.com for your NOVA account ID and API key.`,
  },
  {
    id: "register",
    step: "3",
    icon: UserPlus,
    title: "Register for the Hackathon",
    subtitle: "Record your intent to compete",
    content: ({ onNext }) => (
      <div className="space-y-4">
        <div className="rounded-lg border border-border bg-muted px-3.5 py-3 text-sm text-muted-foreground">
          Tell your agent in the chat: &ldquo;Register me for the hackathon.&rdquo;
          <br />
          Have your NOVA Account ID ready — the agent will ask for it.
        </div>

        <Link
          to="/chat"
          className="group flex items-center gap-4 rounded-xl border-2 border-primary/30 bg-primary/5 px-5 py-4 hover:border-primary hover:bg-primary/10 transition-all duration-200"
        >
          <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-lg bg-primary/10">
            <MessageCircle size={22} className="text-primary" />
          </div>
          <div className="flex-1 min-w-0">
            <p className="text-base font-semibold text-foreground group-hover:text-primary transition-colors">
              Open Chat
            </p>
            <p className="text-sm text-muted-foreground mt-0.5">
              Say: &ldquo;Register me for the hackathon&rdquo;
            </p>
          </div>
          <ExternalLink
            size={16}
            className="shrink-0 text-muted-foreground group-hover:text-primary transition-colors"
          />
        </Link>

        <div className="flex justify-end">
          <Button size="sm" onClick={onNext}>
            Next: Contribute
            <ChevronDown size={12} className="-rotate-90" />
          </Button>
        </div>
      </div>
    ),
    markdown: `## Step 3: Register for the Hackathon

Open the chat and tell your agent: "Register me for the hackathon."

Have your NOVA Account ID ready.`,
  },
  {
    id: "contribute",
    step: "4",
    icon: Upload,
    title: "Contribute Skills & Tools",
    subtitle: "Publish your extensions to IronHub",
    content: ({ onNext }) => (
      <div className="space-y-4">
        <div className="rounded-lg border border-border bg-muted px-3.5 py-3 text-sm text-muted-foreground">
          Before submitting, contribute your custom skills and tools to IronHub — this is part of
          the competition.
        </div>

        <div className="rounded-lg border border-border bg-muted px-3.5 py-3 space-y-4">
          <p className="text-sm font-semibold text-foreground">Submission paths</p>
          <div className="space-y-3">
            <div>
              <p className="text-sm font-medium text-foreground">Create a Skill</p>
              <p className="text-sm text-muted-foreground">
                Propose a{" "}
                <code className="rounded bg-secondary px-1 py-0.5 font-mono text-xs">SKILL.md</code>{" "}
                branch:
              </p>
              <a
                href="https://github.com/nearai/ironhub/issues/new?template=new-skill.yml"
                target="_blank"
                rel="noopener noreferrer"
                className="mt-1 inline-flex items-center gap-1 text-sm text-primary underline underline-offset-4"
              >
                New skill template <ExternalLink size={10} />
              </a>
            </div>
            <div>
              <p className="text-sm font-medium text-foreground">Create a Tool</p>
              <p className="text-sm text-muted-foreground">
                Propose a new WASM tool trunk with auth scopes and action surface:
              </p>
              <a
                href="https://github.com/nearai/ironhub/issues/new?template=new-tool.yml"
                target="_blank"
                rel="noopener noreferrer"
                className="mt-1 inline-flex items-center gap-1 text-sm text-primary underline underline-offset-4"
              >
                New tool template <ExternalLink size={10} />
              </a>
            </div>
            <div>
              <p className="text-sm font-medium text-foreground">No-code with Iliad</p>
              <p className="text-sm text-muted-foreground">
                Use the visual builder at{" "}
                <a
                  href="https://iliad.codes"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-primary underline underline-offset-4"
                >
                  iliad.codes
                </a>
                .
              </p>
            </div>
          </div>
        </div>

        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" size="sm" asChild>
              <a
                href="https://hub.ironclaw.com/developer"
                target="_blank"
                rel="noopener noreferrer"
              >
                <ExternalLink size={12} />
                IronHub developer hub
              </a>
            </Button>
            <Button variant="outline" size="sm" asChild>
              <a href="https://iliad.codes" target="_blank" rel="noopener noreferrer">
                <ExternalLink size={12} />
                Iliad
              </a>
            </Button>
          </div>
          <Button size="sm" onClick={onNext}>
            Next: Submit entry
            <ChevronDown size={12} className="-rotate-90" />
          </Button>
        </div>
      </div>
    ),
    markdown: `## Step 4: Contribute Skills & Tools`,
  },
  {
    id: "submit",
    step: "5",
    icon: Zap,
    title: "Submit Your Final Entry",
    subtitle: "Encrypt and upload via NOVA",
    content: ({ onNext }) => (
      <div className="space-y-4">
        <div className="rounded-lg border border-border bg-muted px-3.5 py-3 text-sm text-muted-foreground">
          Tell your agent: &ldquo;Submit my final entry.&rdquo; The{" "}
          <strong>submit_final_entry</strong> method validates inputs, builds a submission file, and
          uploads it via the nova-submit tool.
        </div>

        <div className="rounded-lg border border-border bg-muted px-3.5 py-3">
          <p className="text-sm font-semibold text-foreground mb-2">What you need to provide</p>
          <div className="overflow-x-auto">
            <table className="w-full text-sm border-collapse">
              <thead>
                <tr className="bg-secondary">
                  <th className="px-3 py-2 text-left text-xs font-semibold text-muted-foreground border-b border-border">
                    Field
                  </th>
                  <th className="px-3 py-2 text-left text-xs font-semibold text-muted-foreground border-b border-border">
                    Required
                  </th>
                  <th className="px-3 py-2 text-left text-xs font-semibold text-muted-foreground border-b border-border">
                    Notes
                  </th>
                </tr>
              </thead>
              <tbody>
                {[
                  ["NOVA Account ID", "Yes", "Must match registration"],
                  ["NOVA API Key", "Yes", "From nova-sdk.com. Never stored or echoed"],
                  ["Project Title", "Yes", "Short name for your project"],
                  ["Workflow Description", "Yes", "One sentence, \u2264280 chars"],
                  ["Demo URL", "Yes", "~5 min video, publicly viewable"],
                  ["GitHub Repo", "No", "Public repo URL"],
                  ["Skills List", "No", "Comma-separated custom skills/tools"],
                  ["Demo Notes", "No", "Anything for the judges"],
                ].map(([field, req, notes]) => (
                  <tr key={field} className="border-b border-border last:border-0">
                    <td className="px-3 py-2 font-mono text-xs text-foreground">{field}</td>
                    <td className="px-3 py-2 text-xs text-foreground">{req}</td>
                    <td className="px-3 py-2 text-xs text-muted-foreground">{notes}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>

        <div className="rounded-md border border-border bg-muted/50 px-3.5 py-2.5 text-xs text-muted-foreground">
          On success you get a CID as proof. <strong>Rotate your NOVA API key</strong> at
          nova-sdk.com afterward.
        </div>

        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" size="sm" asChild>
              <a
                href="https://github.com/jcarbonnell/ironclaw-hackathon"
                target="_blank"
                rel="noopener noreferrer"
              >
                <ExternalLink size={12} />
                ironclaw-hackathon repo
              </a>
            </Button>
            <Button variant="outline" size="sm" asChild>
              <a href="https://nova-sdk.com" target="_blank" rel="noopener noreferrer">
                <ExternalLink size={12} />
                nova-sdk.com
              </a>
            </Button>
          </div>
          <Button size="sm" onClick={onNext}>
            Next: Get support
            <ChevronDown size={12} className="-rotate-90" />
          </Button>
        </div>
      </div>
    ),
    markdown: `## Step 5: Submit Your Final Entry`,
  },
  {
    id: "support",
    step: "6",
    icon: MessageCircle,
    title: "Get Support",
    subtitle: "Join the IronClaw community on Telegram",
    content: () => (
      <div className="space-y-4">
        <div className="rounded-lg border border-border bg-muted px-3.5 py-3 text-sm text-muted-foreground">
          Stuck? Need help with your agent, the hackathon skill, or NOVA setup? The IronClaw
          community is active on Telegram.
        </div>

        <a
          href="https://t.me/ironclawAI"
          target="_blank"
          rel="noopener noreferrer"
          className="group flex items-center gap-4 rounded-xl border-2 border-primary/30 bg-primary/5 px-5 py-4 hover:border-primary hover:bg-primary/10 transition-all duration-200"
        >
          <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-lg bg-primary/10">
            <MessageCircle size={22} className="text-primary" />
          </div>
          <div className="flex-1 min-w-0">
            <p className="text-base font-semibold text-foreground group-hover:text-primary transition-colors">
              t.me/ironclawAI
            </p>
            <p className="text-sm text-muted-foreground mt-0.5">
              Ask questions, share progress, connect with other participants
            </p>
          </div>
          <ExternalLink
            size={16}
            className="shrink-0 text-muted-foreground group-hover:text-primary transition-colors"
          />
        </a>

        <div className="flex flex-wrap gap-2">
          <Button variant="outline" size="sm" asChild>
            <a href="https://t.me/ironclawAI" target="_blank" rel="noopener noreferrer">
              <ExternalLink size={12} />
              Join Telegram
            </a>
          </Button>
          <Button variant="outline" size="sm" asChild>
            <a href="https://docs.ironclaw.com" target="_blank" rel="noopener noreferrer">
              <ExternalLink size={12} />
              IronClaw Docs
            </a>
          </Button>
        </div>
      </div>
    ),
    markdown: `## Step 6: Get Support\n\nhttps://t.me/ironclawAI`,
  },
];

function buildRebornMarkdown(): string {
  return [
    "---",
    "name: ironclaw-hackathon-guide",
    "version: 1.0.0",
    "description: Full walkthrough for the NEAR Legion IronClaw Hackathon.",
    "activation:",
    "  keywords: [hackathon, ironclaw, nova, near, reborn]",
    "  tags: [hackathon, ironclaw]",
    "  max_context_tokens: 4000",
    "---",
    "",
    "# IronClaw Hackathon Guide (Reborn Binary)",
    "",
    ...steps.flatMap((s) => [s.markdown, ""]),
  ].join("\n");
}

function StepProgressBar({
  steps: stepList,
  completedSteps,
  activeStep,
  onStepClick,
}: {
  steps: typeof steps;
  completedSteps: Set<StepId>;
  activeStep: StepId | null;
  onStepClick: (id: StepId) => void;
}) {
  return (
    <div className="flex items-center gap-0 overflow-x-auto pb-1 scrollbar-none">
      {stepList.map((step, index) => {
        const isCompleted = completedSteps.has(step.id);
        const isActive = activeStep === step.id;
        const Icon = step.icon;

        return (
          <div key={step.id} className="flex items-center shrink-0">
            <button
              type="button"
              onClick={() => onStepClick(step.id)}
              title={step.title}
              className={`flex flex-col items-center gap-1 px-2 py-1.5 rounded-lg transition-colors cursor-pointer group ${
                isActive
                  ? "text-primary"
                  : isCompleted
                    ? "text-[color:var(--near-green)]"
                    : "text-muted-foreground hover:text-foreground"
              }`}
            >
              <div
                className={`flex h-7 w-7 items-center justify-center rounded-full border-2 transition-colors ${
                  isCompleted
                    ? "border-[color:var(--near-green)] bg-[color:var(--near-green)]/10"
                    : isActive
                      ? "border-primary bg-primary/10"
                      : "border-border bg-card group-hover:border-border-strong"
                }`}
              >
                {isCompleted ? (
                  <Check size={12} className="text-[color:var(--near-green)]" />
                ) : (
                  <Icon size={11} />
                )}
              </div>
              <span className="text-[10px] font-medium hidden sm:block max-w-[56px] text-center leading-tight truncate">
                {step.title.split(" ").slice(0, 2).join(" ")}
              </span>
              <span className="text-[10px] font-mono sm:hidden">{step.step}</span>
            </button>
            {index < stepList.length - 1 && (
              <div
                className={`h-px w-4 shrink-0 mx-0.5 transition-colors ${
                  completedSteps.has(stepList[index + 1].id) || isCompleted
                    ? "bg-[color:var(--near-green)]/40"
                    : "bg-border"
                }`}
              />
            )}
          </div>
        );
      })}
    </div>
  );
}

function IronclawPage() {
  const { connectionMode } = useConnectionMode();
  const { status: connectionStatus, refetch: refetchStatus } = useIronclawStatus();
  const [completedSteps, setCompletedSteps] = useState<Set<StepId>>(new Set());
  const [activeStep, setActiveStep] = useState<StepId | null>("api-key");
  const [copied, setCopied] = useState(false);
  const stepsRef = useRef<HTMLDivElement>(null);

  const isConnected = connectionStatus === "connected";

  const binarySteps = steps.filter((s) => ["api-key", "setup"].includes(s.id));
  const hackathonSteps = steps.filter((s) => !["api-key", "setup"].includes(s.id));
  const allBinaryComplete = binarySteps.every((s) => completedSteps.has(s.id));

  const toggleStep = (id: StepId) => {
    setActiveStep((prev) => (prev === id ? null : id));
  };

  const markCompleteAndAdvance = (id: StepId) => {
    setCompletedSteps((prev) => new Set([...prev, id]));
    const currentIndex = steps.findIndex((s) => s.id === id);
    if (currentIndex < steps.length - 1) {
      setActiveStep(steps[currentIndex + 1].id);
      setTimeout(() => {
        document
          .getElementById(`step-${steps[currentIndex + 1].id}`)
          ?.scrollIntoView({ behavior: "smooth", block: "nearest" });
      }, 50);
    }
  };

  const scrollToLocalSetup = () => {
    toggleStep("setup");
    stepsRef.current?.scrollIntoView({ behavior: "smooth", block: "start" });
  };

  const handleCopy = async () => {
    await navigator.clipboard.writeText(buildRebornMarkdown());
    setCopied(true);
    toast.success("Guide copied — load as IronClaw skill");
    setTimeout(() => setCopied(false), 2000);
  };

  const renderStep = (step: (typeof steps)[0]) => {
    const isOpen = activeStep === step.id;
    const isCompleted = completedSteps.has(step.id);
    const Icon = step.icon;

    return (
      <div
        key={step.id}
        id={`step-${step.id}`}
        className={`rounded-xl border bg-card overflow-hidden transition-colors ${
          isCompleted ? "border-[color:var(--near-green)]/30" : "border-border"
        }`}
      >
        <button
          type="button"
          onClick={() => toggleStep(step.id)}
          className="flex w-full items-center gap-3 px-5 py-4 text-left cursor-pointer hover:bg-muted/50 transition-colors duration-150"
        >
          <div
            className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-md transition-colors ${
              isCompleted
                ? "bg-[color:var(--near-green)]/10 text-[color:var(--near-green)]"
                : isOpen
                  ? "bg-primary/10 text-primary"
                  : "bg-secondary text-muted-foreground"
            }`}
          >
            {isCompleted ? <Check size={14} /> : <Icon size={14} />}
          </div>
          <div className="flex-1 min-w-0">
            <span
              className={`text-sm font-semibold ${isCompleted ? "text-muted-foreground line-through decoration-[color:var(--near-green)]/50" : "text-foreground"}`}
            >
              {step.title}
            </span>
            <p className="text-xs text-muted-foreground mt-0.5">{step.subtitle}</p>
          </div>
          <div className="flex items-center gap-2 shrink-0">
            <ChevronDown
              size={16}
              className={`text-muted-foreground transition-transform duration-200 ${
                isOpen ? "rotate-0" : "-rotate-90"
              }`}
            />
          </div>
        </button>
        {isOpen && (
          <div className="border-t border-border px-5 py-4">
            {step.content({ onNext: () => markCompleteAndAdvance(step.id) })}
            <div className="mt-4 pt-3 border-t border-border flex items-center justify-between">
              <button
                type="button"
                onClick={() => setActiveStep(null)}
                className="text-xs text-muted-foreground hover:text-foreground transition-colors"
              >
                Collapse
              </button>
              {!isCompleted && (
                <button
                  type="button"
                  onClick={() => markCompleteAndAdvance(step.id)}
                  className="flex items-center gap-1.5 rounded-md border border-[color:var(--near-green)]/40 bg-[color:var(--near-green)]/5 px-3 py-1 text-xs font-medium text-[color:var(--near-green)] hover:bg-[color:var(--near-green)]/10 transition-colors"
                >
                  <Check size={10} />
                  Mark complete
                </button>
              )}
            </div>
          </div>
        )}
      </div>
    );
  };

  return (
    <div className="flex min-h-[calc(100dvh-4rem)] flex-col overflow-auto">
      <div className="flex-1">
        <div className="mx-auto w-full max-w-4xl space-y-4 px-4 py-6 sm:px-6 sm:py-10 pb-20 sm:pb-12">
          <div className="rounded-xl border border-border bg-card p-6">
            <div className="flex items-center gap-3">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-primary text-primary-foreground">
                <Zap size={18} />
              </div>
              <div>
                <span className="text-base font-semibold text-foreground">IronClaw Setup</span>
                <p className="mt-0.5 text-sm text-muted-foreground">
                  Connect your agent to this dashboard
                </p>
              </div>
            </div>

            {connectionStatus !== "checking" && (
              <div
                className={`mt-4 flex items-center gap-3 rounded-lg border px-3.5 py-2.5 ${
                  isConnected
                    ? "border-[color:var(--near-green)]/30 bg-[color:var(--near-green)]/5"
                    : "border-destructive/30 bg-destructive/5"
                }`}
              >
                <div
                  className={`h-2 w-2 rounded-full shrink-0 ${
                    isConnected ? "bg-[color:var(--near-green)]" : "bg-destructive"
                  }`}
                />
                <span className="text-xs text-muted-foreground flex-1">
                  {isConnected
                    ? `Connected via ${connectionMode === "hosted" ? "deployed" : "tunnel"}`
                    : "Agent not connected"}
                </span>
                {isConnected ? (
                  <button
                    type="button"
                    onClick={() => refetchStatus()}
                    className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors"
                  >
                    <RefreshCw size={10} />
                    Refresh
                  </button>
                ) : (
                  <div className="flex items-center gap-2">
                    <Link
                      to="/settings/ironclaw"
                      className="flex items-center gap-1 text-xs font-medium text-primary hover:text-primary/80 transition-colors"
                    >
                      Settings
                    </Link>
                    <button
                      type="button"
                      onClick={() => refetchStatus()}
                      className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground transition-colors"
                    >
                      <RefreshCw size={10} />
                      Retry
                    </button>
                  </div>
                )}
              </div>
            )}
          </div>

          <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
            <div
              onClick={scrollToLocalSetup}
              className={`rounded-xl border-2 bg-card p-6 hover:border-primary/40 transition-colors text-left cursor-pointer ${
                isConnected ? "border-[color:var(--near-green)]/30" : "border-primary/20"
              }`}
            >
              <div className="flex items-center gap-3 mb-4">
                <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-primary/10">
                  <Terminal size={18} className="text-primary" />
                </div>
                <div>
                  <h3 className="text-base font-semibold text-foreground">Tunnel</h3>
                  <p className="text-xs text-muted-foreground">run locally, expose with ngrok</p>
                </div>
              </div>
              <p className="text-sm text-muted-foreground mb-4">
                Run Reborn locally and expose it via ngrok. The script prints a Tunnel URL and
                API Token to paste into Settings.
              </p>
              <Button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  scrollToLocalSetup();
                }}
              >
                {isConnected ? "Reconfigure" : "Set up tunnel"}
              </Button>
            </div>

            <div
              className={`rounded-xl border-2 bg-card p-6 text-left opacity-60 blur-[1px] pointer-events-none select-none ${
                isConnected ? "border-[color:var(--near-green)]/30" : "border-primary/20"
              }`}
            >
              <div className="flex items-center gap-3 mb-4">
                <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-primary/10">
                  <Cloud size={18} className="text-primary" />
                </div>
                <div>
                  <h3 className="text-base font-semibold text-foreground">Deploy Your Own</h3>
                  <p className="text-xs text-muted-foreground">agent.near.ai · TEE deployment</p>
                </div>
              </div>
              <p className="text-sm text-muted-foreground mb-4">
                Configure a hosted agent at agent.near.ai, deploy to a Trusted Execution
                Environment, generate an API token, and connect via Settings. Share API keys with
                your team — each user gets their own dashboard.
              </p>
              <span className="inline-flex h-9 items-center justify-center rounded-lg bg-primary px-4 text-sm font-bold text-primary-foreground">
                Deploy Agent
              </span>
            </div>
          </div>

          <div ref={stepsRef}>
            <div className="rounded-xl border border-border bg-card p-4 sm:p-6 space-y-4">
              <div className="flex items-center justify-between gap-4 flex-wrap">
                <div>
                  <p className="text-sm font-semibold text-foreground">Tunnel Setup</p>
                  <p className="text-xs text-muted-foreground mt-0.5">
                    {allBinaryComplete
                      ? "All steps complete!"
                      : `Step ${binarySteps.filter((s) => completedSteps.has(s.id)).length + 1} of ${binarySteps.length}`}
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  {completedSteps.size > 0 && (
                    <button
                      type="button"
                      onClick={() => setCompletedSteps(new Set())}
                      className="text-xs text-muted-foreground hover:text-foreground transition-colors"
                    >
                      Reset progress
                    </button>
                  )}
                  <Button variant="outline" size="sm" onClick={handleCopy}>
                    {copied ? <Check size={14} /> : <Copy size={14} />}
                    {copied ? "Copied" : "Copy as skill"}
                  </Button>
                </div>
              </div>

              <StepProgressBar
                steps={binarySteps}
                completedSteps={completedSteps}
                activeStep={activeStep}
                onStepClick={toggleStep}
              />

              {allBinaryComplete && (
                <div className="rounded-lg border border-[color:var(--near-green)]/30 bg-[color:var(--near-green)]/5 px-4 py-3 text-sm text-[color:var(--near-green)] font-medium">
                  You&apos;re all set! Head back to the chat to start using your agent.
                </div>
              )}
            </div>

            <div className="space-y-3 mt-3">{binarySteps.map(renderStep)}</div>
          </div>

          <div>
            <div className="rounded-xl border border-border bg-card p-4 sm:p-6">
              <h2 className="text-base font-semibold text-foreground">Hackathon</h2>
              <p className="text-sm text-muted-foreground mt-1">
                Steps to participate in the NEAR Legion IronClaw Hackathon
              </p>
            </div>

            <div className="space-y-3 mt-3">{hackathonSteps.map(renderStep)}</div>
          </div>

          <div className="rounded-xl border border-border bg-card p-6">
            <p className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-3">
              Quick links
            </p>
            <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
              <UrlCard href="https://ironclaw.com" label="IronClaw" />
              <UrlCard
                href="https://github.com/nearai/ironclaw/blob/main/docs/reborn-binary.md"
                label="Reborn binary docs"
              />
              <UrlCard
                href="https://github.com/nearai/ironclaw/blob/main/docs/reborn-binary.md#extension"
                label="Nova-Submit extension"
              />
              <UrlCard
                href="https://github.com/jcarbonnell/ironclaw-hackathon"
                label="ironclaw-hackathon"
              />
              <UrlCard href="https://nova-sdk.com" label="NOVA SDK" />
              <UrlCard href="https://docs.ironclaw.com" label="IronClaw Docs" />
              <UrlCard href="https://cloud.near.ai" label="NEAR AI Cloud" />
              <UrlCard
                href="https://docs.near.ai/cloud/quickstart#setup"
                label="API key setup guide"
              />
              <UrlCard href="https://t.me/ironclawAI" label="Telegram: @ironclawAI" />
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function UrlCard({ href, label }: { href: string; label: string }) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      className="group flex items-center gap-2 rounded-lg border border-border bg-muted px-3.5 py-2.5 text-sm text-muted-foreground hover:text-foreground hover:border-border-strong transition-colors duration-150"
    >
      <span className="flex-1 truncate font-medium">{label}</span>
      <ExternalLink
        size={12}
        className="shrink-0 opacity-0 group-hover:opacity-100 transition-opacity"
      />
    </a>
  );
}
