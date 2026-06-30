import { Navigate, useNavigate, useParams } from "react-router";
import { React, html } from "../../lib/html.js";
import { DashboardTab } from "./components/dashboard-tab.js";
import { UsageTab } from "./components/usage-tab.js";
import { UserDetail } from "./components/user-detail.js";
import { AdminUsersTab } from "./components/users-tab.js";

export function AdminPage() {
  const { tab = "dashboard" } = useParams();
  const navigate = useNavigate();
  const [selectedUserId, setSelectedUserId] = React.useState(null);

  const handleSelectUser = React.useCallback(
    (id) => {
      setSelectedUserId(id);
      navigate("/admin/users");
    },
    [navigate]
  );

  const handleBack = React.useCallback(() => {
    setSelectedUserId(null);
  }, []);

  const tabContent = {
    dashboard: html`<${DashboardTab}
      onSelectUser=${handleSelectUser}
      onNavigateTab=${(id) => navigate("/admin/" + id)}
    />`,
    users: selectedUserId
      ? html`<${UserDetail} userId=${selectedUserId} onBack=${handleBack} />`
      : html`<${AdminUsersTab}
          selectedUserId=${selectedUserId}
          onSelectUser=${handleSelectUser}
        />`,
    usage: html`<${UsageTab} onSelectUser=${handleSelectUser} />`,
  };

  if (!tabContent[tab]) {
    return html`<${Navigate} to="/admin/dashboard" replace />`;
  }

  return html`
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">${tabContent[tab]}</div>
      </div>
    </div>
  `;
}
