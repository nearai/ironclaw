use ironclaw_threads::ContextMessage;
use ironclaw_turns::run_profile::PromptContextTokenBudget;

use crate::estimate_tokens_from_chars;

pub(crate) type SelectedPromptContextMessage = (ContextMessage, u64);

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PromptContextSelection {
    pub(crate) selected: Vec<SelectedPromptContextMessage>,
    pub(crate) dropped_messages: usize,
    pub(crate) dropped_tokens: u64,
}

pub(crate) fn select_prompt_context_messages(
    messages: Vec<ContextMessage>,
    budget: PromptContextTokenBudget,
) -> PromptContextSelection {
    let visible_tokens = budget.visible_transcript_tokens();
    let messages = messages
        .into_iter()
        .map(|message| {
            let tokens = estimate_tokens_from_chars(&message.content).as_u64();
            (message, tokens)
        })
        .collect::<Vec<_>>();
    if visible_tokens == 0 {
        return PromptContextSelection {
            dropped_messages: messages.len(),
            dropped_tokens: messages
                .iter()
                .map(|(_, tokens)| *tokens)
                .fold(0_u64, u64::saturating_add),
            selected: Vec::new(),
        };
    }
    let mut selected = Vec::new();
    let mut selected_tokens = 0_u64;
    let mut dropped_messages = 0_usize;
    let mut dropped_tokens = 0_u64;

    for (index, (message, message_tokens)) in messages.iter().enumerate().rev() {
        let fits = selected_tokens.saturating_add(*message_tokens) <= visible_tokens;
        if fits {
            selected_tokens = selected_tokens.saturating_add(*message_tokens);
            selected.push((message.clone(), *message_tokens));
        } else {
            dropped_messages = index.saturating_add(1);
            dropped_tokens = messages[..=index]
                .iter()
                .map(|(_, tokens)| *tokens)
                .fold(0_u64, u64::saturating_add);
            break;
        }
    }

    selected.reverse();
    PromptContextSelection {
        selected,
        dropped_messages,
        dropped_tokens,
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_threads::{ContextMessage, MessageKind, ThreadMessageId};
    use ironclaw_turns::run_profile::PromptContextTokenBudget;

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

        let selection =
            select_prompt_context_messages(messages, PromptContextTokenBudget::new(2, 0, 0));

        assert_eq!(
            selection
                .selected
                .iter()
                .map(|(message, _)| message.sequence)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
    }

    #[test]
    fn selector_rejects_newest_message_when_it_exceeds_budget() {
        let messages = vec![message(1, "aaaa"), message(2, "this message is too large")];

        let selection =
            select_prompt_context_messages(messages, PromptContextTokenBudget::new(1, 0, 0));

        assert!(selection.selected.is_empty());
    }

    #[test]
    fn selector_returns_empty_for_empty_input() {
        let selection =
            select_prompt_context_messages(Vec::new(), PromptContextTokenBudget::new(1, 0, 0));

        assert!(selection.selected.is_empty());
    }

    #[test]
    fn selector_returns_empty_when_visible_budget_is_zero() {
        let selection = select_prompt_context_messages(
            vec![message(1, "a")],
            PromptContextTokenBudget::new(1, 1, 0),
        );

        assert!(selection.selected.is_empty());
    }

    #[test]
    fn selector_admits_message_at_exact_budget_boundary() {
        let messages = vec![message(1, "a"), message(2, "b")];

        let selection =
            select_prompt_context_messages(messages, PromptContextTokenBudget::new(2, 0, 0));

        assert_eq!(
            selection
                .selected
                .iter()
                .map(|(message, _)| message.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn selector_reports_dropped_messages_and_tokens_when_budget_exceeded() {
        let messages = vec![message(1, "a"), message(2, "b"), message(3, "c")];

        let selection =
            select_prompt_context_messages(messages, PromptContextTokenBudget::new(2, 0, 0));

        assert_eq!(selection.dropped_messages, 1);
        assert_eq!(selection.dropped_tokens, 1);
    }

    #[test]
    fn selector_reports_zero_dropped_messages_when_everything_fits() {
        let messages = vec![message(1, "a"), message(2, "b")];

        let selection =
            select_prompt_context_messages(messages, PromptContextTokenBudget::new(2, 0, 0));

        assert_eq!(selection.dropped_messages, 0);
        assert_eq!(selection.dropped_tokens, 0);
    }
}
