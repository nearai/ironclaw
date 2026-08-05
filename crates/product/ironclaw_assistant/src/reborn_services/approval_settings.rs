//! Approval-setting contracts used by the product-facing service.
//!
//! Keeping these imports behind a focused module prevents the route-facing
//! service root from becoming a composition point for the approval runtime.

pub(super) use ironclaw_approvals::{
    AUTO_APPROVE_DEFAULT_ENABLED, AutoApproveSettingKey, AutoApproveSettingStorePort,
    PersistentApprovalAction, PersistentApprovalPolicyError, PersistentApprovalPolicyInput,
    PersistentApprovalPolicyKey, PersistentApprovalPolicyStorePort, ToolPermissionOverride,
    ToolPermissionOverrideInput, ToolPermissionOverrideKey, ToolPermissionOverrideStorePort,
    ToolPermissionState, permission_mode_allows_persistent_approval,
};
