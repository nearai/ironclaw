use std::collections::BTreeSet;

use ironclaw_host_api::ids::UserId;
use thiserror::Error;

/// A non-empty set of users admitted to an extension-scoped resource.
///
/// This value object is shared by registered-definition visibility and
/// installation ownership. It carries no lifecycle meaning by itself: the
/// containing record decides whether membership grants catalog discovery or
/// participation in an installed extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserMembership {
    user_ids: BTreeSet<UserId>,
}

/// A user membership with one or more users authorized to manage the roster.
///
/// Registered package definitions use this shape because sharing requires an
/// authority that ordinary installation membership deliberately does not
/// have. Manager and member sets are independent so a future tenant admin may
/// manage a private definition without automatically gaining catalog
/// visibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedUserMembership {
    managers: UserMembership,
    membership: UserMembership,
}

impl ManagedUserMembership {
    pub fn managed_by(manager: UserId) -> Self {
        Self {
            membership: UserMembership::user(manager.clone()),
            managers: UserMembership::user(manager),
        }
    }

    pub fn with_managers_and_members(
        manager_user_ids: BTreeSet<UserId>,
        member_user_ids: BTreeSet<UserId>,
    ) -> Result<Self, ManagedUserMembershipError> {
        let managers = UserMembership::users(manager_user_ids)
            .map_err(|_| ManagedUserMembershipError::EmptyManagers)?;
        let membership = UserMembership::users(member_user_ids)
            .map_err(|_| ManagedUserMembershipError::EmptyMembers)?;
        Ok(Self {
            managers,
            membership,
        })
    }

    pub fn managers(&self) -> &UserMembership {
        &self.managers
    }

    pub fn membership(&self) -> &UserMembership {
        &self.membership
    }

    pub fn can_manage(&self, user_id: &UserId) -> bool {
        self.managers.contains(user_id)
    }

    pub fn contains(&self, user_id: &UserId) -> bool {
        self.membership.contains(user_id)
    }
}

impl UserMembership {
    pub fn user(user_id: UserId) -> Self {
        Self {
            user_ids: BTreeSet::from([user_id]),
        }
    }

    pub fn users(user_ids: BTreeSet<UserId>) -> Result<Self, EmptyUserMembership> {
        if user_ids.is_empty() {
            return Err(EmptyUserMembership);
        }
        Ok(Self { user_ids })
    }

    pub fn user_ids(&self) -> &BTreeSet<UserId> {
        &self.user_ids
    }

    pub fn contains(&self, user_id: &UserId) -> bool {
        self.user_ids.contains(user_id)
    }

    /// Returns `None` when `user_id` is already a member.
    pub fn joined_by(&self, user_id: &UserId) -> Option<Self> {
        if self.contains(user_id) {
            return None;
        }
        let mut joined = self.user_ids.clone();
        joined.insert(user_id.clone());
        Some(Self { user_ids: joined })
    }

    /// Returns `None` when removing `user_id` leaves no members.
    pub fn without(&self, user_id: &UserId) -> Option<Self> {
        let mut remaining = self.user_ids.clone();
        remaining.remove(user_id);
        if remaining.is_empty() {
            None
        } else {
            Some(Self {
                user_ids: remaining,
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("extension membership must contain at least one user")]
pub struct EmptyUserMembership;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ManagedUserMembershipError {
    #[error("managed membership must contain at least one manager")]
    EmptyManagers,
    #[error("managed membership must contain at least one visible member")]
    EmptyMembers,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: &str) -> UserId {
        UserId::new(id).expect("valid user")
    }

    #[test]
    fn membership_is_non_empty_and_supports_join_and_leave() {
        assert_eq!(
            UserMembership::users(BTreeSet::new()),
            Err(EmptyUserMembership)
        );

        let alice = user("alice");
        let bob = user("bob");
        let membership = UserMembership::user(alice.clone());
        assert!(membership.contains(&alice));
        assert_eq!(membership.joined_by(&alice), None);

        let joined = membership.joined_by(&bob).expect("bob joins");
        assert!(joined.contains(&alice));
        assert!(joined.contains(&bob));
        assert_eq!(joined.without(&bob), Some(membership.clone()));
        assert_eq!(membership.without(&alice), None);
    }

    #[test]
    fn managed_membership_supports_multiple_non_visible_managers() {
        let alice = user("alice");
        let bob = user("bob");
        let admin = user("admin");
        let managed = ManagedUserMembership::managed_by(alice.clone());
        assert!(managed.can_manage(&alice));
        assert!(managed.contains(&alice));
        assert!(!managed.can_manage(&bob));

        let managed = ManagedUserMembership::with_managers_and_members(
            BTreeSet::from([alice.clone(), admin.clone()]),
            BTreeSet::from([alice, bob.clone()]),
        )
        .expect("independent manager and member sets");
        assert!(managed.can_manage(&admin));
        assert!(!managed.contains(&admin));

        assert_eq!(
            ManagedUserMembership::with_managers_and_members(
                BTreeSet::new(),
                BTreeSet::from([bob]),
            ),
            Err(ManagedUserMembershipError::EmptyManagers)
        );
        assert_eq!(
            ManagedUserMembership::with_managers_and_members(
                BTreeSet::from([admin]),
                BTreeSet::new(),
            ),
            Err(ManagedUserMembershipError::EmptyMembers)
        );
    }
}
