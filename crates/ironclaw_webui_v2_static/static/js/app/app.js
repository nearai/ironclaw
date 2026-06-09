import { BrowserRouter, Navigate, Route, Routes, useLocation, useNavigate } from "react-router";
import { React, html } from "../lib/html.js";
import { useAuthSession } from "./auth.js";
import { defaultRoute } from "./routes.js";
import { GatewayLayout } from "../layout/gateway-layout.js";
import { LoginPage as LoginView } from "../pages/login/login-page.js";
import { ChatPage } from "../pages/chat/chat-page.js";
import { OnboardingPage } from "../pages/onboarding/onboarding-page.js";
import { WorkspacePage } from "../pages/workspace/workspace-page.js";
import { ProjectsPage } from "../pages/projects/projects-page.js";
import { MissionsPage } from "../pages/missions/missions-page.js";
import { JobsPage } from "../pages/jobs/jobs-page.js";
import { RoutinesPage } from "../pages/routines/routines-page.js";
import { AutomationsPage } from "../pages/automations/automations-page.js";
import { ExtensionsPage } from "../pages/extensions/extensions-page.js";
import { SettingsPage } from "../pages/settings/settings-page.js";
import { AdminPage } from "../pages/admin/admin-page.js";
import { LogsPage } from "../pages/logs/logs-page.js";

function AuthLoading() {
  return html`
    <main className="grid min-h-[100dvh] place-items-center bg-[var(--v2-canvas)] px-6">
      <div className="text-sm text-[var(--v2-text-muted)]">Checking session...</div>
    </main>
  `;
}

function LoginPage({ auth }) {
  const navigate = useNavigate();
  const location = useLocation();
  const fromLocation = location.state?.from;
  const from = fromLocation
    ? `${fromLocation.pathname || defaultRoute}${fromLocation.search || ""}${fromLocation.hash || ""}`
    : defaultRoute;
  const redirectAfter = `/v2${from === "/" ? "" : from}`;

  const handleSubmit = React.useCallback(
    (token) => {
      auth.signIn(token);
      navigate(from, { replace: true });
    },
    [auth, from, navigate]
  );

  if (auth.isChecking) {
    return html`<${AuthLoading} />`;
  }

  if (auth.isAuthenticated) {
    return html`<${Navigate} to=${from} replace />`;
  }

  return html`<${LoginView}
    initialToken=${auth.token}
    error=${auth.error}
    oauthRedirectAfter=${redirectAfter}
    onSubmit=${handleSubmit}
  />`;
}

function RequireAuth({ auth, children }) {
  const location = useLocation();

  if (auth.isChecking) {
    return html`<${AuthLoading} />`;
  }

  if (!auth.isAuthenticated) {
    return html`<${Navigate} to="/login" replace state=${{ from: location }} />`;
  }

  return children;
}

function AuthenticatedLayout({ auth }) {
  return html`
    <${RequireAuth} auth=${auth}>
      <${GatewayLayout}
        token=${auth.token}
        profile=${auth.profile}
        isChecking=${auth.isChecking}
        isAdmin=${auth.isAdmin}
        onSignOut=${auth.signOut}
      />
    <//>
  `;
}

function AdminRoute({ auth }) {
  if (!auth.isAdmin) {
    return html`<${Navigate} to=${defaultRoute} replace />`;
  }
  return html`<${AdminPage} />`;
}

export function App() {
  const auth = useAuthSession();

  return html`
    <${BrowserRouter} basename="/v2">
      <${Routes}>
        <${Route} path="/login" element=${html`<${LoginPage} auth=${auth} />`} />
        <${Route} path="/" element=${html`<${AuthenticatedLayout} auth=${auth} />`}>
          <${Route} index element=${html`<${Navigate} to=${defaultRoute} replace />`} />
          <${Route} path="overview" element=${html`<${Navigate} to=${defaultRoute} replace />`} />
          <${Route} path="welcome" element=${html`<${OnboardingPage} />`} />
          <${Route} path="chat" element=${html`<${ChatPage} />`} />
          <${Route} path="chat/:threadId" element=${html`<${ChatPage} />`} />
          <${Route} path="workspace" element=${html`<${WorkspacePage} />`} />
          <${Route} path="workspace/*" element=${html`<${WorkspacePage} />`} />
          <${Route} path="projects" element=${html`<${ProjectsPage} />`} />
          <${Route} path="projects/:projectId" element=${html`<${ProjectsPage} />`} />
          <${Route} path="projects/:projectId/missions/:missionId" element=${html`<${ProjectsPage} />`} />
          <${Route} path="projects/:projectId/threads/:threadId" element=${html`<${ProjectsPage} />`} />
          <${Route} path="missions" element=${html`<${MissionsPage} />`} />
          <${Route} path="missions/:missionId" element=${html`<${MissionsPage} />`} />
          <${Route} path="jobs" element=${html`<${JobsPage} />`} />
          <${Route} path="jobs/:jobId" element=${html`<${JobsPage} />`} />
          <${Route} path="routines" element=${html`<${RoutinesPage} />`} />
          <${Route} path="routines/:routineId" element=${html`<${RoutinesPage} />`} />
          <${Route} path="automations" element=${html`<${AutomationsPage} />`} />
          <${Route} path="extensions" element=${html`<${ExtensionsPage} />`} />
          <${Route} path="extensions/:tab" element=${html`<${ExtensionsPage} />`} />
          <${Route} path="logs" element=${html`<${LogsPage} />`} />
          <${Route} path="settings" element=${html`<${SettingsPage} />`} />
          <${Route} path="settings/:tab" element=${html`<${SettingsPage} />`} />
          <${Route} path="admin" element=${html`<${AdminRoute} auth=${auth} />`} />
          <${Route} path="admin/:tab" element=${html`<${AdminRoute} auth=${auth} />`} />
        <//>
        <${Route} path="*" element=${html`<${Navigate} to=${defaultRoute} replace />`} />
      <//>
    <//>
  `;
}
