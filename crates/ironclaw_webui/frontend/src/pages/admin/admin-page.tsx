import { Navigate, useNavigate, useParams } from "react-router";
import React from "react";
import { RouteLoadBoundary } from "../../app/route-load-boundary";

const UserDetail = React.lazy(() =>
  import("./components/user-detail").then(({ UserDetail }) => ({ default: UserDetail }))
);
const AdminUsersTab = React.lazy(() =>
  import("./components/users-tab").then(({ AdminUsersTab }) => ({ default: AdminUsersTab }))
);
const AdminConfigurationTab = React.lazy(() =>
  import("./components/configuration-tab").then(({ AdminConfigurationTab }) => ({
    default: AdminConfigurationTab,
  }))
);

export function AdminPage() {
  // Users and Configuration are the shipped admin tabs in this port;
  // dashboard/usage analytics stay out of the production bundle.
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
    users: selectedUserId
      ? (<UserDetail userId={selectedUserId} onBack={handleBack} />)
      : (<AdminUsersTab
          onSelectUser={handleSelectUser}
        />),
    configuration: (<AdminConfigurationTab />),
  };

  if (!tabContent[tab]) {
    return (<Navigate to="/admin/users" replace />);
  }

  return (
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="v2-page-entrance flex-1 p-4 sm:p-6">
        <div className="space-y-5">
          <RouteLoadBoundary>{tabContent[tab]}</RouteLoadBoundary>
        </div>
      </div>
    </div>
  );
}
