use std::sync::Arc;

use ironclaw_operator::llm_admin::llm_config_service::NearAiLoginStateStore;
use ironclaw_reborn_config::RebornBootConfig;

use crate::RebornBuildError;
use crate::webui::route_mounts::PublicRouteMount;

pub(crate) fn nearai_login_callback_mount(
    session: Arc<ironclaw_llm::SessionManager>,
    reload: Arc<dyn crate::LlmReloadTrigger>,
    boot: RebornBootConfig,
    states: Arc<NearAiLoginStateStore>,
) -> Result<PublicRouteMount, RebornBuildError> {
    let mount = ironclaw_operator::llm_admin::nearai_login_serve::nearai_login_callback_mount(
        session, reload, boot, states,
    )?;
    Ok(PublicRouteMount::new(mount.router, mount.descriptors))
}
