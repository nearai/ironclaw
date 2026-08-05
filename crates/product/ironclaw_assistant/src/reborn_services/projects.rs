//! Project wire vocabulary for the WebUI v2 service.
//!
//! The DTOs are re-exported from `ironclaw_product_contracts::workspace_views`
//! as part of this crate's public product API (the ~120-DTO re-export set the
//! §11.2.4 import-path rule deliberately leaves ungoverned).
//!
//! The **port** is not here. `trait ProjectService` and `ProjectServiceError`
//! moved to `ironclaw_product_contracts::project_service` on 2026-08-05
//! (PROPOSAL §12.13 D-P) so their implementing adapter could follow the records
//! it gates into `ironclaw_identity::projects`, and the port-location rule
//! forbids a second import path for a contracts-owned trait — so callers name
//! `ironclaw_product_contracts::project_service::{ProjectService,
//! ProjectServiceError}` directly rather than reaching them through this crate.

pub use ironclaw_product_contracts::workspace_views::{
    ProjectCaller, RebornAddMemberRequest, RebornCreateProjectRequest, RebornDeleteProjectRequest,
    RebornGetProjectRequest, RebornListMembersRequest, RebornListMembersResponse,
    RebornListProjectsRequest, RebornListProjectsResponse, RebornProjectInfo,
    RebornProjectMemberInfo, RebornProjectMemberStatus, RebornProjectResponse, RebornProjectRole,
    RebornProjectState, RebornRemoveMemberRequest, RebornUpdateMemberRoleRequest,
    RebornUpdateProjectRequest,
};
