use ironclaw_threads::ContextMessage;
use ironclaw_turns::run_profile::PromptContextTokenBudget;

use crate::estimate_tokens_from_chars;

pub(crate) type SelectedPromptContextMessage = (ContextMessage, u64);

pub(crate) fn select_prompt_context_messages(
    messages: Vec<ContextMessage>,
    budget: PromptContextTokenBudget,
) -> Vec<SelectedPromptContextMessage> {
    let visible_tokens = budget.visible_transcript_tokens();
    if visible_tokens == 0 {
        return Vec::new();
    }
    let mut selected = Vec::new();
    let mut selected_tokens = 0_u64;

    for message in messages.into_iter().rev() {
        let message_tokens = estimate_tokens_from_chars(&message.content).as_u64();
        let fits = selected_tokens.saturating_add(message_tokens) <= visible_tokens;
        if fits {
            selected_tokens = selected_tokens.saturating_add(message_tokens);
            selected.push((message, message_tokens));
        } else {
            break;
        }
    }

    selected.reverse();
    selected
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
        }
    }

    #[test]
    fn selector_keeps_contiguous_newest_messages_within_budget() {
        let messages = vec![message(1, "a"), message(2, "b"), message(3, "c")];

        let selected =
            select_prompt_context_messages(messages, PromptContextTokenBudget::new(2, 0, 0));

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
            select_prompt_context_messages(messages, PromptContextTokenBudget::new(1, 0, 0));

        assert!(selected.is_empty());
    }

    #[test]
    fn selector_returns_empty_for_empty_input() {
        let selected =
            select_prompt_context_messages(Vec::new(), PromptContextTokenBudget::new(1, 0, 0));

        assert!(selected.is_empty());
    }

    #[test]
    fn selector_returns_empty_when_visible_budget_is_zero() {
        let selected = select_prompt_context_messages(
            vec![message(1, "a")],
            PromptContextTokenBudget::new(1, 1, 0),
        );

        assert!(selected.is_empty());
    }

    #[test]
    fn selector_admits_message_at_exact_budget_boundary() {
        let messages = vec![message(1, "a"), message(2, "b")];

        let selected =
            select_prompt_context_messages(messages, PromptContextTokenBudget::new(2, 0, 0));

        assert_eq!(
            selected
                .iter()
                .map(|(message, _)| message.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }
}
