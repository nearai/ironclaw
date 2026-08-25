import { Navigate, useNavigate, useParams } from "react-router";
import React from "react";
import { RouteLoadBoundary } from "../../app/route-load-boundary";
import { PageScroll, PageStack } from "../../layout/page-shell";

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

export function AdminPage({ threadScrapingEnabled = false }) {
  // Users and Configuration are the shipped admin tabs in this port.
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
      ? (<UserDetail
          userId={selectedUserId}
          onBack={handleBack}
          threadScrapingEnabled={threadScrapingEnabled}
        />)
      : (<AdminUsersTab
          onSelectUser={handleSelectUser}
        />),
    configuration: (<AdminConfigurationTab />),
  };

  if (!tabContent[tab]) {
    return (<Navigate to="/admin/users" replace />);
  }

  return (
    <PageScroll>
      <PageStack>
        <RouteLoadBoundary>{tabContent[tab]}</RouteLoadBoundary>
      </PageStack>
    </PageScroll>
  );
}
