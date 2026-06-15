import { createFileRoute, Link, Outlet, useRouterState } from "@tanstack/react-router";
import { sessionQueryOptions } from "@/app";
import { Tabs, TabsList, TabsTrigger } from "@/components";

export const Route = createFileRoute("/_layout/_authenticated/settings")({
  head: () => ({
    meta: [
      { title: "Settings | auth.everything.dev" },
      { name: "description", content: "Manage your account identity and security." },
    ],
  }),
  loader: async ({ context }) => {
    await context.queryClient.ensureQueryData(
      sessionQueryOptions(context.authClient, context.session),
    );
  },
  component: SettingsLayout,
});

const tabs = [
  { value: "profile", to: "/settings/profile", label: "Profile" },
  { value: "auth-methods", to: "/settings/auth-methods", label: "Auth Methods" },
  { value: "api-keys", to: "/settings/api-keys", label: "API Keys" },
  { value: "security", to: "/settings/security", label: "Security" },
] as const;

function SettingsLayout() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });

  const activeTab =
    tabs.find((t) => pathname === t.to || pathname.startsWith(`${t.to}/`))?.value ?? "profile";

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <div className="flex shrink-0 items-center justify-between gap-3 border-b border-border bg-card px-4 py-2.5 sm:px-6 sm:py-3">
        <h1 className="text-xl font-semibold text-foreground">Settings</h1>
      </div>

      <div className="flex-1 overflow-y-auto px-4 py-6 sm:px-6">
        <div className="mx-auto max-w-3xl space-y-6">
          <Tabs value={activeTab} className="w-full min-w-0">
            <TabsList className="w-full justify-start overflow-x-auto">
              {tabs.map((tab) => (
                <TabsTrigger key={tab.value} value={tab.value} asChild className="shrink-0">
                  <Link to={tab.to}>{tab.label}</Link>
                </TabsTrigger>
              ))}
            </TabsList>
          </Tabs>

          <Outlet />
        </div>
      </div>
    </div>
  );
}
