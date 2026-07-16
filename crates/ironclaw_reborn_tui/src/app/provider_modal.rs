//! Provider config modal: a three-level state machine (`Providers` ->
//! `Models` -> `Confirmed`), each forward transition emitting the matching
//! `Effect::Api`. `Esc` steps back one level; only `Esc` from the top level
//! closes the modal.

use crossterm::event::{KeyCode, KeyEvent};
use ironclaw_product_workflow::{LlmProbeResult, LlmProviderView};

use super::{ApiCall, AppState, Effect, Modal};

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderModalState {
    Providers {
        providers: Vec<LlmProviderView>,
        selected: usize,
        loading: bool,
    },
    Models {
        provider_id: String,
        adapter: String,
        base_url: Option<String>,
        models: Vec<String>,
        selected: usize,
        loading: bool,
    },
    Confirmed {
        provider_id: String,
        model: String,
        test_result: Option<LlmProbeResult>,
    },
}

impl Default for ProviderModalState {
    fn default() -> Self {
        Self::Providers {
            providers: Vec::new(),
            selected: 0,
            loading: false,
        }
    }
}

/// `Ctrl+L` from the composer: opens the modal and requests the provider
/// catalog.
pub(crate) fn open(state: &mut AppState) -> Vec<Effect> {
    state.modal = Some(Modal::Provider(ProviderModalState::Providers {
        providers: Vec::new(),
        selected: 0,
        loading: true,
    }));
    vec![Effect::Api(ApiCall::LlmProviders)]
}

pub(crate) fn dispatch_key(
    state: &mut AppState,
    key: KeyEvent,
    modal: ProviderModalState,
) -> Vec<Effect> {
    match key.code {
        KeyCode::Esc => dispatch_esc(state, modal),
        KeyCode::Up => {
            state.modal = Some(Modal::Provider(step_selection(modal, -1)));
            Vec::new()
        }
        KeyCode::Down => {
            state.modal = Some(Modal::Provider(step_selection(modal, 1)));
            Vec::new()
        }
        KeyCode::Enter => dispatch_enter(state, modal),
        _ => {
            state.modal = Some(Modal::Provider(modal));
            Vec::new()
        }
    }
}

fn step_selection(modal: ProviderModalState, delta: i32) -> ProviderModalState {
    match modal {
        ProviderModalState::Providers {
            providers,
            selected,
            loading,
        } => {
            let selected = step_index(selected, delta, providers.len());
            ProviderModalState::Providers {
                providers,
                selected,
                loading,
            }
        }
        ProviderModalState::Models {
            provider_id,
            adapter,
            base_url,
            models,
            selected,
            loading,
        } => {
            let selected = step_index(selected, delta, models.len());
            ProviderModalState::Models {
                provider_id,
                adapter,
                base_url,
                models,
                selected,
                loading,
            }
        }
        other @ ProviderModalState::Confirmed { .. } => other,
    }
}

fn step_index(selected: usize, delta: i32, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    if delta < 0 {
        selected.saturating_sub(1)
    } else {
        (selected + 1).min(len - 1)
    }
}

fn dispatch_esc(state: &mut AppState, modal: ProviderModalState) -> Vec<Effect> {
    match modal {
        ProviderModalState::Providers { .. } => {
            state.modal = None;
        }
        ProviderModalState::Models { .. } => {
            state.modal = Some(Modal::Provider(ProviderModalState::Providers {
                providers: Vec::new(),
                selected: 0,
                loading: false,
            }));
        }
        ProviderModalState::Confirmed { .. } => {
            // `Confirmed` doesn't carry `adapter`/`base_url` (its shape is
            // deliberately slimmer than `Models`'), so there is no data to
            // step back to a live Models view with; fall back to a fresh
            // Providers view rather than fabricate an adapter.
            state.modal = Some(Modal::Provider(ProviderModalState::Providers {
                providers: Vec::new(),
                selected: 0,
                loading: false,
            }));
        }
    }
    Vec::new()
}

fn dispatch_enter(state: &mut AppState, modal: ProviderModalState) -> Vec<Effect> {
    match modal {
        ProviderModalState::Providers {
            providers,
            selected,
            ..
        } => {
            let Some(provider) = providers.get(selected).cloned() else {
                state.modal = Some(Modal::Provider(ProviderModalState::Providers {
                    providers,
                    selected,
                    loading: false,
                }));
                return Vec::new();
            };
            let provider_id = provider.id;
            let adapter = provider.adapter;
            let base_url = provider.base_url;
            state.modal = Some(Modal::Provider(ProviderModalState::Models {
                provider_id: provider_id.clone(),
                adapter: adapter.clone(),
                base_url: base_url.clone(),
                models: Vec::new(),
                selected: 0,
                loading: true,
            }));
            vec![Effect::Api(ApiCall::LlmListModels {
                provider_id,
                adapter,
                base_url,
            })]
        }
        ProviderModalState::Models {
            provider_id,
            adapter,
            base_url,
            models,
            selected,
            ..
        } => {
            let Some(model) = models.get(selected).cloned() else {
                state.modal = Some(Modal::Provider(ProviderModalState::Models {
                    provider_id,
                    adapter,
                    base_url,
                    models,
                    selected,
                    loading: false,
                }));
                return Vec::new();
            };
            state.modal = Some(Modal::Provider(ProviderModalState::Confirmed {
                provider_id: provider_id.clone(),
                model: model.clone(),
                test_result: None,
            }));
            vec![
                Effect::Api(ApiCall::LlmSetActive {
                    provider_id: provider_id.clone(),
                    model,
                }),
                Effect::Api(ApiCall::LlmTestConnection {
                    provider_id,
                    adapter,
                    base_url,
                }),
            ]
        }
        confirmed @ ProviderModalState::Confirmed { .. } => {
            state.modal = Some(Modal::Provider(confirmed));
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{ctrl, key, models_modal_with, providers_modal_with};
    use super::super::{ApiCall, AppEvent, AppState, Effect, Modal, reduce};
    use super::*;

    #[test]
    fn ctrl_l_opens_provider_modal_and_requests_providers() {
        let mut state = AppState::default();
        let effects = reduce(&mut state, AppEvent::Key(ctrl('l')));
        assert!(matches!(state.modal, Some(Modal::Provider(_))));
        assert!(matches!(effects[0], Effect::Api(ApiCall::LlmProviders)));
    }

    #[test]
    fn enter_on_provider_lists_models() {
        let mut state = AppState::default().set_modal(Some(providers_modal_with(
            &[("openai", "open_ai_completions")],
            0,
        )));
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Enter)));
        assert!(matches!(
            &effects[0],
            Effect::Api(ApiCall::LlmListModels { provider_id, adapter, .. })
                if provider_id == "openai" && adapter == "open_ai_completions"
        ));
        assert!(matches!(
            &state.modal,
            Some(Modal::Provider(ProviderModalState::Models { provider_id, .. })) if provider_id == "openai"
        ));
    }

    #[test]
    fn enter_on_model_sets_active_and_triggers_test_connection() {
        let mut state = AppState::default().set_modal(Some(models_modal_with(
            "openai",
            "open_ai_completions",
            &["gpt-5"],
            0,
        )));
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Enter)));
        assert!(matches!(
            &effects[0],
            Effect::Api(ApiCall::LlmSetActive { provider_id, model })
                if provider_id == "openai" && model == "gpt-5"
        ));
        assert!(matches!(
            &effects[1],
            Effect::Api(ApiCall::LlmTestConnection { provider_id, .. }) if provider_id == "openai"
        ));
        assert!(matches!(
            &state.modal,
            Some(Modal::Provider(ProviderModalState::Confirmed { provider_id, model, .. }))
                if provider_id == "openai" && model == "gpt-5"
        ));
    }

    #[test]
    fn esc_steps_back_one_level_not_closes_modal() {
        let mut state = AppState::default().set_modal(Some(models_modal_with(
            "openai",
            "open_ai_completions",
            &["gpt-5"],
            0,
        )));
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Esc)));
        assert!(effects.is_empty());
        assert!(
            matches!(
                &state.modal,
                Some(Modal::Provider(ProviderModalState::Providers { .. }))
            ),
            "Esc from Models steps back to Providers, it does not close the modal"
        );
    }

    #[test]
    fn esc_from_providers_closes_the_modal() {
        let mut state = AppState::default().set_modal(Some(providers_modal_with(
            &[("openai", "open_ai_completions")],
            0,
        )));
        reduce(&mut state, AppEvent::Key(key(KeyCode::Esc)));
        assert!(state.modal.is_none());
    }
}
