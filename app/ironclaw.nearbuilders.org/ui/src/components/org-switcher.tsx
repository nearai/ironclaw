import { Link } from "@tanstack/react-router";
import { Building2, Check, Plus } from "lucide-react";
import type { Organization } from "@/app";
import { useAuthClient } from "@/app";
import { Button } from "./ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "./ui/dropdown-menu";

interface OrgSwitcherProps {
  organizations: Organization[];
  activeOrgId?: string | null;
  onSwitch?: (orgId: string) => void;
}

export function OrgSwitcher({ organizations, activeOrgId, onSwitch }: OrgSwitcherProps) {
  const auth = useAuthClient();
  const activeOrg = organizations.find((o) => o.id === activeOrgId);

  const handleSwitch = async (orgId: string) => {
    if (orgId === activeOrgId) return;
    const { error } = await auth.organization.setActive({ organizationId: orgId });
    if (!error) {
      onSwitch?.(orgId);
    }
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          className="hidden sm:flex items-center gap-2 text-xs text-muted-foreground max-w-[180px]"
        >
          <Building2 className="h-3.5 w-3.5 shrink-0" />
          <span className="truncate min-w-0">{activeOrg?.name ?? "workspace"}</span>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <DropdownMenuLabel className="text-xs text-muted-foreground">
          organizations
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        {organizations.map((org) => (
          <DropdownMenuItem
            key={org.id}
            className="flex items-center justify-between cursor-pointer"
            onClick={() => handleSwitch(org.id)}
          >
            <span className="truncate min-w-0 flex-1">{org.name}</span>
            {org.id === activeOrgId && <Check className="h-3.5 w-3.5 text-muted-foreground" />}
          </DropdownMenuItem>
        ))}
        {organizations.length === 0 && (
          <DropdownMenuItem disabled className="text-muted-foreground">
            no organizations
          </DropdownMenuItem>
        )}
        <DropdownMenuSeparator />
        <DropdownMenuItem asChild>
          <Link to="/organizations/new" className="flex items-center gap-2 cursor-pointer">
            <Plus className="h-3.5 w-3.5" />
            new organization
          </Link>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
