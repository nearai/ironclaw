import { Mail, RefreshCw, Shield, Trash2, User, UserCog } from "lucide-react";
import type { ApiClient } from "@/app";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Card, CardContent } from "./ui/card";

type OrgMembersResult = Awaited<ReturnType<ApiClient["auth"]["listMembers"]>>;
type MemberItem = OrgMembersResult[number];

interface MemberCardProps {
  member: MemberItem;
  canManage: boolean;
  onRemove?: () => void;
  onUpdateRole?: (role: "owner" | "admin" | "member") => void;
  isRemoving?: boolean;
  isUpdatingRole?: boolean;
}

export function MemberCard({
  member,
  canManage,
  onRemove,
  onUpdateRole,
  isRemoving,
  isUpdatingRole,
}: MemberCardProps) {
  const user = member.user;

  return (
    <Card>
      <CardContent className="p-5 space-y-3">
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-center gap-3 min-w-0">
            {user?.image ? (
              <img
                src={user.image}
                alt=""
                className="w-9 h-9 rounded-full object-cover border-2 border-outset border-[rgb(51,51,51)] dark:border-[rgb(100,100,100)]"
              />
            ) : (
              <div className="w-9 h-9 rounded-full bg-muted flex items-center justify-center text-xs font-medium">
                <User className="h-4 w-4 text-muted-foreground" />
              </div>
            )}
            <div className="min-w-0 space-y-0.5">
              <div className="font-medium text-sm truncate">
                {user?.name || user?.email || member.userId}
              </div>
              {user?.email && user.name && (
                <div className="text-xs text-muted-foreground truncate">{user.email}</div>
              )}
            </div>
          </div>
          <Badge variant="outline" className="shrink-0">
            {member.role}
          </Badge>
        </div>

        {canManage && onUpdateRole && (
          <div className="flex flex-wrap gap-2">
            {member.role !== "owner" && (
              <Button
                onClick={() => onUpdateRole("owner")}
                disabled={isUpdatingRole}
                variant="outline"
                size="sm"
              >
                <Shield className="h-3 w-3 mr-1" />
                make owner
              </Button>
            )}
            {member.role !== "admin" && (
              <Button
                onClick={() => onUpdateRole("admin")}
                disabled={isUpdatingRole}
                variant="outline"
                size="sm"
              >
                <UserCog className="h-3 w-3 mr-1" />
                make admin
              </Button>
            )}
            {member.role !== "member" && (
              <Button
                onClick={() => onUpdateRole("member")}
                disabled={isUpdatingRole}
                variant="outline"
                size="sm"
              >
                make member
              </Button>
            )}
          </div>
        )}

        {canManage && onRemove && (
          <Button
            onClick={onRemove}
            disabled={isRemoving}
            variant="outline"
            size="sm"
            className="text-destructive hover:text-destructive"
          >
            <Trash2 className="h-3 w-3 mr-1" />
            {isRemoving ? "removing..." : "remove"}
          </Button>
        )}
      </CardContent>
    </Card>
  );
}

type OrgInvitationsResult = Awaited<ReturnType<ApiClient["auth"]["listInvitations"]>>;
type InvitationItem = OrgInvitationsResult[number];

interface InvitationCardProps {
  invitation: InvitationItem;
  onResend?: () => void;
  onCancel?: () => void;
  isResending?: boolean;
  isCancelling?: boolean;
}

export function InvitationCard({
  invitation,
  onResend,
  onCancel,
  isResending,
  isCancelling,
}: InvitationCardProps) {
  return (
    <Card>
      <CardContent className="p-5 space-y-3">
        <div className="flex items-start justify-between gap-3">
          <div className="space-y-1 min-w-0">
            <div className="flex items-center gap-2">
              <Mail className="h-3.5 w-3.5 text-muted-foreground" />
              <div className="font-medium text-sm break-all">{invitation.email}</div>
            </div>
            <div className="text-xs text-muted-foreground font-mono">{invitation.role}</div>
          </div>
          <div className="flex gap-1 shrink-0">
            {onResend && (
              <Button onClick={onResend} disabled={isResending} variant="outline" size="sm">
                <RefreshCw className="h-3 w-3 mr-1" />
                resend
              </Button>
            )}
            {onCancel && (
              <Button
                onClick={onCancel}
                disabled={isCancelling}
                variant="outline"
                size="sm"
                className="text-destructive hover:text-destructive"
              >
                <Trash2 className="h-3 w-3 mr-1" />
                cancel
              </Button>
            )}
          </div>
        </div>
        <div className="text-xs text-muted-foreground">
          expires {new Date(invitation.expiresAt).toLocaleString()}
        </div>
      </CardContent>
    </Card>
  );
}
