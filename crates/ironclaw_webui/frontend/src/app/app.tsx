import { BrowserRouter, Navigate, Route, Routes, useLocation, useNavigate } from "react-router";
import React from "react";
import { useT } from "../lib/i18n";
import { useAuthSession } from "./auth";
import { defaultRoute } from "./routes";
import { GatewayLayout } from "../layout/gateway-layout";
import { LoginPage as LoginView } from "../pages/login/login-page";
import { ChatPage } from "../pages/chat/chat-page";
import { OnboardingPage } from "../pages/onboarding/onboarding-page";
import { WorkspacePage } from "../pages/workspace/workspace-page";
import { ProjectsPage } from "../pages/projects/projects-page";
import { MissionsPage } from "../pages/missions/missions-page";
import { JobsPage } from "../pages/jobs/jobs-page";
import { RoutinesPage } from "../pages/routines/routines-page";
import { AutomationsPage } from "../pages/automations/automations-page";
import { ExtensionsPage } from "../pages/extensions/extensions-page";
import { SettingsPage } from "../pages/settings/settings-page";
import { AdminPage } from "../pages/admin/admin-page";
import { LogsPage } from "../pages/logs/logs-page";

function AuthLoading() {
  const t = useT();
  return (
    <main className="grid min-h-[100dvh] place-items-center bg-[var(--v2-canvas)] px-6">
      <div className="text-sm text-[var(--v2-text-muted)]">{t("app.checkingSession")}</div>
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

  if (!auth.isAuthenticated) {
    return (<Navigate to="/login" replace state={{ from: location }} />);
  }

  return children;
}

function AuthenticatedLayout({ auth }) {
  return (
    <RequireAuth auth={auth}>
      <GatewayLayout
        token={auth.token}
        profile={auth.profile}
        isChecking={auth.isChecking}
        isAdmin={auth.isAdmin}
        rebornProjectsEnabled={auth.rebornProjectsEnabled}
        globalAutoApproveEnabled={auth.globalAutoApproveEnabled}
        onSignOut={auth.signOut}
      />
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
          <Route path="welcome" element={(<OnboardingPage />)} />
          <Route path="chat" element={(<ChatPage />)} />
          <Route path="chat/:threadId" element={(<ChatPage />)} />
          <Route path="workspace" element={(<WorkspacePage />)} />
          <Route path="workspace/*" element={(<WorkspacePage />)} />
          <Route path="projects" element={(<ProjectsPage />)} />
          <Route path="projects/:projectId" element={(<ProjectsPage />)} />
          <Route path="projects/:projectId/missions/:missionId" element={(<ProjectsPage />)} />
          <Route path="projects/:projectId/threads/:threadId" element={(<ProjectsPage />)} />
          <Route path="missions" element={(<MissionsPage />)} />
          <Route path="missions/:missionId" element={(<MissionsPage />)} />
          <Route path="jobs" element={(<JobsPage />)} />
          <Route path="jobs/:jobId" element={(<JobsPage />)} />
          <Route path="routines" element={(<RoutinesPage />)} />
          <Route path="routines/:routineId" element={(<RoutinesPage />)} />
          <Route path="automations" element={(<AutomationsPage />)} />
          <Route path="extensions" element={(<ExtensionsPage isAdmin={auth.isAdmin} />)} />
          <Route path="extensions/:tab" element={(<ExtensionsPage isAdmin={auth.isAdmin} />)} />
          <Route path="logs" element={(<LogsPage />)} />
          <Route path="settings" element={(<SettingsPage />)} />
          <Route path="settings/:tab" element={(<SettingsPage />)} />
          <Route path="admin" element={(<AdminRoute auth={auth} />)} />
          <Route path="admin/:tab" element={(<AdminRoute auth={auth} />)} />
        </Route>
        <Route path="*" element={(<Navigate to={defaultRoute} replace />)} />
      </Routes>
    </BrowserRouter>
  );
}
