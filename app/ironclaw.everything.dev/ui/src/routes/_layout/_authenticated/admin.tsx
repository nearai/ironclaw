import { createFileRoute, Outlet, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/_layout/_authenticated/admin")({
  beforeLoad: async ({ context }) => {
    if (!context.auth.isAdmin) {
      throw redirect({ to: "/home" });
    }
  },
  component: AdminLayout,
});

function AdminLayout() {
  return (
    <div className="flex h-full flex-col">
      <div className="flex shrink-0 items-center justify-between gap-3 border-b border-border bg-card px-4 py-2.5 sm:px-6 sm:py-3">
        <h1 className="text-xl font-semibold text-foreground">Admin</h1>
      </div>
      <div className="flex-1 overflow-y-auto px-4 py-6 sm:px-6">
        <div className="mx-auto max-w-3xl space-y-6">
          <Outlet />
        </div>
      </div>
    </div>
  );
}
