import { createFileRoute } from "@tanstack/react-router";
import {
  Check,
  ChevronDown,
  Cloud,
  Copy,
  ExternalLink,
  Key,
  MessageCircle,
  Package,
  Terminal,
  Upload,
  UserPlus,
  Zap,
} from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { CommandCopy } from "@/components/ui/command-copy";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

export const Route = createFileRoute("/_layout/ironclaw")({
  head: () => ({
    meta: [
      { title: "IronClaw Setup | NEAR Builders" },
      {
        name: "description",
        content:
          "Set up IronClaw: get a NEAR AI API key, run the reborn binary with WebUI, install the hackathon skill, register, contribute, and submit.",
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
          <li>
            <strong className="text-foreground">Export your API key</strong>
          </li>
        </ol>

        <CommandCopy command='export NEARAI_API_KEY="your-key-here"' />

        <p className="text-xs text-muted-foreground">
          The{" "}
          <code className="rounded bg-secondary px-1 py-0.5 font-mono text-xs">
            scripts/run-reborn-webui.sh
          </code>{" "}
          script in the next step will configure the model provider automatically (defaults to
          DeepSeek V4 Flash via NEAR AI).
        </p>

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
4. Export your API key:

\`\`\`bash
export NEARAI_API_KEY="your-key-here"
\`\`\`

Setup guide: https://docs.near.ai/cloud/quickstart#setup`,
  },
  {
    id: "setup",
    step: "1",
    icon: Terminal,
    title: "Set Up IronClaw (Reborn)",
    subtitle: "Build and run the reborn binary with the WebUI",
    content: ({ onNext }) => (
      <div className="space-y-4">
        <div className="rounded-lg border border-border bg-muted px-3.5 py-3 text-sm text-muted-foreground">
          <strong>ironclaw-reborn</strong> is the standalone binary with the WebChat v2 UI. Build
          from source — pre-built releases are not yet available.
        </div>

        <div className="rounded-xl border-2 border-primary/30 bg-primary/5 px-5 py-4 space-y-3">
          <p className="text-sm font-bold text-foreground">Quick start (recommended)</p>
          <p className="text-sm text-muted-foreground">
            The repo includes{" "}
            <code className="rounded bg-secondary px-1 py-0.5 font-mono text-xs">
              scripts/run-reborn-webui.sh
            </code>{" "}
            which handles the entire setup:
          </p>
          <CommandCopy command="git clone https://github.com/NEARBuilders/ironclaw.git && cd ironclaw" />
          <CommandCopy command='export NEARAI_API_KEY="your-key-here"' />
          <CommandCopy command="scripts/run-reborn-webui.sh" />
          <p className="text-sm text-muted-foreground">
            Open{" "}
            <a
              href="http://127.0.0.1:3000/v2"
              target="_blank"
              rel="noopener noreferrer"
              className="text-primary underline underline-offset-4"
            >
              http://127.0.0.1:3000/v2
            </a>{" "}
            and log in with the printed token.
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
          <div className="mt-4 space-y-4">
            <CommandCopy command="git clone https://github.com/NEARBuilders/ironclaw.git" />
            <CommandCopy command="cd ironclaw" />
            <CommandCopy command="cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- --help" />
            <p className="text-xs text-muted-foreground">
              Or build separately:{" "}
              <code className="rounded bg-secondary px-1 py-0.5 font-mono text-xs">
                cargo build -p ironclaw_reborn_cli --bin ironclaw-reborn
              </code>
            </p>
            <CommandCopy command='export IRONCLAW_REBORN_HOME="$HOME/.ironclaw-reborn-demo"' />
            <p className="text-xs text-muted-foreground">
              The home directory <strong>must not</strong> be inside the repo.
            </p>
            <CommandCopy command='export IRONCLAW_REBORN_WEBUI_TOKEN="$(openssl rand -hex 32)"' />
            <CommandCopy command='export IRONCLAW_REBORN_WEBUI_USER_ID="reborn-cli"' />
            <CommandCopy command="cargo run -q -p ironclaw_reborn_cli --features webui-v2-beta --bin ironclaw-reborn -- serve" />
          </div>
        </details>

        <div className="flex flex-wrap items-center justify-between gap-2">
          <Button variant="outline" size="sm" asChild>
            <a
              href="https://github.com/nearai/ironclaw/blob/main/docs/reborn-binary.md"
              target="_blank"
              rel="noopener noreferrer"
            >
              <ExternalLink size={12} />
              Reborn binary docs
            </a>
          </Button>
          <Button size="sm" onClick={onNext}>
            Next: Install skill
            <ChevronDown size={12} className="-rotate-90" />
          </Button>
        </div>
      </div>
    ),
    markdown: `## Step 1: Set Up IronClaw (Reborn)

### Quick start (recommended)

\`\`\`bash
git clone https://github.com/NEARBuilders/ironclaw.git && cd ironclaw
export NEARAI_API_KEY="your-key-here"
scripts/run-reborn-webui.sh
\`\`\`

Reborn binary docs: https://github.com/nearai/ironclaw/blob/main/docs/reborn-binary.md`,
  },
  {
    id: "skills",
    step: "2",
    icon: Package,
    title: "Install the Hackathon Skill",
    subtitle: "Equip your agent with nova-submit and the hackathon skill",
    content: ({ onNext }) => (
      <div className="space-y-4">
        <div className="rounded-lg border border-border bg-muted px-3.5 py-3 text-sm text-muted-foreground">
          Your agent needs two things: the <strong>nova-submit</strong> Reborn extension (encrypts
          and uploads files to a NOVA group) and the <strong>ironclaw-hackathon</strong> skill
          (guides your agent through register and submit flows).
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
          for your NOVA account ID and API key.
        </p>

        <CommandCopy command='export IRONCLAW_REBORN_HOME="$HOME/.ironclaw-reborn-demo"' />
        <p className="text-xs text-muted-foreground">
          Make sure{" "}
          <code className="rounded bg-secondary px-1 py-0.5 font-mono text-xs">
            IRONCLAW_REBORN_HOME
          </code>{" "}
          matches what{" "}
          <code className="rounded bg-secondary px-1 py-0.5 font-mono text-xs">
            scripts/run-reborn-webui.sh
          </code>{" "}
          sets.
        </p>
        <CommandCopy command="cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- extension install nova-submit" />
        <CommandCopy command="cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- extension activate nova-submit" />
        <CommandCopy command="git clone https://github.com/jcarbonnell/ironclaw-hackathon.git" />
        <CommandCopy command='mkdir -p "$IRONCLAW_REBORN_HOME/local-dev/tenants/default/users/reborn-cli/skills/ironclaw-hackathon"' />
        <CommandCopy command='cp ironclaw-hackathon/skill/SKILL.md "$IRONCLAW_REBORN_HOME/local-dev/tenants/default/users/reborn-cli/skills/ironclaw-hackathon/"' />
        <p className="text-sm text-muted-foreground">Verify everything is set up:</p>
        <CommandCopy command='cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- extension search nova && cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- skills list | grep hackathon' />

        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" size="sm" asChild>
              <a
                href="https://github.com/nearai/ironclaw/blob/main/docs/reborn-binary.md#extension"
                target="_blank"
                rel="noopener noreferrer"
              >
                <ExternalLink size={12} />
                Nova-Submit docs
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
            Next: Register
            <ChevronDown size={12} className="-rotate-90" />
          </Button>
        </div>
      </div>
    ),
    markdown: `## Step 2: Install the Hackathon Skill

\`\`\`bash
export IRONCLAW_REBORN_HOME="$HOME/.ironclaw-reborn-demo"
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- extension install nova-submit
cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- extension activate nova-submit
git clone https://github.com/jcarbonnell/ironclaw-hackathon.git
mkdir -p "$IRONCLAW_REBORN_HOME/local-dev/tenants/default/users/reborn-cli/skills/ironclaw-hackathon"
cp ironclaw-hackathon/skill/SKILL.md "$IRONCLAW_REBORN_HOME/local-dev/tenants/default/users/reborn-cli/skills/ironclaw-hackathon/"
\`\`\``,
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
          Tell your agent: &ldquo;Register me for the hackathon.&rdquo; The{" "}
          <strong>register_competing_agent</strong> method records your intent to compete.
        </div>

        <div className="rounded-lg border border-border bg-muted px-3.5 py-3">
          <p className="text-sm font-semibold text-foreground mb-2">What you need to provide</p>
          <div className="overflow-x-auto">
            <table className="w-full text-sm border-collapse">
              <thead>
                <tr className="bg-secondary">
                  <th className="px-3 py-2 text-left text-xs font-semibold text-muted-foreground border-b border-border">Field</th>
                  <th className="px-3 py-2 text-left text-xs font-semibold text-muted-foreground border-b border-border">Required</th>
                  <th className="px-3 py-2 text-left text-xs font-semibold text-muted-foreground border-b border-border">Purpose</th>
                </tr>
              </thead>
              <tbody>
                <tr className="border-b border-border">
                  <td className="px-3 py-2 font-mono text-xs text-foreground">Agent ID</td>
                  <td className="px-3 py-2 text-xs text-foreground">Yes</td>
                  <td className="px-3 py-2 text-xs text-muted-foreground">Short handle, no spaces/slashes/quotes</td>
                </tr>
                <tr className="border-b border-border">
                  <td className="px-3 py-2 font-mono text-xs text-foreground">Participant Name</td>
                  <td className="px-3 py-2 text-xs text-foreground">Yes</td>
                  <td className="px-3 py-2 text-xs text-muted-foreground">Name or @handle for the leaderboard</td>
                </tr>
                <tr>
                  <td className="px-3 py-2 font-mono text-xs text-foreground">NOVA Account ID</td>
                  <td className="px-3 py-2 text-xs text-foreground">Yes</td>
                  <td className="px-3 py-2 text-xs text-muted-foreground">
                    Must match at submission time, e.g.{" "}
                    <code className="rounded bg-secondary px-1 py-0.5 font-mono text-xs">alice.nova-sdk.near</code>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>

        <p className="text-sm text-muted-foreground">
          The skill prints a <strong>registration block</strong>. Copy it and send to hackathon
          staff — they need your NOVA account ID to add you to the submission group.
        </p>

        <div className="rounded-md border border-border bg-muted/50 px-3.5 py-2.5 text-xs text-muted-foreground">
          Until staff adds you, you <strong>cannot submit</strong> — the upload will fail.
        </div>

        <div className="flex justify-end">
          <Button size="sm" onClick={onNext}>
            Next: Contribute
            <ChevronDown size={12} className="-rotate-90" />
          </Button>
        </div>
      </div>
    ),
    markdown: `## Step 3: Register for the Hackathon

Tell your agent: "Register me for the hackathon."`,
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
              <a href="https://hub.ironclaw.com/developer" target="_blank" rel="noopener noreferrer">
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
                  <th className="px-3 py-2 text-left text-xs font-semibold text-muted-foreground border-b border-border">Field</th>
                  <th className="px-3 py-2 text-left text-xs font-semibold text-muted-foreground border-b border-border">Required</th>
                  <th className="px-3 py-2 text-left text-xs font-semibold text-muted-foreground border-b border-border">Notes</th>
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
              <a href="https://github.com/jcarbonnell/ironclaw-hackathon" target="_blank" rel="noopener noreferrer">
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
  const [completedSteps, setCompletedSteps] = useState<Set<StepId>>(new Set());
  const [activeStep, setActiveStep] = useState<StepId | null>("api-key");
  const [copied, setCopied] = useState(false);
  const [activeTab, setActiveTab] = useState("reborn");

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

  const handleCopy = async () => {
    await navigator.clipboard.writeText(buildRebornMarkdown());
    setCopied(true);
    toast.success("Guide copied — load as IronClaw skill");
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <div className="flex min-h-[calc(100dvh-4rem)] flex-col overflow-auto">
      <div className="flex-1">
        <div className="mx-auto w-full max-w-4xl space-y-4 px-4 py-6 sm:px-6 sm:py-10 animate-fade-in">
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
          </div>

          <Tabs value={activeTab} onValueChange={setActiveTab} className="w-full">
            <TabsList className="w-full justify-start">
              <TabsTrigger value="v1">IronClaw v1</TabsTrigger>
              <TabsTrigger value="reborn">
                IronClaw Reborn
                <span className="ml-1.5 rounded-full bg-primary/15 px-1.5 py-0.5 text-[10px] font-semibold text-primary">
                  connects here
                </span>
              </TabsTrigger>
            </TabsList>

            <TabsContent value="v1" className="mt-4 space-y-4">
              <div className="rounded-xl border border-border bg-card p-6 space-y-4">
                <div className="flex items-start gap-3">
                  <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-secondary">
                    <Cloud size={14} className="text-muted-foreground" />
                  </div>
                  <div>
                    <p className="text-sm font-semibold text-foreground">
                      IronClaw v1 (agent.near.ai)
                    </p>
                    <p className="mt-1 text-sm text-muted-foreground">
                      The hosted service at{" "}
                      <a
                        href="https://agent.near.ai"
                        target="_blank"
                        rel="noopener noreferrer"
                        className="text-primary underline underline-offset-4"
                      >
                        agent.near.ai
                      </a>
                      . This is the legacy architecture —{" "}
                      <strong>it cannot connect to this dashboard</strong>. Use it directly in the
                      browser or via the CLI.
                    </p>
                  </div>
                </div>

                <div className="rounded-md border border-border bg-muted/50 px-3.5 py-2.5 text-xs text-muted-foreground">
                  This version does not support the WebUI v2 gateway. To chat through this
                  dashboard, you need the Reborn binary.
                </div>

                <div className="space-y-3">
                  <p className="text-sm font-medium text-foreground">NOVA submission via ironhub</p>
                  <p className="text-sm text-muted-foreground">
                    IronClaw v1 uses the{" "}
                    <code className="rounded bg-secondary px-1 py-0.5 font-mono text-xs">
                      ironhub
                    </code>{" "}
                    CLI command for NOVA submit instead of the built-in extension:
                  </p>
                  <CommandCopy command="ironhub nova submit --file ./submission.json" />
                  <p className="text-xs text-muted-foreground">
                    See the{" "}
                    <a
                      href="https://github.com/nearai/ironhub"
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-primary underline underline-offset-4"
                    >
                      ironhub repo
                    </a>{" "}
                    for full CLI reference.
                  </p>
                </div>

                <div className="flex flex-wrap items-center justify-between gap-3 pt-2 border-t border-border">
                  <div className="flex flex-wrap gap-2">
                    <Button variant="outline" size="sm" asChild>
                      <a href="https://agent.near.ai" target="_blank" rel="noopener noreferrer">
                        <ExternalLink size={12} />
                        agent.near.ai
                      </a>
                    </Button>
                    <Button variant="outline" size="sm" asChild>
                      <a
                        href="https://github.com/nearai/ironhub"
                        target="_blank"
                        rel="noopener noreferrer"
                      >
                        <ExternalLink size={12} />
                        ironhub repo
                      </a>
                    </Button>
                  </div>
                  <Button size="sm" onClick={() => setActiveTab("reborn")}>
                    <Zap size={12} />
                    Switch to Reborn
                  </Button>
                </div>
              </div>
            </TabsContent>

            <TabsContent value="reborn" className="mt-4 space-y-4">
              <div className="rounded-xl border border-border bg-card p-4 sm:p-6 space-y-4">
                <div className="flex items-center justify-between gap-4 flex-wrap">
                  <p className="text-sm font-semibold text-foreground">
                    {completedSteps.size === steps.length
                      ? "All steps complete!"
                      : `Step ${completedSteps.size + 1} of ${steps.length}`}
                  </p>
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
                  steps={steps}
                  completedSteps={completedSteps}
                  activeStep={activeStep}
                  onStepClick={toggleStep}
                />

                {completedSteps.size === steps.length && (
                  <div className="rounded-lg border border-[color:var(--near-green)]/30 bg-[color:var(--near-green)]/5 px-4 py-3 text-sm text-[color:var(--near-green)] font-medium">
                    You&apos;re all set! Head back to the chat to start using your agent.
                  </div>
                )}
              </div>

              <div className="space-y-3">
                {steps.map((step) => {
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
                          {isCompleted ? (
                            <Check size={14} />
                          ) : (
                            <Icon size={14} />
                          )}
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
                          {!isCompleted && (
                            <button
                              type="button"
                              onClick={(e) => {
                                e.stopPropagation();
                                markCompleteAndAdvance(step.id);
                              }}
                              className="hidden sm:flex items-center gap-1 rounded-md border border-[color:var(--near-green)]/40 bg-[color:var(--near-green)]/5 px-2 py-0.5 text-[10px] font-medium text-[color:var(--near-green)] hover:bg-[color:var(--near-green)]/10 transition-colors"
                            >
                              <Check size={9} />
                              Done
                            </button>
                          )}
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
                })}
              </div>
            </TabsContent>
          </Tabs>

          <div className="rounded-xl border border-border bg-card p-6">
            <p className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-3">
              Quick links
            </p>
            <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
              <UrlCard href="https://ironclaw.com" label="IronClaw" />
              <UrlCard href="https://github.com/nearai/ironclaw/blob/main/docs/reborn-binary.md" label="Reborn binary docs" />
              <UrlCard href="https://github.com/nearai/ironclaw/blob/main/docs/reborn-binary.md#extension" label="Nova-Submit extension" />
              <UrlCard href="https://github.com/jcarbonnell/ironclaw-hackathon" label="ironclaw-hackathon" />
              <UrlCard href="https://nova-sdk.com" label="NOVA SDK" />
              <UrlCard href="https://docs.ironclaw.com" label="IronClaw Docs" />
              <UrlCard href="https://cloud.near.ai" label="NEAR AI Cloud" />
              <UrlCard href="https://docs.near.ai/cloud/quickstart#setup" label="API key setup guide" />
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
