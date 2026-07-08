use std::collections::{BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, Utc};
use ironclaw_filesystem::SeqNo;
use ironclaw_host_api::ResourceReservationId;

use crate::{
    ReservationRecord, ResourceAccount, ResourceError, ResourceLimits, ResourceState, ResourceTally,
};

const ACCOUNT_SHARDS: usize = 64;

pub(super) struct ResourceAuthority {
    shards: Vec<Mutex<AccountShard>>,
    commit_gates: Vec<Mutex<()>>,
    reservations: Mutex<HashMap<ResourceReservationId, ReservationRecord>>,
    latest_seq: Mutex<SeqNo>,
    poisoned: Mutex<Option<String>>,
}

#[derive(Default)]
struct AccountShard {
    limits: HashMap<ResourceAccount, ResourceLimits>,
    reserved_by_account: HashMap<ResourceAccount, ResourceTally>,
    usage_by_account: HashMap<ResourceAccount, ResourceTally>,
    period_anchors: HashMap<ResourceAccount, DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct AccountParts {
    limits: Option<ResourceLimits>,
    reserved: Option<ResourceTally>,
    usage: Option<ResourceTally>,
    period_anchor: Option<DateTime<Utc>>,
}

impl ResourceAuthority {
    pub(super) fn from_state(state: ResourceState, latest_seq: SeqNo) -> Self {
        let authority = Self {
            shards: (0..ACCOUNT_SHARDS)
                .map(|_| Mutex::new(AccountShard::default()))
                .collect(),
            commit_gates: (0..ACCOUNT_SHARDS).map(|_| Mutex::new(())).collect(),
            reservations: Mutex::new(state.reservations),
            latest_seq: Mutex::new(latest_seq),
            poisoned: Mutex::new(None),
        };
        for (account, limits) in state.limits {
            authority
                .shard_for_account(&account)
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .limits
                .insert(account, limits);
        }
        for (account, tally) in state.reserved_by_account {
            authority
                .shard_for_account(&account)
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .reserved_by_account
                .insert(account, tally);
        }
        for (account, tally) in state.usage_by_account {
            authority
                .shard_for_account(&account)
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .usage_by_account
                .insert(account, tally);
        }
        for (account, anchor) in state.period_anchors {
            authority
                .shard_for_account(&account)
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .period_anchors
                .insert(account, anchor);
        }
        authority
    }

    pub(super) fn check_available(&self) -> Result<(), ResourceError> {
        let poisoned = self.poisoned.lock().map_err(|_| ResourceError::Storage {
            reason: "resource governor poison lock poisoned".to_string(),
        })?;
        if let Some(reason) = poisoned.as_ref() {
            return Err(ResourceError::Storage {
                reason: reason.clone(),
            });
        }
        Ok(())
    }

    pub(super) fn poison(&self, error: ResourceError) {
        if let ResourceError::Storage { reason } = error
            && let Ok(mut poisoned) = self.poisoned.lock()
        {
            *poisoned = Some(reason);
        }
    }

    pub(super) fn set_latest_seq(&self, seq: SeqNo) -> Result<(), ResourceError> {
        *self.latest_seq.lock().map_err(|_| ResourceError::Storage {
            reason: "resource governor journal cursor lock poisoned".to_string(),
        })? = seq;
        Ok(())
    }

    pub(super) fn lock_commit_for_accounts(
        &self,
        accounts: &[ResourceAccount],
    ) -> Result<Vec<MutexGuard<'_, ()>>, ResourceError> {
        let mut indexes = BTreeSet::new();
        for account in accounts {
            indexes.insert(account_shard_index(account));
        }
        let mut guards = Vec::with_capacity(indexes.len());
        for index in indexes {
            guards.push(
                self.commit_gates[index]
                    .lock()
                    .map_err(|_| ResourceError::Storage {
                        reason: "resource governor commit gate lock poisoned".to_string(),
                    })?,
            );
        }
        Ok(guards)
    }

    pub(super) fn lock_reservations(
        &self,
    ) -> Result<MutexGuard<'_, HashMap<ResourceReservationId, ReservationRecord>>, ResourceError>
    {
        self.reservations
            .lock()
            .map_err(|_| ResourceError::Storage {
                reason: "resource governor reservation map lock poisoned".to_string(),
            })
    }

    pub(super) fn lock_accounts(
        &self,
        accounts: &[ResourceAccount],
    ) -> Result<LockedAccounts<'_>, ResourceError> {
        let mut indexes = BTreeSet::new();
        for account in accounts {
            indexes.insert(account_shard_index(account));
        }
        let mut guards = Vec::with_capacity(indexes.len());
        for index in indexes {
            let guard = self.shards[index]
                .lock()
                .map_err(|_| ResourceError::Storage {
                    reason: "resource governor account shard lock poisoned".to_string(),
                })?;
            guards.push((index, guard));
        }
        Ok(LockedAccounts { guards })
    }

    fn shard_for_account(&self, account: &ResourceAccount) -> &Mutex<AccountShard> {
        &self.shards[account_shard_index(account)]
    }
}

pub(super) struct LockedAccounts<'a> {
    guards: Vec<(usize, MutexGuard<'a, AccountShard>)>,
}

impl LockedAccounts<'_> {
    pub(super) fn state_for_accounts(
        &mut self,
        accounts: &[ResourceAccount],
        reservations: HashMap<ResourceReservationId, ReservationRecord>,
    ) -> ResourceState {
        let mut state = ResourceState {
            reservations,
            ..ResourceState::default()
        };
        for account in accounts {
            let shard = self.shard_mut(account);
            if let Some(limits) = shard.limits.get(account) {
                state.limits.insert(account.clone(), limits.clone());
            }
            if let Some(tally) = shard.reserved_by_account.get(account) {
                state
                    .reserved_by_account
                    .insert(account.clone(), tally.clone());
            }
            if let Some(tally) = shard.usage_by_account.get(account) {
                state
                    .usage_by_account
                    .insert(account.clone(), tally.clone());
            }
            if let Some(anchor) = shard.period_anchors.get(account) {
                state.period_anchors.insert(account.clone(), *anchor);
            }
        }
        state
    }

    pub(super) fn write_accounts_from_state(
        &mut self,
        accounts: &[ResourceAccount],
        state: &ResourceState,
    ) {
        for account in accounts {
            let shard = self.shard_mut(account);
            write_optional(
                &mut shard.limits,
                account,
                state.limits.get(account).cloned(),
            );
            write_optional(
                &mut shard.reserved_by_account,
                account,
                state.reserved_by_account.get(account).cloned(),
            );
            write_optional(
                &mut shard.usage_by_account,
                account,
                state.usage_by_account.get(account).cloned(),
            );
            write_optional(
                &mut shard.period_anchors,
                account,
                state.period_anchors.get(account).copied(),
            );
        }
    }

    pub(super) fn account_parts(&mut self, account: &ResourceAccount) -> AccountParts {
        let shard = self.shard_mut(account);
        AccountParts {
            limits: shard.limits.get(account).cloned(),
            reserved: shard.reserved_by_account.get(account).cloned(),
            usage: shard.usage_by_account.get(account).cloned(),
            period_anchor: shard.period_anchors.get(account).copied(),
        }
    }

    fn shard_mut(&mut self, account: &ResourceAccount) -> &mut AccountShard {
        let index = account_shard_index(account);
        self.guards
            .iter_mut()
            .find(|(candidate, _)| *candidate == index)
            .map(|(_, guard)| &mut **guard)
            // lock_accounts builds the guard list from exactly the account
            // shard indexes requested before LockedAccounts is constructed.
            .expect("account shard was locked") // safety: lock_accounts constructs guards for every requested account shard.
    }
}

fn write_optional<T: Clone>(
    map: &mut HashMap<ResourceAccount, T>,
    account: &ResourceAccount,
    value: Option<T>,
) {
    match value {
        Some(value) => {
            map.insert(account.clone(), value);
        }
        None => {
            map.remove(account);
        }
    }
}

fn account_shard_index(account: &ResourceAccount) -> usize {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    account.hash(&mut hasher);
    (hasher.finish() as usize) % ACCOUNT_SHARDS
}
