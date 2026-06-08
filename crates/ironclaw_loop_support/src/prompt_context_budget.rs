use ironclaw_threads::ContextMessage;
use ironclaw_turns::run_profile::PromptContextTokenBudget;

use crate::estimate_tokens_from_chars;

pub(crate) fn select_prompt_context_messages(
    messages: Vec<ContextMessage>,
    budget: PromptContextTokenBudget,
) -> Vec<ContextMessage> {
    let visible_tokens = budget.visible_transcript_tokens();
    let mut selected = Vec::new();
    let mut selected_tokens = 0_u64;

    for message in messages.into_iter().rev() {
        let message_tokens = estimate_tokens_from_chars(&message.content).as_u64();
        let fits = selected_tokens.saturating_add(message_tokens) <= visible_tokens;
        if selected.is_empty() || fits {
            selected_tokens = selected_tokens.saturating_add(message_tokens);
            selected.push(message);
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
        let messages = vec![message(1, "aaaa"), message(2, "bbbb"), message(3, "cccc")];

        let selected =
            select_prompt_context_messages(messages, PromptContextTokenBudget::new(2, 0, 0));

        assert_eq!(
            selected
                .iter()
                .map(|message| message.sequence)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
    }

    #[test]
    fn selector_keeps_newest_message_when_it_exceeds_budget() {
        let messages = vec![message(1, "aaaa"), message(2, "this message is too large")];

        let selected =
            select_prompt_context_messages(messages, PromptContextTokenBudget::new(1, 0, 0));

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].sequence, 2);
    }
}
