import { createFileRoute, Link, Outlet, useRouterState } from "@tanstack/react-router";
import { BookOpen, Bot, MessageSquare, Puzzle, Wrench } from "lucide-react";
import { getAccount, getActiveRuntime, getAppName, sessionQueryOptions } from "@/app";
import builtOn from "@/assets/built_on.png";
import builtOnRev from "@/assets/built_on_rev.png";
import { IronclawStatus } from "@/components/ironclaw-status";
import { ThemeToggle } from "@/components/theme-toggle";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { UserNav } from "@/components/user-nav";
import { useIronclawStatus } from "@/hooks/use-ironclaw-status";
import { pluginSidebarItems, type SidebarItem, type SidebarRole } from "@/lib/plugin-sidebar.gen";

function filterSidebarByRole(items: SidebarItem[], userRole: SidebarRole): SidebarItem[] {
  return items.filter((item) => {
    if (item.roleRequired === "anon") return true;
    if (item.roleRequired === "member" && userRole !== "anon") return true;
    if (item.roleRequired === "admin" && userRole === "admin") return true;
    return false;
  });
}

function getUserRole(isAuthenticated: boolean, isAdmin: boolean): SidebarRole {
  if (isAdmin) return "admin";
  if (isAuthenticated) return "member";
  return "anon";
}

export const Route = createFileRoute("/_layout")({
  beforeLoad: async ({ context }) => {
    const { queryClient, authClient } = context;
    const session = await queryClient.ensureQueryData(
      sessionQueryOptions(authClient, context.session),
    );

    return {
      runtimeConfig: context.runtimeConfig,
      session,
    };
  },
  component: Layout,
});

function Layout() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const isNavigating = useRouterState({ select: (s) => s.status === "pending" });
  const { runtimeConfig, session } = Route.useRouteContext();
  const appName = getAppName(runtimeConfig);
  const runtime = getActiveRuntime(runtimeConfig);
  const account = getAccount(runtimeConfig);
  const isAuthenticated = !!session?.user;
  const userRole = getUserRole(isAuthenticated, session?.user?.role === "admin");
  const { status: connectionStatus } = useIronclawStatus();

  const ironclawSidebarItems: SidebarItem[] = [
    ...(connectionStatus === "connected" ? [] : [{ icon: Wrench, label: "setup", to: "/setup" as const, roleRequired: "member" as const }]),
    { icon: MessageSquare, label: "chat", to: "/" as const, roleRequired: "anon" as const },
    { icon: Bot, label: "automations", to: "/automations" as const, roleRequired: "anon" as const },
    { icon: Puzzle, label: "extensions", to: "/extensions" as const, roleRequired: "anon" as const },
    { icon: BookOpen, label: "skills", to: "/skills" as const, roleRequired: "anon" as const },
  ];
  const visibleItems = filterSidebarByRole([...pluginSidebarItems, ...ironclawSidebarItems], userRole);
  const gatewayId = runtime?.gatewayId;

  const isActive = (item: SidebarItem) => {
    return pathname === item.to || (item.to !== "/" && pathname.startsWith(`${item.to}/`));
  };

  return (
    <TooltipProvider>
      <div className="h-screen w-full flex overflow-hidden bg-background text-foreground">
        {isAuthenticated && (
          <aside className="hidden sm:flex h-full shrink-0 w-16 flex-col items-center border-r border-border bg-card animate-fade-in">
            <div className="flex-1 w-full overflow-y-auto flex flex-col items-center gap-1.5 py-4 min-h-0">
              <Tooltip>
                <TooltipTrigger asChild>
                  <Link
                    to="/"
                    preload="intent"
                    aria-label={`${appName} home`}
                    className="mb-3 flex items-center justify-center w-10 h-10"
                  >
                    <img
                      src="/logo.png"
                      alt={`${appName} logo`}
                      className="w-full h-full object-contain"
                    />
                  </Link>
                </TooltipTrigger>
                <TooltipContent side="right">{appName}</TooltipContent>
              </Tooltip>

              <div className="w-8 h-px bg-border/20 my-1" />

              {visibleItems.map((item) => {
                const Icon = item.icon;
                const active = isActive(item);
                  const isIronclaw = item.to === "/setup";
                const className = `relative flex items-center justify-center w-10 h-10 border-2 border-outset border-border-strong shadow-sm transition-all duration-200 ease-out hover:shadow-md ${active ? "bg-foreground text-background" : "bg-card text-foreground hover:bg-muted"}`;

                const statusDotColor =
                  connectionStatus === "connected"
                    ? "bg-[color:var(--near-green)]"
                    : connectionStatus === "checking"
                      ? "bg-muted-foreground"
                      : "bg-destructive";

                return (
                  <Tooltip key={item.label}>
                    <TooltipTrigger asChild>
                      <Link to={item.to} preload="intent" className={className}>
                        <Icon className="w-4 h-4" />
                        {isIronclaw && (
                          <span
                            className={`absolute top-0.5 right-0.5 h-2 w-2 rounded-full border-2 border-card ${statusDotColor}`}
                          />
                        )}
                      </Link>
                    </TooltipTrigger>
                    <TooltipContent side="right">{item.label}</TooltipContent>
                  </Tooltip>
                );
              })}
            </div>

            <div className="shrink-0 w-full flex justify-center py-3 bg-card border-t border-border z-10">
              <ThemeToggle />
            </div>
          </aside>
        )}

        <div className="flex-1 flex flex-col min-w-0 min-h-0 overflow-hidden">
          <div className="shrink-0 flex items-center justify-center py-1.5 px-3 bg-yellow-300 border-b border-yellow-400">
            <span className="text-[11px] font-bold tracking-wide text-yellow-950 text-center">
              Beta database will be wiped periodically. Do not save data you want to keep.
            </span>
          </div>

          <header
            className={`shrink-0 bg-card/50 ${isAuthenticated ? "border-b border-border animate-fade-in" : ""}`}
          >
            {isNavigating && (
              <div className="absolute top-0 left-0 right-0 h-[2px] z-50 overflow-hidden">
                <div
                  className="h-full bg-foreground animate-progress-bar"
                  style={{ width: "100%" }}
                />
              </div>
            )}

            <div className="flex items-center justify-between px-4 sm:px-6 h-12">
              {isAuthenticated ? (
                <div className="flex items-center gap-2 text-xs text-muted-foreground font-mono min-w-0">
                  <Link
                    aria-label={`${appName} home`}
                    className="sm:hidden flex items-center justify-center w-8 h-8"
                    to="/"
                    preload="intent"
                  >
                    <img
                      src="/logo.png"
                      alt={`${appName} logo`}
                      className="w-full h-full object-contain"
                    />
                  </Link>

                  <div className="hidden sm:flex items-center gap-2">
                    {gatewayId && (
                      <>
                        <span>{gatewayId}</span>
                        <span>/</span>
                      </>
                    )}
                    <span>{runtime?.accountId ?? account}</span>
                    <span>/</span>
                    <span className="truncate">
                      {pathname === "/" ? "chat" : pathname.slice(1).split("/").join(" / ")}
                    </span>
                  </div>
                </div>
              ) : (
                <Link
                  to="/login"
                  aria-label={`${appName} home`}
                  className="flex items-center justify-center w-10 h-10 transition-opacity duration-200 hover:opacity-70"
                >
                  <svg
                    viewBox="0 0 24 24"
                    fill="currentColor"
                    className="w-5 h-5 text-foreground"
                    aria-label={`${appName} logo`}
                  >
                    <title>{appName}</title>
                    <circle cx="12" cy="12" r="10" />
                  </svg>
                </Link>
              )}

              <div className="flex items-center gap-2">
                {isAuthenticated && <IronclawStatus />}
                <UserNav />
              </div>
            </div>
          </header>

          <main className="flex-1 w-full min-h-0 overflow-hidden flex flex-col">
            <Outlet />
          </main>

          {pathname !== "/" && (
            <footer className="shrink-0 flex justify-center py-6 pb-20 sm:pb-6">
              <a
                href="https://near.dev"
                target="_blank"
                rel="noopener noreferrer"
                className="relative h-6 w-[100px]"
              >
                <img
                  src={builtOn}
                  alt="Built on NEAR"
                  className="absolute inset-0 h-full w-full object-contain dark:hidden"
                />
                <img
                  src={builtOnRev}
                  alt="Built on NEAR"
                  className="absolute inset-0 hidden h-full w-full object-contain dark:block"
                />
              </a>
            </footer>
          )}

          {!isAuthenticated && (
            <div className="fixed bottom-4 left-4 z-40">
              <ThemeToggle />
            </div>
          )}

          {isAuthenticated && (
            <nav className="fixed bottom-0 left-0 right-0 sm:hidden border-t border-border bg-card animate-fade-in z-40">
              <div
                className="flex items-center justify-around px-2 pt-2"
                style={{ paddingBottom: "calc(0.5rem + env(safe-area-inset-bottom, 0px))" }}
              >
                {visibleItems.map((item) => {
                  const Icon = item.icon;
                  const active = isActive(item);
                const isIronclaw = item.to === "/setup";
                  const className = `flex flex-col items-center justify-center gap-0.5 p-1.5 transition-colors duration-200 ${active ? "text-foreground" : "text-muted-foreground"}`;

                  const statusDotColor =
                    connectionStatus === "connected"
                      ? "bg-[color:var(--near-green)]"
                      : connectionStatus === "checking"
                        ? "bg-muted-foreground"
                        : "bg-destructive";

                  return (
                    <Link key={item.label} to={item.to} preload="intent" className={className}>
                      <div className="relative">
                        <Icon className="w-4 h-4" />
                        {isIronclaw && (
                          <span
                            className={`absolute -top-0.5 -right-0.5 h-2 w-2 rounded-full border border-card ${statusDotColor}`}
                          />
                        )}
                      </div>
                      <span className="text-[10px]">{item.label}</span>
                    </Link>
                  );
                })}

                <div className="flex flex-col items-center justify-center p-1.5">
                  <ThemeToggle />
                </div>
              </div>
            </nav>
          )}
        </div>
      </div>
    </TooltipProvider>
  );
}
