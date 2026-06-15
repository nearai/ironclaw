import { createFileRoute } from "@tanstack/react-router";
import {
  ChevronDown,
  Cloud,
  ExternalLink,
  MessageCircle,
  Terminal,
  Upload,
  UserPlus,
  Zap,
} from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { CommandCopy } from "@/components/ui/command-copy";

export const Route = createFileRoute("/_layout/hackathon/")({
  component: HackathonGuidePage,
});

const sections = [
  {
    id: "api-key",
    step: "0",
    icon: Cloud,
    title: "Get Your NEAR AI API Key",
    subtitle: "Create an account, claim free credits, and generate credentials",
    content: (
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
            <strong className="text-foreground">Claim $5 of free credits</strong>
          </li>
          <li>
            <strong className="text-foreground">Generate an API key</strong> in the "API Keys" section
          </li>
          <li>
            <strong className="text-foreground">Export your API key</strong>
          </li>
        </ol>

        <CommandCopy command='export NEARAI_API_KEY="your-key-here"' />

        <p className="text-xs text-muted-foreground">
          The run-reborn-webui.sh script in the next step will configure the model provider
          automatically (defaults to DeepSeek V4 Flash via NEAR AI).
        </p>

        <div className="rounded-md border border-border bg-muted/50 px-3.5 py-2.5 text-xs text-muted-foreground">
          Never share your API key publicly or commit it to version control.
        </div>
      </div>
    ),
  },
  {
    id: "setup",
    step: "1",
    icon: Terminal,
    title: "Set Up IronClaw (Reborn)",
    subtitle: "Build and run the reborn binary",
    content: (
      <div className="space-y-4">
        <div className="rounded-xl border-2 border-primary/40 bg-primary/5 px-5 py-4 space-y-3">
          <p className="text-sm font-bold text-foreground">Quick start (recommended)</p>
          <p className="text-sm text-muted-foreground">
            The repo includes run-reborn-webui.sh which handles the entire setup.
            Just export your provider key and run:
          </p>
          <CommandCopy command="git clone https://github.com/NEARBuilders/ironclaw.git && cd ironclaw" />
          <CommandCopy command='export NEARAI_API_KEY="your-key-here"' />
          <CommandCopy command="scripts/run-reborn-webui.sh" />
          <p className="text-sm text-muted-foreground">
            This opens ironclaw at http://127.0.0.1:3000. Copy the printed login token.
          </p>
        </div>

        <CommandCopy
          command='export IRONCLAW_REBORN_CORS_ORIGINS="http://localhost:3001"'
          label="Enable CORS for this site"
        />

        <p className="text-sm text-muted-foreground">
          Set this env var before starting ironclaw, or add{" "}
          <code className="rounded bg-secondary px-1 py-0.5">allowed_origins</code> to your config.toml.
          Then paste the printed login token below.
        </p>
      </div>
    ),
  },
  {
    id: "skills",
    step: "2",
    icon: Cloud,
    title: "Install the Hackathon Skill",
    subtitle: "Equip your agent with nova-submit and the hackathon skill",
    content: (
      <div className="space-y-4">
        <CommandCopy command="cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- extension install nova-submit" />
        <CommandCopy command="cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- extension activate nova-submit" />
        <CommandCopy command="git clone https://github.com/jcarbonnell/ironclaw-hackathon.git" />
        <CommandCopy command='export IRONCLAW_REBORN_HOME="$HOME/.ironclaw-reborn-demo"' />
        <CommandCopy command='mkdir -p "$IRONCLAW_REBORN_HOME/local-dev/tenants/default/users/reborn-cli/skills/ironclaw-hackathon"' />
        <CommandCopy command={`cp ironclaw-hackathon/skill/SKILL.md "$IRONCLAW_REBORN_HOME/local-dev/tenants/default/users/reborn-cli/skills/ironclaw-hackathon/"`} />
        <p className="text-sm text-muted-foreground">Verify everything is set up:</p>
        <CommandCopy command="cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- extension search nova && cargo run -q -p ironclaw_reborn_cli --bin ironclaw-reborn -- skills list | grep hackathon" />
      </div>
    ),
  },
  {
    id: "register",
    step: "3",
    icon: UserPlus,
    title: "Register for the Hackathon",
    subtitle: "Record your intent to compete",
    content: (
      <div className="space-y-4">
        <p className="text-sm text-muted-foreground">
          Use the form below to register, or tell your agent: "Register me for the hackathon."
        </p>
        <div className="rounded-lg border border-border bg-muted px-3.5 py-3">
          <p className="text-sm font-semibold text-foreground mb-2">What you need to provide</p>
          <div className="overflow-x-auto">
            <table className="w-full text-sm border-collapse">
              <thead>
                <tr className="bg-secondary">
                  <th className="px-3 py-2 text-left text-xs font-semibold text-muted-foreground border-b border-border">Field</th>
                  <th className="px-3 py-2 text-left text-xs font-semibold text-muted-foreground border-b border-border">Purpose</th>
                </tr>
              </thead>
              <tbody>
                <tr className="border-b border-border">
                  <td className="px-3 py-2 font-mono text-xs text-foreground">Agent ID</td>
                  <td className="px-3 py-2 text-xs text-muted-foreground">Short handle, no spaces/slashes/quotes</td>
                </tr>
                <tr className="border-b border-border">
                  <td className="px-3 py-2 font-mono text-xs text-foreground">Participant Name</td>
                  <td className="px-3 py-2 text-xs text-muted-foreground">Name or @handle for the leaderboard</td>
                </tr>
                <tr>
                  <td className="px-3 py-2 font-mono text-xs text-foreground">NOVA Account ID</td>
                  <td className="px-3 py-2 text-xs text-muted-foreground">Must match at submission time</td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </div>
    ),
  },
  {
    id: "contribute",
    step: "4",
    icon: Upload,
    title: "Contribute Skills & Tools",
    subtitle: "Publish your extensions to IronHub",
    content: (
      <div className="space-y-4">
        <p className="text-sm text-muted-foreground">
          Before submitting, contribute your custom skills and tools to IronHub.
        </p>
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" size="sm" asChild>
            <a href="https://github.com/nearai/ironhub/issues/new?template=new-skill.yml" target="_blank" rel="noopener noreferrer">
              <ExternalLink size={12} />
              New skill template
            </a>
          </Button>
          <Button variant="outline" size="sm" asChild>
            <a href="https://github.com/nearai/ironhub/issues/new?template=new-tool.yml" target="_blank" rel="noopener noreferrer">
              <ExternalLink size={12} />
              New tool template
            </a>
          </Button>
          <Button variant="outline" size="sm" asChild>
            <a href="https://iliad.codes" target="_blank" rel="noopener noreferrer">
              <ExternalLink size={12} />
              Iliad
            </a>
          </Button>
        </div>
      </div>
    ),
  },
  {
    id: "submit",
    step: "5",
    icon: Zap,
    title: "Submit Your Final Entry",
    subtitle: "Encrypt and upload via NOVA",
    content: (
      <div className="space-y-4">
        <p className="text-sm text-muted-foreground">
          Tell your agent: "Submit my final entry." The <strong>submit_final_entry</strong> method
          validates inputs, builds a submission file, and uploads it via the nova-submit tool.
        </p>
        <div className="rounded-lg border border-border bg-muted px-3.5 py-3">
          <p className="text-sm font-semibold text-foreground mb-2">What you need to provide</p>
          <div className="overflow-x-auto">
            <table className="w-full text-sm border-collapse">
              <thead>
                <tr className="bg-secondary">
                  <th className="px-3 py-2 text-left text-xs font-semibold text-muted-foreground border-b border-border">Field</th>
                  <th className="px-3 py-2 text-left text-xs font-semibold text-muted-foreground border-b border-border">Notes</th>
                </tr>
              </thead>
              <tbody>
                <tr className="border-b border-border">
                  <td className="px-3 py-2 font-mono text-xs text-foreground">NOVA Account ID</td>
                  <td className="px-3 py-2 text-xs text-muted-foreground">Must match registration</td>
                </tr>
                <tr className="border-b border-border">
                  <td className="px-3 py-2 font-mono text-xs text-foreground">NOVA API Key</td>
                  <td className="px-3 py-2 text-xs text-muted-foreground">From nova-sdk.com. Never stored or echoed</td>
                </tr>
                <tr className="border-b border-border">
                  <td className="px-3 py-2 font-mono text-xs text-foreground">Project Title</td>
                  <td className="px-3 py-2 text-xs text-muted-foreground">Short name for your project</td>
                </tr>
                <tr className="border-b border-border">
                  <td className="px-3 py-2 font-mono text-xs text-foreground">Workflow Description</td>
                  <td className="px-3 py-2 text-xs text-muted-foreground">One sentence, 280 chars max</td>
                </tr>
                <tr className="border-b border-border">
                  <td className="px-3 py-2 font-mono text-xs text-foreground">Demo URL</td>
                  <td className="px-3 py-2 text-xs text-muted-foreground">5 min video, publicly viewable</td>
                </tr>
                <tr>
                  <td className="px-3 py-2 font-mono text-xs text-foreground">GitHub Repo</td>
                  <td className="px-3 py-2 text-xs text-muted-foreground">Public repo URL (optional)</td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </div>
    ),
  },
  {
    id: "support",
    step: "6",
    icon: MessageCircle,
    title: "Get Support",
    subtitle: "Join the community on Telegram",
    content: (
      <div className="space-y-4">
        <a
          href="https://t.me/ironclawAI"
          target="_blank"
          rel="noopener noreferrer"
          className="group flex items-center gap-4 rounded-xl border-2 border-primary/40 bg-primary/5 px-5 py-4 hover:border-primary hover:bg-primary/10 transition-all"
        >
          <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-lg bg-primary/10">
            <MessageCircle size={22} className="text-primary" />
          </div>
          <div className="flex-1 min-w-0">
            <p className="text-base font-semibold text-foreground group-hover:text-primary transition-colors">
              t.me/ironclawAI
            </p>
            <p className="text-sm text-muted-foreground mt-0.5">
              Ask questions, share progress, connect with outros participants
            </p>
          </div>
          <ExternalLink size={16} className="shrink-0 text-muted-foreground group-hover:text-primary transition-colors" />
        </a>
      </div>
    ),
  },
];

function HackathonGuidePage() {
  const [openSections, setOpenSections] = useState<Set<string>>(new Set(["api-key", "setup"]));

  const toggleSection = (id: string) => {
    setOpenSections((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  return (
    <div className="mx-auto w-full max-w-4xl space-y-3 px-4 py-6 sm:px-6 sm:py-10">
      <div className="rounded-xl border border-border bg-card p-6 space-y-3">
        <div className="flex items-center gap-3">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-foreground text-background">
            <Zap size={18} />
          </div>
          <div>
            <span className="text-base font-semibold text-foreground">Hackathon Guide</span>
            <p className="mt-0.5 text-sm text-muted-foreground">
              6 steps — API key, reborn setup, skill install, register, contribute, submit
            </p>
          </div>
        </div>
      </div>

      <div className="space-y-2">
        {sections.map((section) => {
          const isOpen = openSections.has(section.id);
          return (
            <div key={section.id} className="rounded-xl border border-border bg-card overflow-hidden">
              <button
                type="button"
                onClick={() => toggleSection(section.id)}
                className="flex w-full items-center gap-3 px-5 py-4 text-left cursor-pointer hover:bg-muted/50 transition-colors"
              >
                <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-secondary text-[11px] font-bold text-muted-foreground font-mono">
                  {section.step}
                </div>
                <div className="flex-1 min-w-0">
                  <span className="text-sm font-semibold text-foreground">{section.title}</span>
                  <p className="text-xs text-muted-foreground mt-0.5">{section.subtitle}</p>
                </div>
                <ChevronDown
                  size={16}
                  className={`shrink-0 text-muted-foreground transition-transform duration-200 ${
                    isOpen ? "rotate-0" : "-rotate-90"
                  }`}
                />
              </button>
              {isOpen && (
                <div className="border-t border-border px-5 py-4">{section.content}</div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
