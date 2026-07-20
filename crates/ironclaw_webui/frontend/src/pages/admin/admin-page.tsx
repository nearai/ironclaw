import { Navigate, useNavigate, useParams } from "react-router";
import React from "react";
import { DashboardTab } from "./components/dashboard-tab";
import { UsageTab } from "./components/usage-tab";
import { UserDetail } from "./components/user-detail";
import { AdminUsersTab } from "./components/users-tab";

export function AdminPage() {
  // Users is the only shipped admin tab in this port; dashboard/usage
  // (analytics) are out of scope, so both the default and the fallback land on
  // Users rather than the empty dashboard.
  const { tab = "users" } = useParams();
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
    dashboard: (<DashboardTab
      onSelectUser={handleSelectUser}
      onNavigateTab={(id) => navigate("/admin/" + id)}
    />),
    users: selectedUserId
      ? (<UserDetail userId={selectedUserId} onBack={handleBack} />)
      : (<AdminUsersTab
          selectedUserId={selectedUserId}
          onSelectUser={handleSelectUser}
        />),
    usage: (<UsageTab onSelectUser={handleSelectUser} />),
  };

  if (!tabContent[tab]) {
    return (<Navigate to="/admin/users" replace />);
  }

  return (
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">{tabContent[tab]}</div>
      </div>
    </div>
  );
}
