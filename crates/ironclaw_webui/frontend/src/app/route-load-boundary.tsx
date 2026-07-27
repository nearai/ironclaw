import React from "react";
import { useLocation } from "react-router";
import { Button } from "../design-system/button";
import { useT } from "../lib/i18n";

export function RouteLoading() {
  const t = useT();

  return (
    <div
      role="status"
      aria-live="polite"
      className="grid min-h-48 place-items-center px-6 py-12 text-sm text-[var(--v2-text-muted)]"
    >
      {t("app.loadingPage")}
    </div>
  );
}

interface RouteLoadErrorProps {
  onRetry: () => void;
}

export function RouteLoadError({ onRetry }: RouteLoadErrorProps) {
  const t = useT();

  return (
    <div className="grid min-h-48 place-items-center px-6 py-12">
      <div
        role="alert"
        className="w-full max-w-md rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] p-6 text-center shadow-sm"
      >
        <h2 className="text-lg font-semibold text-[var(--v2-text-strong)]">
          {t("app.pageLoadFailedTitle")}
        </h2>
        <p className="mt-2 text-sm text-[var(--v2-text-muted)]">
          {t("app.pageLoadFailedDescription")}
        </p>
        <Button type="button" className="mt-5" onClick={onRetry}>
          {t("app.reloadPage")}
        </Button>
      </div>
    </div>
  );
}

interface RouteErrorBoundaryProps {
  children: React.ReactNode;
  fallback: React.ReactNode;
}

interface RouteErrorBoundaryState {
  failed: boolean;
}

export class RouteErrorBoundary extends React.Component<
  RouteErrorBoundaryProps,
  RouteErrorBoundaryState
> {
  state: RouteErrorBoundaryState = { failed: false };

  static getDerivedStateFromError(): RouteErrorBoundaryState {
    return { failed: true };
  }

  componentDidCatch() {
    console.error("Failed to render a lazy-loaded route");
  }

  render() {
    return this.state.failed ? this.props.fallback : this.props.children;
  }
}

interface RouteLoadBoundaryProps {
  children: React.ReactNode;
}

export function RouteLoadBoundary({ children }: RouteLoadBoundaryProps) {
  const location = useLocation();

  return (
    <RouteErrorBoundary
      key={location.pathname}
      fallback={<RouteLoadError onRetry={() => window.location.reload()} />}
    >
      <React.Suspense fallback={<RouteLoading />}>{children}</React.Suspense>
    </RouteErrorBoundary>
  );
}
