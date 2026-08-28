use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, PromptContextTokenBudget,
};
use ironclaw_threads::{ContextMessage, ContextWindowTruncation, MessageKind, ThreadMessageId};

use crate::estimate_tokens_from_chars;

pub(crate) type SelectedPromptContextMessage = (ContextMessage, u64);

#[derive(Debug)]
pub(crate) struct PromptContextSelection {
    selected: Vec<SelectedPromptContextMessage>,
    pub(crate) truncation: Option<ContextWindowTruncation>,
}

impl PromptContextSelection {
    pub(crate) fn len(&self) -> usize {
        self.selected.len()
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    #[cfg(test)]
    fn iter(&self) -> std::slice::Iter<'_, SelectedPromptContextMessage> {
        self.selected.iter()
    }
}

impl IntoIterator for PromptContextSelection {
    type Item = SelectedPromptContextMessage;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.selected.into_iter()
    }
}

pub(crate) fn select_prompt_context_messages(
    mut messages: Vec<ContextMessage>,
    budget: PromptContextTokenBudget,
    pinned_message_id: Option<ThreadMessageId>,
) -> Result<PromptContextSelection, AgentLoopHostError> {
    let visible_tokens = budget.admitted_transcript_tokens();
    let pinned = pinned_message_id
        .and_then(|message_id| {
            messages.iter().position(|message| {
                message.message_id == Some(message_id) && message.kind == MessageKind::User
            })
        })
        .map(|index| messages.remove(index));
    let pinned = pinned.map(|message| {
        let tokens = estimate_tokens_from_chars(&message.content).as_u64();
        (message, tokens)
    });
    let pinned_tokens = pinned.as_ref().map(|(_, tokens)| *tokens).unwrap_or(0);
    if pinned_tokens > visible_tokens {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::BudgetExceeded,
            "accepted task exceeds the prompt context token budget",
        ));
    }

    let mut selected_tokens = pinned_tokens;
    let mut selected_start = messages.len();
    for (index, message) in messages.iter().enumerate().rev() {
        let message_tokens = estimate_tokens_from_chars(&message.content).as_u64();
        if selected_tokens.saturating_add(message_tokens) > visible_tokens {
            break;
        }
        selected_tokens = selected_tokens.saturating_add(message_tokens);
        selected_start = index;
    }

    let truncation = selected_start
        .checked_sub(1)
        .and_then(|index| messages.get(index))
        .map(|message| ContextWindowTruncation {
            omitted_through_sequence: message.sequence,
            omitted_through_kind: message.kind,
        });
    let mut selected = messages
        .into_iter()
        .skip(selected_start)
        .map(|message| {
            let tokens = estimate_tokens_from_chars(&message.content).as_u64();
            (message, tokens)
        })
        .collect::<Vec<_>>();
    if let Some(pinned) = pinned {
        selected.push(pinned);
    }
    selected.sort_by_key(|(message, _)| message.sequence);
    Ok(PromptContextSelection {
        selected,
        truncation,
    })
}

#[cfg(test)]
mod tests {
    use ironclaw_loop_contracts::PromptContextTokenBudget;
    use ironclaw_threads::{ContextMessage, ContextWindowTruncation, MessageKind, ThreadMessageId};

    use super::select_prompt_context_messages;
    fn message(sequence: u64, content: &str) -> ContextMessage {
        ContextMessage {
            message_id: Some(
                ThreadMessageId::parse(&format!("00000000-0000-0000-0000-{sequence:012}")).unwrap(),
            ),
            summary_id: None,
            sequence,
            kind: MessageKind::User,
            tool_result_provider_call: None,
            content: content.to_string(),
            image_attachments: Vec::new(),
        }
    }

    #[test]
    fn selector_keeps_contiguous_newest_messages_within_budget() {
        let messages = vec![message(1, "a"), message(2, "b"), message(3, "c")];

        let selected =
            select_prompt_context_messages(messages, PromptContextTokenBudget::new(2, 0, 0), None)
                .unwrap();

        assert_eq!(
            selected
                .iter()
                .map(|(message, _)| message.sequence)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
    }

    #[test]
    fn selector_rejects_newest_message_when_it_exceeds_budget() {
        let messages = vec![message(1, "aaaa"), message(2, "this message is too large")];

        let selected =
            select_prompt_context_messages(messages, PromptContextTokenBudget::new(1, 0, 0), None)
                .unwrap();

        assert_eq!(
            selected.truncation,
            Some(ContextWindowTruncation {
                omitted_through_sequence: 2,
                omitted_through_kind: MessageKind::User,
            })
        );
        assert!(selected.is_empty());
    }

    #[test]
    fn selector_returns_empty_for_empty_input() {
        let selected = select_prompt_context_messages(
            Vec::new(),
            PromptContextTokenBudget::new(1, 0, 0),
            None,
        )
        .unwrap();

        assert!(selected.is_empty());
    }

    #[test]
    fn selector_uses_reserve_as_compaction_trigger_not_admission_limit() {
        let selected = select_prompt_context_messages(
            vec![message(1, "a")],
            PromptContextTokenBudget::new(1, 1, 0),
            None,
        )
        .unwrap();

        assert_eq!(selected.len(), 1);
    }

    #[test]
    fn selector_admits_message_at_exact_budget_boundary() {
        let messages = vec![message(1, "a"), message(2, "b")];

        let selected =
            select_prompt_context_messages(messages, PromptContextTokenBudget::new(2, 0, 0), None)
                .unwrap();

        assert_eq!(
            selected
                .iter()
                .map(|(message, _)| message.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn selector_reserves_budget_for_the_accepted_task() {
        let pinned = message(1, "original task");
        let pinned_id = pinned.message_id;
        let messages = vec![pinned, message(2, "older result"), message(3, "newest")];

        let selected = select_prompt_context_messages(
            messages,
            PromptContextTokenBudget::new(6, 0, 0),
            pinned_id,
        )
        .expect("accepted task and newest suffix should fit");

        assert_eq!(
            selected
                .iter()
                .map(|(message, _)| message.sequence)
                .collect::<Vec<_>>(),
            vec![1, 3]
        );
    }

    #[test]
    fn selector_fails_when_the_accepted_task_exceeds_the_budget() {
        let pinned = message(1, "accepted task is larger than the available budget");
        let pinned_id = pinned.message_id;

        let error = select_prompt_context_messages(
            vec![pinned],
            PromptContextTokenBudget::new(1, 0, 0),
            pinned_id,
        )
        .expect_err("accepted task must never be silently dropped");

        assert_eq!(
            error.kind,
            ironclaw_loop_contracts::AgentLoopHostErrorKind::BudgetExceeded
        );
    }

    #[test]
    fn selector_does_not_pin_a_non_user_message() {
        let mut assistant = message(1, "assistant content larger than the budget");
        assistant.kind = ironclaw_threads::MessageKind::Assistant;
        let accepted_id = assistant.message_id;

        let selected = select_prompt_context_messages(
            vec![assistant],
            PromptContextTokenBudget::new(1, 0, 0),
            accepted_id,
        )
        .expect("a non-user accepted ref must not become a mandatory task");

        assert!(selected.is_empty());
    }
}
