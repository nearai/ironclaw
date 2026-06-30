import { Link } from "react-router";
import { html } from "../lib/html.js";
import { SidebarFooter } from "./sidebar-footer.js";
import { SidebarNav } from "./sidebar-nav.js";
import { SidebarThreads } from "./sidebar-threads.js";
import { SidebarTraceCredits } from "./sidebar-trace-credits.js";

export function Sidebar({
  id,
  threadsState,
  theme,
  toggleTheme,
  profile,
  isAdmin,
  rebornProjectsEnabled = false,
  onSignOut,
  onClose,
  onNewChat,
  onSelectThread,
  onDeleteThread,
}) {
  return html`
    <aside
      id=${id}
      className="flex h-full w-[260px] shrink-0 flex-col border-r border-[var(--v2-panel-border)] bg-[var(--v2-surface)]"
    >
      <div className="flex items-center gap-2.5 px-4 py-5">
        <${Link}
          to="/chat"
          onClick=${onClose}
          className="flex items-center gap-2.5 opacity-90 hover:opacity-100"
          aria-label="IronClaw"
        >
          <img src="/v2/assets/logo.jpg" alt="IronClaw" className="h-7 w-auto" />
        <//>
      </div>

      <${SidebarNav}
        onNewChat=${onNewChat}
        isCreating=${threadsState.isCreating}
        isAdmin=${isAdmin}
        onNavigate=${onClose}
      />

      <${SidebarTraceCredits} />

      <div className="mt-3 flex min-h-0 flex-1 flex-col">
        <${SidebarThreads}
          threads=${threadsState.threads}
          activeThreadId=${threadsState.activeThreadId}
          rebornProjectsEnabled=${rebornProjectsEnabled}
          onSelect=${onSelectThread}
          onDelete=${onDeleteThread}
          onNavigate=${onClose}
        />
      </div>

      <${SidebarFooter}
        theme=${theme}
        toggleTheme=${toggleTheme}
        profile=${profile}
        onSignOut=${onSignOut}
      />
    </aside>
  `;
}
