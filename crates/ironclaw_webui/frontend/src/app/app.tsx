import { BrowserRouter, Navigate, Route, Routes, useLocation, useNavigate } from "react-router";
import React from "react";
import { useT } from "../lib/i18n";
import { useAuthSession } from "./auth";
import { defaultRoute } from "./routes";
import { LoginPage as LoginView } from "../pages/login/login-page";
import { Button } from "../design-system/button";
import { RouteLoadBoundary } from "./route-load-boundary";

const GatewayLayout = React.lazy(() =>
  import("../layout/gateway-layout").then(({ GatewayLayout }) => ({ default: GatewayLayout }))
);
const ChatPage = React.lazy(() =>
  import("../pages/chat/chat-page").then(({ ChatPage }) => ({ default: ChatPage }))
);
const OnboardingPage = React.lazy(() =>
  import("../pages/onboarding/onboarding-page").then(({ OnboardingPage }) => ({
    default: OnboardingPage,
  }))
);
const WorkspacePage = React.lazy(() =>
  import("../pages/workspace/workspace-page").then(({ WorkspacePage }) => ({
    default: WorkspacePage,
  }))
);
const ProjectsPage = React.lazy(() =>
  import("../pages/projects/projects-page").then(({ ProjectsPage }) => ({
    default: ProjectsPage,
  }))
);
const MissionsPage = React.lazy(() =>
  import("../pages/missions/missions-page").then(({ MissionsPage }) => ({
    default: MissionsPage,
  }))
);
const JobsPage = React.lazy(() =>
  import("../pages/jobs/jobs-page").then(({ JobsPage }) => ({ default: JobsPage }))
);
const RoutinesPage = React.lazy(() =>
  import("../pages/routines/routines-page").then(({ RoutinesPage }) => ({
    default: RoutinesPage,
  }))
);
const AutomationsPage = React.lazy(() =>
  import("../pages/automations/automations-page").then(({ AutomationsPage }) => ({
    default: AutomationsPage,
  }))
);
const ExtensionsPage = React.lazy(() =>
  import("../pages/extensions/extensions-page").then(({ ExtensionsPage }) => ({
    default: ExtensionsPage,
  }))
);
const SettingsPage = React.lazy(() =>
  import("../pages/settings/settings-page").then(({ SettingsPage }) => ({
    default: SettingsPage,
  }))
);
const AdminPage = React.lazy(() =>
  import("../pages/admin/admin-page").then(({ AdminPage }) => ({ default: AdminPage }))
);
const LogsPage = React.lazy(() =>
  import("../pages/logs/logs-page").then(({ LogsPage }) => ({ default: LogsPage }))
);

function LazyRoute({ children }) {
  return (<RouteLoadBoundary>{children}</RouteLoadBoundary>);
}

function AuthLoading() {
  const t = useT();
  return (
    <main className="grid min-h-[100dvh] place-items-center bg-[var(--v2-canvas)] px-6">
      <div className="text-sm text-[var(--v2-text-muted)]">{t("app.checkingSession")}</div>
    </main>
  );
}

interface AuthSessionErrorProps {
  onRetry: () => void;
  onSignOut: () => void;
}

function AuthSessionError({ onRetry, onSignOut }: AuthSessionErrorProps) {
  const t = useT();
  return (
    <main className="grid min-h-[100dvh] place-items-center bg-[var(--v2-canvas)] px-6">
      <div
        role="alert"
        data-testid="session-check-error"
        className="w-full max-w-md rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] p-6 text-center shadow-sm"
      >
        <h1 className="text-lg font-semibold text-[var(--v2-text-strong)]">
          {t("app.sessionCheckFailedTitle")}
        </h1>
        <p className="mt-2 text-sm text-[var(--v2-text-muted)]">
          {t("app.sessionCheckFailedDescription")}
        </p>
        <div className="mt-5 flex flex-col justify-center gap-2 sm:flex-row">
          <Button
            type="button"
            data-testid="session-check-retry"
            onClick={onRetry}
          >
            {t("app.retrySession")}
          </Button>
          <Button
            type="button"
            variant="secondary"
            data-testid="session-check-sign-out"
            onClick={onSignOut}
          >
            {t("header.signOut")}
          </Button>
        </div>
      </div>
    </main>
  );
}

function LoginPage({ auth }) {
  const navigate = useNavigate();
  const location = useLocation();
  const fromLocation = location.state?.from;
  const from = fromLocation
    ? `${fromLocation.pathname || defaultRoute}${fromLocation.search || ""}${fromLocation.hash || ""}`
    : defaultRoute;
  const redirectAfter = from;

  const handleSubmit = React.useCallback(
    (token) => {
      auth.signIn(token);
      navigate(from, { replace: true });
    },
    [auth, from, navigate]
  );

  if (auth.isChecking) {
    return (<AuthLoading />);
  }

  if (auth.sessionCheckFailed) {
    return (
      <AuthSessionError
        onRetry={auth.retrySessionCheck}
        onSignOut={auth.signOut}
      />
    );
  }

  if (auth.isAuthenticated) {
    return (<Navigate to={from} replace />);
  }

  return (<LoginView
    initialToken={auth.token}
    error={auth.error}
    oauthRedirectAfter={redirectAfter}
    onSubmit={handleSubmit}
  />);
}

function RequireAuth({ auth, children }) {
  const location = useLocation();

  if (auth.isChecking) {
    return (<AuthLoading />);
  }

  if (auth.sessionCheckFailed) {
    return (
      <AuthSessionError
        onRetry={auth.retrySessionCheck}
        onSignOut={auth.signOut}
      />
    );
  }

  if (!auth.isAuthenticated) {
    return (<Navigate to="/login" replace state={{ from: location }} />);
  }

  return children;
}

function AuthenticatedLayout({ auth }) {
  return (
    <RequireAuth auth={auth}>
      <LazyRoute>
        <GatewayLayout
          token={auth.token}
          profile={auth.profile}
          isChecking={auth.isChecking}
          isAdmin={auth.isAdmin}
          rebornProjectsEnabled={auth.rebornProjectsEnabled}
          globalAutoApproveEnabled={auth.globalAutoApproveEnabled}
          onSignOut={auth.signOut}
        />
      </LazyRoute>
    </RequireAuth>
  );
}

function AdminRoute({ auth }) {
  if (!auth.isAdmin) {
    return (<Navigate to={defaultRoute} replace />);
  }
  return (<AdminPage />);
}

export function App() {
  const auth = useAuthSession();

  return (
    <BrowserRouter>
      <Routes>
        <Route path="/login" element={(<LoginPage auth={auth} />)} />
        <Route path="/" element={(<AuthenticatedLayout auth={auth} />)}>
          <Route index element={(<Navigate to={defaultRoute} replace />)} />
          <Route path="overview" element={(<Navigate to={defaultRoute} replace />)} />
          <Route path="welcome" element={(<LazyRoute><OnboardingPage /></LazyRoute>)} />
          <Route path="chat" element={(<LazyRoute><ChatPage /></LazyRoute>)} />
          <Route path="chat/:threadId" element={(<LazyRoute><ChatPage /></LazyRoute>)} />
          <Route path="workspace" element={(<LazyRoute><WorkspacePage /></LazyRoute>)} />
          <Route path="workspace/*" element={(<LazyRoute><WorkspacePage /></LazyRoute>)} />
          <Route path="projects" element={(<LazyRoute><ProjectsPage /></LazyRoute>)} />
          <Route path="projects/:projectId" element={(<LazyRoute><ProjectsPage /></LazyRoute>)} />
          <Route
            path="projects/:projectId/missions/:missionId"
            element={(<LazyRoute><ProjectsPage /></LazyRoute>)}
          />
          <Route
            path="projects/:projectId/threads/:threadId"
            element={(<LazyRoute><ProjectsPage /></LazyRoute>)}
          />
          <Route path="missions" element={(<LazyRoute><MissionsPage /></LazyRoute>)} />
          <Route path="missions/:missionId" element={(<LazyRoute><MissionsPage /></LazyRoute>)} />
          <Route path="jobs" element={(<LazyRoute><JobsPage /></LazyRoute>)} />
          <Route path="jobs/:jobId" element={(<LazyRoute><JobsPage /></LazyRoute>)} />
          <Route path="routines" element={(<LazyRoute><RoutinesPage /></LazyRoute>)} />
          <Route path="routines/:routineId" element={(<LazyRoute><RoutinesPage /></LazyRoute>)} />
          <Route path="automations" element={(<LazyRoute><AutomationsPage /></LazyRoute>)} />
          <Route
            path="extensions"
            element={(<LazyRoute><ExtensionsPage isAdmin={auth.isAdmin} /></LazyRoute>)}
          />
          <Route
            path="extensions/:tab"
            element={(<LazyRoute><ExtensionsPage isAdmin={auth.isAdmin} /></LazyRoute>)}
          />
          <Route path="logs" element={(<LazyRoute><LogsPage /></LazyRoute>)} />
          <Route path="settings" element={(<LazyRoute><SettingsPage /></LazyRoute>)} />
          <Route path="settings/:tab" element={(<LazyRoute><SettingsPage /></LazyRoute>)} />
          <Route
            path="admin"
            element={(<LazyRoute><AdminRoute auth={auth} /></LazyRoute>)}
          />
          <Route
            path="admin/:tab"
            element={(<LazyRoute><AdminRoute auth={auth} /></LazyRoute>)}
          />
        </Route>
        <Route path="*" element={(<Navigate to={defaultRoute} replace />)} />
      </Routes>
    </BrowserRouter>
  );
}
