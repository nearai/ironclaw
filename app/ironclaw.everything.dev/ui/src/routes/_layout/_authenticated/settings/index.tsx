import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/_layout/_authenticated/settings/")({
  loader: async () => {
    throw redirect({ to: "/settings/profile" });
  },
});
