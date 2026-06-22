import { createFileRoute, Link } from "@tanstack/react-router";
import { ExternalLink, FileText, MessageCircle, Zap } from "lucide-react";
import ironclawLogo from "@/assets/ironclaw-attack.png";

export const Route = createFileRoute("/_layout/")({
  head: () => ({
    meta: [
      { title: "IronClaw Dashboard | NEAR Builders" },
      {
        name: "description",
        content:
          "Deploy your IronClaw agent, connect via tunnel, or run locally. Share API keys and collaborate through this dashboard.",
      },
    ],
  }),
  component: LandingPage,
});

function LandingPage() {
  return (
    <div className="flex min-h-[calc(100dvh-4rem)] flex-col overflow-auto">
      <div className="flex-1">
        <div className="mx-auto w-full max-w-4xl px-4 py-12 sm:px-6 sm:py-16 pb-20 sm:pb-12">
          <div className="text-center space-y-3">
            <div className="flex justify-center mb-2">
              <img
                src={ironclawLogo}
                alt="IronClaw"
                className="h-20 w-20 object-contain"
              />
            </div>

            <h1 className="text-4xl sm:text-5xl font-bold tracking-tight text-foreground">
              IronClaw Agent Dashboard
            </h1>

            <p className="text-base sm:text-lg text-muted-foreground font-medium max-w-xl mx-auto leading-relaxed">
              Connect your agent three ways — deploy and share, tunnel for staging, or run locally in
              dev. Each flow works with the same core dashboard.
            </p>

            <div className="flex items-center justify-center gap-3 pt-2">
              <Link
                to="/setup"
                className="inline-flex items-center gap-2 rounded-full border border-primary/40 bg-primary/5 px-5 py-2.5 text-sm font-medium text-primary hover:bg-primary/10 transition-colors"
              >
                <Zap size={14} />
                Setup
              </Link>
              <Link
                to="/skill"
                className="inline-flex items-center gap-2 rounded-full border border-primary/40 bg-primary/5 px-5 py-2.5 text-sm font-medium text-primary hover:bg-primary/10 transition-colors"
              >
                <FileText size={14} />
                Skill
              </Link>
            </div>

            <p className="text-sm text-muted-foreground/60">
              (and you can{" "}
              <a
                href="https://dev.everything.dev/about"
                target="_blank"
                rel="noopener noreferrer"
                className="underline underline-offset-2 hover:text-muted-foreground transition-colors"
              >
                customize and deploy your own UI
              </a>{" "}
              too 😉)
            </p>
          </div>

          <div className="mt-12 flex justify-center">
            <a
              href="https://nearbuilders.org"
              target="_blank"
              rel="noopener noreferrer"
              className="group flex items-center gap-4 rounded-xl border-2 border-[color:var(--near-green)]/40 bg-[color:var(--near-green)]/5 px-6 py-5 hover:border-[color:var(--near-green)] hover:bg-[color:var(--near-green)]/10 transition-all duration-200 max-w-md w-full"
            >
              <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-lg bg-[color:var(--near-green)]/10">
                <MessageCircle size={22} className="text-[color:var(--near-green)]" />
              </div>
              <div className="flex-1 min-w-0">
                <p className="text-base font-semibold text-foreground group-hover:text-[color:var(--near-green)] transition-colors">
                  Join NearBuilders
                </p>
                <p className="text-sm text-muted-foreground mt-0.5">
                  Connect with the community building on NEAR
                  <br />
                  and get onboarded.
                </p>
              </div>
              <ExternalLink
                size={16}
                className="shrink-0 text-muted-foreground group-hover:text-[color:var(--near-green)] transition-colors"
              />
            </a>
          </div>
        </div>
      </div>
    </div>
  );
}
