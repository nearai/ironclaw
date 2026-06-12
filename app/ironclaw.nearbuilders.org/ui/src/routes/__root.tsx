import { TanStackDevtools } from "@tanstack/react-devtools";
import {
  ClientOnly,
  createRootRouteWithContext,
  HeadContent,
  Link,
  Outlet,
  Scripts,
} from "@tanstack/react-router";
import { TanStackRouterDevtoolsPanel } from "@tanstack/react-router-devtools";
import { getRemoteScripts } from "everything-dev/ui/head";
import { getSocialImageMeta } from "everything-dev/ui/metadata";
import { ThemeProvider } from "next-themes";
import { Toaster } from "sonner";
import type { RouterContext } from "@/app";
import { getBaseStyles } from "@/app";
import { sessionQueryKey } from "@/lib/auth";
import TanStackQueryDevtools from "../integrations/tanstack-query/devtools";

export const Route = createRootRouteWithContext<RouterContext>()({
  beforeLoad: async ({ context }) => {
    const session = context.session;

    return {
      runtimeConfig: context.runtimeConfig,
      cspNonce: context.cspNonce,
      session,
    };
  },
  loader: async ({ context }) => {
    const { queryClient } = context;
    const session = context.session;

    if (session && queryClient) {
      queryClient.setQueryData(sessionQueryKey, session);
    }

    return {
      runtimeConfig: context.runtimeConfig,
      cspNonce: context.cspNonce,
      session,
    };
  },
  head: ({ loaderData }) => {
    const runtimeConfig = loaderData?.runtimeConfig;
    const cspNonce = loaderData?.cspNonce;
    const runtimeBasePath = runtimeConfig?.runtime?.runtimeBasePath ?? "/";
    const assetsUrl = runtimeConfig?.assetsUrl?.replace(/\/$/, "");
    const siteUrl = runtimeConfig?.hostUrl
      ? `${runtimeConfig.hostUrl}${runtimeBasePath === "/" ? "" : runtimeBasePath}`
      : "";
    const title = runtimeConfig?.runtime?.title ?? runtimeConfig?.account ?? "";
    const description = runtimeConfig?.runtime?.description ?? "";

    const structuredData = {
      "@context": "https://schema.org",
      "@type": "WebSite",
      name: title,
      description,
      url: runtimeConfig?.hostUrl || undefined,
    };

    return {
      meta: [
        { charSet: "utf-8" },
        {
          name: "viewport",
          content: "width=device-width, initial-scale=1.0, viewport-fit=cover",
        },
        { title },
        { name: "description", content: description },
        { name: "theme-color", content: "#ffffff" },
        { name: "color-scheme", content: "light dark" },
        { name: "application-name", content: title },
        { name: "mobile-web-app-capable", content: "yes" },
        {
          name: "apple-mobile-web-app-status-bar-style",
          content: "black-translucent",
        },
        { name: "format-detection", content: "telephone=no" },
        { name: "robots", content: "index, follow" },
        ...getSocialImageMeta({
          imageUrl: "/metadata.png",
          title,
          description,
          siteName: title,
          siteUrl,
          alt: description,
        }),
      ],
      links: [
        {
          rel: "stylesheet",
          href: `${assetsUrl ?? ""}/static/css/style.css${runtimeConfig?.ui?.integrity ? `?v=${encodeURIComponent(runtimeConfig.ui.integrity)}` : ""}`,
        },
        { rel: "preconnect", href: "https://api.fontshare.com" },
        { rel: "preconnect", href: "https://fonts.googleapis.com" },
        {
          rel: "preconnect",
          href: "https://fonts.gstatic.com",
          crossOrigin: "anonymous",
        },
        { rel: "shortcut icon", href: "/favicon.ico" },
        { rel: "icon", type: "image/svg+xml", href: "/icon.svg" },
        { rel: "icon", type: "image/png", sizes: "32x32", href: "/favicon-32x32.png" },
        { rel: "icon", type: "image/png", sizes: "16x16", href: "/favicon-16x16.png" },
        {
          rel: "apple-touch-icon",
          sizes: "180x180",
          href: "/apple-touch-icon.png",
        },
        { rel: "manifest", href: "/manifest.json" },
        ...(siteUrl ? [{ rel: "canonical", href: siteUrl }] : []),
      ],
      scripts: [
        ...getRemoteScripts({
          runtimeConfig: runtimeConfig ?? undefined,
          containerName: "ui",
          hydratePath: "./Hydrate",
          integrity: runtimeConfig?.ui?.integrity,
          cspNonce,
        }),
        {
          type: "application/ld+json",
          children: JSON.stringify(structuredData),
        },
      ],
    };
  },
  component: RootComponent,
  notFoundComponent: RootNotFound,
  errorComponent: RootError,
});

function RootComponent() {
  const { cspNonce } = Route.useRouteContext();
  return (
    <html lang="en" className="scroll-smooth" suppressHydrationWarning>
      <head>
        <HeadContent />
        <style nonce={cspNonce} dangerouslySetInnerHTML={{ __html: getBaseStyles() }} />
      </head>
      <body>
        <ThemeProvider attribute="class" defaultTheme="system" enableSystem nonce={cspNonce}>
          <div id="root">
            <Outlet />
          </div>
          <Toaster position="bottom-right" richColors closeButton />
        </ThemeProvider>
        <Scripts />
        {process.env.NODE_ENV === "development" && (
          <ClientOnly>
            <TanStackDevtools
              config={{ position: "bottom-right" }}
              plugins={[
                {
                  name: "Tanstack Router",
                  render: <TanStackRouterDevtoolsPanel />,
                },
                TanStackQueryDevtools,
              ]}
            />
          </ClientOnly>
        )}
      </body>
    </html>
  );
}

function RootNotFound() {
  return (
    <DocumentFallback title="Page not found" body="The page you requested doesn't exist here." />
  );
}

function RootError() {
  return (
    <DocumentFallback
      title="Application error"
      body="Something went wrong before the app layout could render."
    />
  );
}

function DocumentFallback({ title, body }: { title: string; body: string }) {
  return (
    <div className="min-h-dvh bg-background text-foreground flex items-center justify-center px-6">
      <div className="max-w-md text-center space-y-4">
        <h1 className="text-3xl font-semibold tracking-tight">{title}</h1>
        <p className="text-sm text-muted-foreground">{body}</p>
        <div>
          <Link
            to="/"
            className="inline-flex items-center justify-center h-10 px-4 border border-border bg-card hover:bg-accent transition-colors"
          >
            Back home
          </Link>
        </div>
      </div>
    </div>
  );
}
